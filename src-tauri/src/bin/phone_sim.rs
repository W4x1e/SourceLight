//! Stands in for the phone app until it exists (docs/MOBILE.md): pairs with a
//! running desktop from its pairing link, then connects and says hello the way
//! the phone will.
//!
//! ```text
//! cargo run --features dev-tools,phone-link --bin phone_sim -- pair "<link>"
//! cargo run --features dev-tools,phone-link --bin phone_sim -- hello [--away]
//! ```
//!
//! The link is the one Settings → Phone → Pair a phone shows (Copy link).
//! `--away` drops the desktop's direct addresses before dialling, so only the
//! relay can reach it: what a phone on another network sees.
//!
//! Its key and the desktop it paired with are kept in `phone_sim.json` in the
//! temp folder, so `hello` after `pair` is the same phone. Delete the file to
//! be a new phone.

use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sl_link::protocol::{PairingOffer, Platform};
use sl_link::{new_secret_key, PairedDesktop, Phone, Relay};

#[derive(Serialize, Deserialize)]
struct Saved {
    key: [u8; 32],
    desktop: Option<PairedDesktop>,
}

fn saved_path() -> PathBuf {
    std::env::temp_dir().join("phone_sim.json")
}

fn load() -> Saved {
    std::fs::read(saved_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| Saved {
            key: new_secret_key(),
            desktop: None,
        })
}

fn save(saved: &Saved) {
    std::fs::write(saved_path(), serde_json::to_vec_pretty(saved).unwrap())
        .expect("write phone_sim.json");
}

/// The relay the desktop uses: n0's public ones while ours do not exist.
fn relay_for(relay: &Option<String>) -> Relay {
    match relay {
        Some(_) => Relay::Public,
        None => Relay::Off,
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let away = args.iter().any(|a| a == "--away");
    let mut saved = load();
    match args.first().map(String::as_str) {
        Some("pair") => {
            let Some(link) = args.get(1) else {
                eprintln!("usage: phone_sim pair \"<link>\"");
                std::process::exit(2);
            };
            let offer = PairingOffer::from_uri(link).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            let mut offer = offer;
            if away {
                offer.addrs.clear();
            }
            let phone = Phone::start(saved.key, relay_for(&offer.relay))
                .await
                .expect("start the phone's endpoint");
            println!("phone {}", phone.id());
            println!("pairing with {} — answer on the desktop…", offer.name);
            match phone.pair(&offer, "phone_sim", Platform::Other).await {
                Ok(desktop) => {
                    println!("paired with {}", desktop.name);
                    saved.desktop = Some(desktop);
                    save(&saved);
                }
                Err(e) => {
                    eprintln!("pairing failed: {e}");
                    save(&saved);
                    std::process::exit(1);
                }
            }
            phone.shutdown().await;
        }
        Some("hello") => {
            let Some(mut desktop) = saved.desktop.clone() else {
                eprintln!("not paired yet: run `phone_sim pair \"<link>\"` first");
                std::process::exit(1);
            };
            if away {
                desktop.addrs.clear();
            }
            let phone = Phone::start(saved.key, relay_for(&desktop.relay))
                .await
                .expect("start the phone's endpoint");
            let started = Instant::now();
            let session = match phone.connect(&desktop).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("could not connect: {e}");
                    std::process::exit(1);
                }
            };
            match session.hello(env!("CARGO_PKG_VERSION"), "phone_sim").await {
                Ok(name) => println!(
                    "hello from {name} after {} ms",
                    started.elapsed().as_millis()
                ),
                Err(e) => {
                    eprintln!("hello refused: {e}");
                    std::process::exit(1);
                }
            }
            for _ in 0..3 {
                match session.ping().await {
                    Ok(rtt) => println!(
                        "ping {} ms over {}",
                        rtt.as_millis(),
                        if session.is_relayed() {
                            "the relay"
                        } else {
                            "a direct path"
                        }
                    ),
                    Err(e) => eprintln!("ping failed: {e}"),
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            session.close();
            phone.shutdown().await;
        }
        _ => {
            eprintln!("usage: phone_sim pair \"<link>\" [--away] | phone_sim hello [--away]");
            std::process::exit(2);
        }
    }
}
