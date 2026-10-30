//! The desktop's end: listens for its phones, and pairs new ones.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use sl_protocol::{PairingOffer, Platform, Refusal, Request, Response, ALPN, PROTOCOL_VERSION};

use crate::frame::{read_frame, write_frame};
use crate::{addr_strings, bind, LinkError, Relay, TestOptions};

/// How long a pairing code works for once shown.
pub const OFFER_TTL: Duration = Duration::from_secs(5 * 60);

/// How long the desktop waits for someone to approve a pairing.
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// The phones this desktop trusts. The desktop app keeps them in its
/// catalogue; tests keep them in memory.
///
/// Synchronous on purpose: each call is one small database read or write, and
/// the link checks [`PeerStore::is_paired`] on every request so a phone
/// removed in Settings is refused at once, not at its next connection.
pub trait PeerStore: Send + Sync + 'static {
    /// Whether this public key belongs to a paired phone.
    fn is_paired(&self, phone: &str) -> bool;
    /// Adds a phone the user approved. Pairing the same key again replaces it.
    fn add(&self, phone: &PairCandidate) -> Result<(), String>;
    /// Notes that a paired phone connected, for "last seen" in Settings.
    fn seen(&self, _phone: &str) {}
}

/// A phone asking to pair, as the desktop shows it for approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairCandidate {
    /// The phone's public key.
    pub id: String,
    pub device_name: String,
    pub platform: Platform,
}

/// Asks the person at the desktop whether to pair a phone. The desktop app
/// shows a dialog; tests answer at once.
pub trait Approver: Send + Sync + 'static {
    /// Resolves to `true` to pair. Not answering within [`APPROVAL_TIMEOUT`]
    /// counts as no.
    fn approve(&self, candidate: PairCandidate) -> Pin<Box<dyn Future<Output = bool> + Send>>;
}

/// Answers a paired phone's library requests (everything but pairing, hello
/// and ping, which the link answers itself). The desktop app reads its
/// catalogue; tests answer from memory. Only ever called for a phone that is
/// paired at that moment.
pub trait Service: Send + Sync + 'static {
    /// The response, and a body to send after it (a JPEG, or nothing).
    fn handle(
        &self,
        phone: String,
        request: Request,
        body: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = (Response, Vec<u8>)> + Send>>;
}

/// What the desktop needs to start listening.
pub struct DesktopConfig {
    /// This desktop's key, from [`crate::new_secret_key`] and then stored.
    pub secret_key: [u8; 32],
    pub relay: Relay,
    /// The computer's name, shown on the phone.
    pub name: String,
    pub peers: Arc<dyn PeerStore>,
    pub approver: Arc<dyn Approver>,
    pub service: Arc<dyn Service>,
    /// How long a pairing code works. [`OFFER_TTL`] unless a test shortens it.
    pub offer_ttl: Duration,
}

/// A running desktop end. Dropping it without [`Desktop::shutdown`] leaves
/// the endpoint to close in the background.
pub struct Desktop {
    router: Router,
    inner: Arc<Inner>,
}

struct Inner {
    name: Mutex<String>,
    peers: Arc<dyn PeerStore>,
    approver: Arc<dyn Approver>,
    service: Arc<dyn Service>,
    offer_ttl: Duration,
    relay_on: bool,
    /// The pairing code on screen, if any. One at a time: showing a new code
    /// retires the old one.
    offer: Mutex<Option<ActiveOffer>>,
}

struct ActiveOffer {
    token: [u8; 16],
    expires: Instant,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner").finish_non_exhaustive()
    }
}

impl Desktop {
    /// Starts listening.
    pub async fn start(config: DesktopConfig) -> Result<Self, LinkError> {
        Self::start_with(config, TestOptions::default()).await
    }

    pub(crate) async fn start_with(
        config: DesktopConfig,
        test: TestOptions,
    ) -> Result<Self, LinkError> {
        let endpoint = bind(
            &config.secret_key,
            &config.relay,
            vec![ALPN.to_vec()],
            &test,
        )
        .await?;
        let inner = Arc::new(Inner {
            name: Mutex::new(config.name),
            peers: config.peers,
            approver: config.approver,
            service: config.service,
            offer_ttl: config.offer_ttl,
            relay_on: config.relay != Relay::Off,
            offer: Mutex::new(None),
        });
        let router = Router::builder(endpoint)
            .accept(
                ALPN,
                Handler {
                    inner: inner.clone(),
                },
            )
            .spawn();
        Ok(Desktop { router, inner })
    }

    /// This desktop's public key.
    pub fn id(&self) -> String {
        self.router.endpoint().id().to_string()
    }

    /// Changes the name phones see, from the next connection.
    pub fn set_name(&self, name: String) {
        *self.inner.name.lock().unwrap() = name;
    }

    /// Makes a new pairing code, retiring any earlier one. With a relay on,
    /// waits briefly for it so the code works away from home too; without one
    /// (or if the relay is unreachable) the code carries direct addresses only.
    pub async fn offer(&self) -> PairingOffer {
        let endpoint = self.router.endpoint();
        if self.inner.relay_on && endpoint.addr().relay_urls().next().is_none() {
            let _ = tokio::time::timeout(Duration::from_secs(5), endpoint.online()).await;
        }
        let mut token = [0u8; 16];
        getrandom::fill(&mut token).expect("the OS random source works");
        *self.inner.offer.lock().unwrap() = Some(ActiveOffer {
            token,
            expires: Instant::now() + self.inner.offer_ttl,
        });
        let (addrs, relay) = addr_strings(&endpoint.addr());
        PairingOffer {
            desktop: self.id(),
            addrs,
            relay,
            token: URL_SAFE_NO_PAD.encode(token),
            name: self.inner.name.lock().unwrap().clone(),
        }
    }

    /// Retires the pairing code on screen, as when its dialog is closed.
    pub fn cancel_offer(&self) {
        *self.inner.offer.lock().unwrap() = None;
    }

    /// Stops listening and closes every connection.
    pub async fn shutdown(self) {
        let _ = self.router.shutdown().await;
    }
}

#[derive(Debug, Clone)]
struct Handler {
    inner: Arc<Inner>,
}

/// How long a refused phone has to read its refusal and hang up before the
/// desktop closes the connection on it. Closing at once can destroy the reply
/// still sitting unread in the phone's buffer.
const REFUSED_GRACE: Duration = Duration::from_secs(2);

impl ProtocolHandler for Handler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let phone = connection.remote_id().to_string();
        // Set by a request answered "not paired": that key gets no more.
        let cut_off = Arc::new(Notify::new());
        // Each request is its own stream, served on its own task: a pairing
        // waiting on the person at the desktop must not hold up a ping, and
        // later, thumbnails are fetched many at a time.
        loop {
            let (mut send, mut recv) = tokio::select! {
                stream = connection.accept_bi() => match stream {
                    Ok(stream) => stream,
                    // The phone closed the connection, or it dropped.
                    Err(_) => return Ok(()),
                },
                _ = cut_off.notified() => break,
            };
            let inner = self.inner.clone();
            let phone = phone.clone();
            let cut_off = cut_off.clone();
            tokio::spawn(async move {
                let (response, body) = match read_frame::<_, Request>(&mut recv).await {
                    Ok((request, body)) => inner.respond(&phone, request, body).await,
                    Err(_) => (
                        Response::Refused {
                            reason: Refusal::BadRequest,
                        },
                        Vec::new(),
                    ),
                };
                let unknown = matches!(
                    response,
                    Response::Refused {
                        reason: Refusal::NotPaired
                    }
                );
                if write_frame(&mut send, &response, &body).await.is_ok() {
                    let _ = send.finish();
                }
                if unknown {
                    cut_off.notify_one();
                }
            });
        }
        // A key that is not paired gets one answer, then nothing: no more
        // requests are read, and the connection closes once the phone hangs
        // up or the grace period ends.
        let _ = tokio::time::timeout(REFUSED_GRACE, connection.closed()).await;
        connection.close(0u32.into(), b"not paired");
        Ok(())
    }
}

impl Inner {
    async fn respond(&self, phone: &str, request: Request, body: Vec<u8>) -> (Response, Vec<u8>) {
        let refuse = |reason| Response::Refused { reason };
        let response = match request {
            // Allowed from any key: presenting the token is what earns trust.
            Request::Pair {
                token,
                device_name,
                platform,
            } => {
                if !self.take_offer(&token) {
                    return (refuse(Refusal::BadToken), Vec::new());
                }
                let candidate = PairCandidate {
                    id: phone.to_string(),
                    device_name,
                    platform,
                };
                let approved = tokio::time::timeout(
                    APPROVAL_TIMEOUT,
                    self.approver.approve(candidate.clone()),
                )
                .await
                .unwrap_or(false);
                if !approved {
                    return (refuse(Refusal::Declined), Vec::new());
                }
                if self.peers.add(&candidate).is_err() {
                    return (refuse(Refusal::BadRequest), Vec::new());
                }
                Response::Paired {
                    desktop_name: self.name.lock().unwrap().clone(),
                }
            }
            _ if !self.peers.is_paired(phone) => refuse(Refusal::NotPaired),
            Request::Hello { protocol, .. } => {
                if protocol < PROTOCOL_VERSION {
                    return (refuse(Refusal::UpdatePhone), Vec::new());
                }
                self.peers.seen(phone);
                Response::Welcome {
                    protocol: PROTOCOL_VERSION,
                    desktop_name: self.name.lock().unwrap().clone(),
                }
            }
            Request::Ping => Response::Pong,
            library => return self.service.handle(phone.to_string(), library, body).await,
        };
        (response, Vec::new())
    }

    /// Spends the pairing code if `token` matches it and it has not expired.
    /// A wrong token leaves the code in place for the right phone; guessing a
    /// 128-bit token is not a practical attack.
    fn take_offer(&self, token: &str) -> bool {
        let Ok(token) = URL_SAFE_NO_PAD.decode(token) else {
            return false;
        };
        let mut offer = self.offer.lock().unwrap();
        let Some(active) = offer.as_ref() else {
            return false;
        };
        if Instant::now() >= active.expires {
            *offer = None;
            return false;
        }
        if !constant_time_eq(&active.token, &token) {
            return false;
        }
        *offer = None;
        true
    }
}

/// Compares without stopping at the first difference, so the time taken says
/// nothing about how much of a guess was right.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn token_comparison() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }
}
