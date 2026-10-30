//! A desktop and a phone, both in this process, talking over real iroh
//! endpoints on loopback (and, for away-from-home, through a local relay).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sl_protocol::{ImageSize, PairingOffer, PhotoRow, Platform, Refusal, Request, Response};

use crate::*;

#[derive(Default)]
struct Memory {
    phones: Mutex<HashMap<String, PairCandidate>>,
    seen: Mutex<Vec<String>>,
}

impl Memory {
    fn remove(&self, phone: &str) {
        self.phones.lock().unwrap().remove(phone);
    }
}

impl PeerStore for Memory {
    fn is_paired(&self, phone: &str) -> bool {
        self.phones.lock().unwrap().contains_key(phone)
    }
    fn add(&self, phone: &PairCandidate) -> Result<(), String> {
        self.phones
            .lock()
            .unwrap()
            .insert(phone.id.clone(), phone.clone());
        Ok(())
    }
    fn seen(&self, phone: &str) {
        self.seen.lock().unwrap().push(phone.to_string());
    }
}

/// Answers every pairing request the same way, at once.
struct Answer(bool);

impl Approver for Answer {
    fn approve(&self, _: PairCandidate) -> Pin<Box<dyn Future<Output = bool> + Send>> {
        let yes = self.0;
        Box::pin(async move { yes })
    }
}

/// A one-photo library, counting what reaches it.
#[derive(Default)]
struct Library {
    calls: AtomicUsize,
}

impl Service for Library {
    fn handle(
        &self,
        _phone: String,
        request: Request,
        body: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = (Response, Vec<u8>)> + Send>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            match request {
                Request::Changes { since, .. } => (
                    Response::Changes {
                        catalogue: "cat".into(),
                        head: 1,
                        rows: if since < 1 {
                            vec![PhotoRow {
                                id: 1,
                                filename: "a.ARW".into(),
                                captured: None,
                                width: 6000,
                                height: 4000,
                                rating: 0,
                                flag: 0,
                                edit_tag: None,
                                edit_on: sl_protocol::EditOn::Raw,
                            }]
                        } else {
                            Vec::new()
                        },
                        removed: Vec::new(),
                        next: 1,
                        more: false,
                    },
                    Vec::new(),
                ),
                Request::Image { id: 1, .. } => (Response::Image, b"\xFF\xD8 jpeg".to_vec()),
                // Echoes a body back, to prove bodies travel both ways.
                Request::Verdicts { verdicts } => (
                    Response::Applied {
                        applied: verdicts.len() as u32,
                    },
                    body,
                ),
                _ => (
                    Response::Refused {
                        reason: Refusal::NotFound,
                    },
                    Vec::new(),
                ),
            }
        })
    }
}

struct Setup {
    relay: Relay,
    approve: bool,
    offer_ttl: Duration,
}

impl Default for Setup {
    fn default() -> Self {
        Setup {
            relay: Relay::Off,
            approve: true,
            offer_ttl: OFFER_TTL,
        }
    }
}

fn test_options(relay: &Relay) -> TestOptions {
    TestOptions {
        insecure_relay: matches!(relay, Relay::Custom(_)),
    }
}

async fn desktop(setup: Setup) -> (Desktop, Arc<Memory>) {
    let (desktop, peers, _) = desktop_with_library(setup).await;
    (desktop, peers)
}

async fn desktop_with_library(setup: Setup) -> (Desktop, Arc<Memory>, Arc<Library>) {
    let peers = Arc::new(Memory::default());
    let library = Arc::new(Library::default());
    let options = test_options(&setup.relay);
    let desktop = Desktop::start_with(
        DesktopConfig {
            secret_key: new_secret_key(),
            relay: setup.relay,
            name: "Studio PC".into(),
            peers: peers.clone(),
            approver: Arc::new(Answer(setup.approve)),
            service: library.clone(),
            offer_ttl: setup.offer_ttl,
        },
        options,
    )
    .await
    .unwrap();
    (desktop, peers, library)
}

async fn phone(relay: Relay) -> Phone {
    let options = test_options(&relay);
    Phone::start_with(new_secret_key(), relay, options)
        .await
        .unwrap()
}

/// What a phone would have if it skipped pairing: the desktop's address, but
/// no place on its list.
fn unpaired(offer: &PairingOffer) -> PairedDesktop {
    PairedDesktop {
        id: offer.desktop.clone(),
        name: offer.name.clone(),
        addrs: offer.addrs.clone(),
        relay: offer.relay.clone(),
    }
}

fn refusal(result: Result<impl std::fmt::Debug, LinkError>) -> Refusal {
    match result {
        Err(LinkError::Refused(r)) => r,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_phone_pairs_then_says_hello() {
    let (desktop, peers) = desktop(Setup::default()).await;
    let offer = desktop.offer().await;
    assert!(!offer.addrs.is_empty(), "the code carries direct addresses");
    assert_eq!(offer.relay, None);

    let phone = phone(Relay::Off).await;
    // Through the QR link, as the phone would get it.
    let scanned = PairingOffer::from_uri(&offer.to_uri()).unwrap();
    let paired = phone
        .pair(&scanned, "Test phone", Platform::Other)
        .await
        .unwrap();
    assert_eq!(paired.id, desktop.id());
    assert_eq!(paired.name, "Studio PC");
    assert!(peers.is_paired(&phone.id()));

    let session = phone.connect(&paired).await.unwrap();
    assert_eq!(
        session.hello("0.1.0", "Test phone").await.unwrap(),
        "Studio PC"
    );
    session.ping().await.unwrap();
    assert!(!session.is_relayed());
    assert_eq!(*peers.seen.lock().unwrap(), [phone.id()]);

    desktop.set_name("Renamed PC".into());
    assert_eq!(
        session.hello("0.1.0", "Test phone").await.unwrap(),
        "Renamed PC"
    );
    session.close();
    phone.shutdown().await;
    desktop.shutdown().await;
}

#[tokio::test]
async fn the_public_key_matches_the_secret() {
    let secret = new_secret_key();
    let phone = phone_with(secret).await;
    assert_eq!(public_key_of(&secret), phone.id());
    // The same key on the next start is the same phone.
    let again = phone_with(secret).await;
    assert_eq!(again.id(), phone.id());
}

async fn phone_with(secret: [u8; 32]) -> Phone {
    Phone::start_with(secret, Relay::Off, TestOptions::default())
        .await
        .unwrap()
}

#[tokio::test]
async fn a_pairing_code_works_once() {
    let (desktop, peers) = desktop(Setup::default()).await;
    let offer = desktop.offer().await;
    let first = phone(Relay::Off).await;
    first.pair(&offer, "First", Platform::Ios).await.unwrap();

    let second = phone(Relay::Off).await;
    let r = second.pair(&offer, "Second", Platform::Android).await;
    assert_eq!(refusal(r), Refusal::BadToken);
    assert!(!peers.is_paired(&second.id()));
}

#[tokio::test]
async fn an_expired_code_is_refused() {
    let (desktop, _) = desktop(Setup {
        offer_ttl: Duration::ZERO,
        ..Setup::default()
    })
    .await;
    let offer = desktop.offer().await;
    let r = phone(Relay::Off)
        .await
        .pair(&offer, "Late", Platform::Ios)
        .await;
    assert_eq!(refusal(r), Refusal::BadToken);
}

#[tokio::test]
async fn a_new_code_retires_the_old_one_and_cancel_retires_both() {
    let (desktop, _) = desktop(Setup::default()).await;
    let old = desktop.offer().await;
    let new = desktop.offer().await;
    let phone = phone(Relay::Off).await;
    let r = phone.pair(&old, "Phone", Platform::Ios).await;
    assert_eq!(refusal(r), Refusal::BadToken);

    desktop.cancel_offer();
    let r = phone.pair(&new, "Phone", Platform::Ios).await;
    assert_eq!(refusal(r), Refusal::BadToken);

    let fresh = desktop.offer().await;
    phone.pair(&fresh, "Phone", Platform::Ios).await.unwrap();
}

/// A wrong guess must not spend the real code.
#[tokio::test]
async fn a_wrong_token_leaves_the_code_for_the_right_phone() {
    let (desktop, _) = desktop(Setup::default()).await;
    let offer = desktop.offer().await;
    let guess = PairingOffer {
        token: "AAAAAAAAAAAAAAAAAAAAAA".into(),
        ..offer.clone()
    };
    let intruder = phone(Relay::Off).await;
    let r = intruder.pair(&guess, "Intruder", Platform::Other).await;
    assert_eq!(refusal(r), Refusal::BadToken);

    phone(Relay::Off)
        .await
        .pair(&offer, "Owner", Platform::Ios)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_declined_phone_stays_unknown() {
    let (desktop, peers) = desktop(Setup {
        approve: false,
        ..Setup::default()
    })
    .await;
    let offer = desktop.offer().await;
    let phone = phone(Relay::Off).await;
    let r = phone.pair(&offer, "Phone", Platform::Ios).await;
    assert_eq!(refusal(r), Refusal::Declined);
    assert!(!peers.is_paired(&phone.id()));

    let session = phone.connect(&unpaired(&offer)).await.unwrap();
    assert_eq!(
        refusal(session.hello("0.1.0", "Phone").await),
        Refusal::NotPaired
    );
}

#[tokio::test]
async fn an_unknown_phone_gets_one_refusal_then_is_cut_off() {
    let (desktop, _) = desktop(Setup::default()).await;
    let offer = desktop.offer().await;
    let phone = phone(Relay::Off).await;
    let session = phone.connect(&unpaired(&offer)).await.unwrap();
    assert_eq!(refusal(session.ping().await), Refusal::NotPaired);
    // The desktop closed the connection after answering.
    let again = tokio::time::timeout(Duration::from_secs(10), session.ping())
        .await
        .expect("a closed connection fails promptly");
    assert!(matches!(again, Err(LinkError::Network(_))), "{again:?}");
}

/// Removing a phone in Settings takes effect on its very next request, not
/// when it next reconnects.
#[tokio::test]
async fn a_removed_phone_is_refused_at_once() {
    let (desktop, peers) = desktop(Setup::default()).await;
    let offer = desktop.offer().await;
    let phone = phone(Relay::Off).await;
    let paired = phone.pair(&offer, "Phone", Platform::Ios).await.unwrap();
    let session = phone.connect(&paired).await.unwrap();
    session.hello("0.1.0", "Phone").await.unwrap();

    peers.remove(&phone.id());
    assert_eq!(refusal(session.ping().await), Refusal::NotPaired);
}

#[tokio::test]
async fn an_older_phone_is_told_to_update() {
    let (desktop, _) = desktop(Setup::default()).await;
    let offer = desktop.offer().await;
    let phone = phone(Relay::Off).await;
    let paired = phone.pair(&offer, "Phone", Platform::Ios).await.unwrap();
    let session = phone.connect(&paired).await.unwrap();
    let old_hello = Request::Hello {
        protocol: 0,
        app_version: "0.0.1".into(),
        device_name: "Phone".into(),
    };
    assert_eq!(
        session.request(&old_hello, &[]).await.unwrap().0,
        Response::Refused {
            reason: Refusal::UpdatePhone
        }
    );
}

/// Away from home: the phone has no address for the desktop that it can
/// reach directly, only the relay, and still pairs and talks.
#[tokio::test]
async fn pairing_and_hello_work_through_a_relay_alone() {
    let (_map, relay_url, _server) = iroh::test_utils::run_relay_server().await.unwrap();
    let relay = Relay::Custom(vec![relay_url.to_string()]);
    let (desktop, peers) = desktop(Setup {
        relay: relay.clone(),
        ..Setup::default()
    })
    .await;

    let mut offer = desktop.offer().await;
    assert_eq!(offer.relay.as_deref(), Some(relay_url.to_string().as_str()));
    offer.addrs.clear();

    let phone = phone(relay).await;
    let paired = phone.pair(&offer, "Phone", Platform::Ios).await.unwrap();
    assert!(peers.is_paired(&phone.id()));
    assert!(paired.addrs.is_empty());

    let session = phone.connect(&paired).await.unwrap();
    assert_eq!(session.hello("0.1.0", "Phone").await.unwrap(), "Studio PC");
    session.ping().await.unwrap();
}

#[tokio::test]
async fn a_damaged_key_is_an_address_error() {
    let phone = phone(Relay::Off).await;
    let desktop = PairedDesktop {
        id: "not a key".into(),
        name: "PC".into(),
        addrs: vec!["127.0.0.1:1".into()],
        relay: None,
    };
    let r = phone.connect(&desktop).await;
    assert!(matches!(r, Err(LinkError::BadAddress(_))), "{:?}", r.err());
}

/// Everything but pairing, hello and ping goes to the desktop's library, with
/// the request's body, and comes back with the response's.
#[tokio::test]
async fn library_requests_reach_the_service_with_their_bodies() {
    let (desktop, _, library) = desktop_with_library(Setup::default()).await;
    let offer = desktop.offer().await;
    let phone = phone(Relay::Off).await;
    let paired = phone.pair(&offer, "Phone", Platform::Ios).await.unwrap();
    let session = phone.connect(&paired).await.unwrap();
    session.hello("0.1.0", "Phone").await.unwrap();

    let (changes, _) = session
        .request(
            &Request::Changes {
                since: -1,
                limit: 500,
            },
            &[],
        )
        .await
        .unwrap();
    let Response::Changes { rows, .. } = changes else {
        panic!("{changes:?}");
    };
    assert_eq!(rows.len(), 1);

    let (image, jpeg) = session
        .request(
            &Request::Image {
                id: 1,
                size: ImageSize::Thumb,
            },
            &[],
        )
        .await
        .unwrap();
    assert_eq!(image, Response::Image);
    assert_eq!(jpeg, b"\xFF\xD8 jpeg");

    // Clones share the connection: several requests at once, as the grid makes.
    let fetches = (0..8).map(|_| {
        let s = session.clone();
        tokio::spawn(async move {
            s.request(
                &Request::Image {
                    id: 1,
                    size: ImageSize::Preview,
                },
                &[],
            )
            .await
            .unwrap()
            .1
        })
    });
    for f in fetches {
        assert_eq!(f.await.unwrap(), b"\xFF\xD8 jpeg");
    }

    let (applied, echoed) = session
        .request(
            &Request::Verdicts {
                verdicts: Vec::new(),
            },
            b"body",
        )
        .await
        .unwrap();
    assert_eq!(applied, Response::Applied { applied: 0 });
    assert_eq!(echoed, b"body");
    assert_eq!(library.calls.load(Ordering::SeqCst), 11);
}

/// An unpaired phone's library request is refused by the link; the library
/// never sees it.
#[tokio::test]
async fn an_unpaired_phone_never_reaches_the_library() {
    let (desktop, _, library) = desktop_with_library(Setup::default()).await;
    let offer = desktop.offer().await;
    let phone = phone(Relay::Off).await;
    let session = phone.connect(&unpaired(&offer)).await.unwrap();
    let (response, body) = session
        .request(
            &Request::Changes {
                since: -1,
                limit: 10,
            },
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        response,
        Response::Refused {
            reason: Refusal::NotPaired
        }
    );
    assert!(body.is_empty());
    assert_eq!(library.calls.load(Ordering::SeqCst), 0);
}
