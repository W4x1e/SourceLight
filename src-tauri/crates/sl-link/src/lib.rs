//! The phone link: pairing, and authenticated connections between a SourceLight
//! desktop and its phones (docs/MOBILE.md §3).
//!
//! **No accounts.** Each device has an Ed25519 key. A phone becomes trusted by
//! scanning a QR code on the desktop's screen ([`Desktop::offer`]), which holds
//! a one-time token; presenting it, and a click on the desktop, adds the
//! phone's key to the desktop's list ([`PeerStore`]). After that the desktop
//! accepts that key and no other.
//!
//! **Transport is iroh.** QUIC with TLS authenticated by those same keys, so
//! the handshake itself proves who each end is and nothing in between can read
//! the traffic. iroh connects directly when it can (same network, or through
//! the router) and through a relay when it cannot, with the same code. A relay
//! passes encrypted packets and stores nothing, so away-from-home needs no
//! accounts either.
//!
//! **This crate stays free of iroh in its public API** apart from the key and
//! address strings, so the apps that use it do not depend on iroh's types.
//! Keys are passed in as bytes; storing them (a file on the desktop, the
//! Keychain or Keystore on a phone) is the app's job.

#![warn(clippy::undocumented_unsafe_blocks)]

mod desktop;
mod frame;
mod phone;

#[cfg(test)]
mod tests;

use std::net::SocketAddr;

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, RelayMode, RelayUrl, SecretKey, TransportAddr};

pub use desktop::{
    Approver, Desktop, DesktopConfig, PairCandidate, PeerStore, Service, APPROVAL_TIMEOUT,
    OFFER_TTL,
};
pub use frame::{FrameError, MAX_BODY, MAX_HEADER};
pub use phone::{PairedDesktop, Phone, Session};
pub use sl_protocol as protocol;

/// Which relays an endpoint uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relay {
    /// No relay: direct connections only, so the phone reaches the desktop on
    /// the same network or through a VPN, and nowhere else.
    Off,
    /// n0's free public relays. Rate-limited and liable to change, so for
    /// development only; a release ships with [`Relay::Custom`] pointing at
    /// our own.
    Public,
    /// These relays, by URL.
    Custom(Vec<String>),
}

/// What went wrong on the link.
#[derive(Debug)]
pub enum LinkError {
    /// The desktop answered and said no.
    Refused(protocol::Refusal),
    /// Could not bind, reach the other side, or keep the connection.
    Network(String),
    /// The other side sent something this version cannot read.
    Protocol(String),
    /// A key, address or relay URL that does not parse.
    BadAddress(String),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::Refused(r) => write!(f, "{r}"),
            LinkError::Network(e) => write!(f, "network: {e}"),
            LinkError::Protocol(e) => write!(f, "protocol: {e}"),
            LinkError::BadAddress(e) => write!(f, "bad address: {e}"),
        }
    }
}

impl std::error::Error for LinkError {}

impl From<FrameError> for LinkError {
    fn from(e: FrameError) -> Self {
        match e {
            FrameError::Io(e) => LinkError::Network(e.to_string()),
            FrameError::Invalid(why) => LinkError::Protocol(why),
        }
    }
}

/// A new device key, for the app to store and pass back on every start.
/// Losing it means pairing again; it never leaves the device.
pub fn new_secret_key() -> [u8; 32] {
    SecretKey::generate().to_bytes()
}

/// The public key, as the other side and the pairing code write it, of the
/// device with this secret key.
pub fn public_key_of(secret: &[u8; 32]) -> String {
    SecretKey::from_bytes(secret).public().to_string()
}

/// Options that tests need and the apps must never set.
#[derive(Debug, Clone, Default)]
pub(crate) struct TestOptions {
    /// Trust any certificate a relay presents: a local test relay signs its
    /// own.
    #[cfg_attr(not(test), allow(dead_code))]
    pub insecure_relay: bool,
}

/// Binds an iroh endpoint with this key and relay choice. `accept` lists the
/// ALPNs it answers; a phone accepts none.
pub(crate) async fn bind(
    secret: &[u8; 32],
    relay: &Relay,
    accept: Vec<Vec<u8>>,
    test: &TestOptions,
) -> Result<Endpoint, LinkError> {
    let relay_mode = match relay {
        Relay::Off => RelayMode::Disabled,
        Relay::Public => RelayMode::Default,
        Relay::Custom(urls) => RelayMode::custom(
            urls.iter()
                .map(|u| parse_relay(u))
                .collect::<Result<Vec<_>, _>>()?,
        ),
    };
    // `Minimal` sets only the crypto provider: no n0 address-lookup service.
    // Each side learns the other's addresses from pairing and from every
    // connection, so nothing about either device is published anywhere.
    let phone = accept.is_empty();
    let builder = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::from_bytes(secret))
        .alpns(accept)
        .relay_mode(relay_mode);
    // A phone sends its packets one at a time. Sent in batches (UDP
    // segmentation offload), a request of more than one packet reached the
    // desktop from the Android emulator as its first packet only, the rest
    // seconds or a minute later, holding up everything behind it; some phone
    // drivers and networks mangle batches the same way. A phone sends
    // little, so batching saves it nothing worth that.
    let builder = if phone {
        builder.transport_config(
            iroh::endpoint::QuicTransportConfig::builder()
                .enable_segmentation_offload(false)
                .build(),
        )
    } else {
        builder
    };
    // iroh only offers this with its `test-utils` feature, which only the
    // tests turn on (a dev-dependency), so a shipped build cannot reach it.
    #[cfg(test)]
    let builder = if test.insecure_relay {
        builder.ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify())
    } else {
        builder
    };
    #[cfg(not(test))]
    let _ = test;
    builder
        .bind()
        .await
        .map_err(|e| LinkError::Network(e.to_string()))
}

fn parse_relay(url: &str) -> Result<RelayUrl, LinkError> {
    url.parse()
        .map_err(|e: iroh::RelayUrlParseError| LinkError::BadAddress(format!("{url}: {e}")))
}

/// An endpoint's addresses as the strings a pairing code and a stored desktop
/// carry: direct `ip:port`s, and the relay if it has one.
pub(crate) fn addr_strings(addr: &EndpointAddr) -> (Vec<String>, Option<String>) {
    let ips = addr.ip_addrs().map(|a| a.to_string()).collect();
    let relay = addr.relay_urls().next().map(|u| u.to_string());
    (ips, relay)
}

/// The reverse of [`addr_strings`], for dialling.
pub(crate) fn endpoint_addr(
    id: &str,
    addrs: &[String],
    relay: Option<&str>,
) -> Result<EndpointAddr, LinkError> {
    let id: iroh::EndpointId = id
        .parse()
        .map_err(|e: iroh::KeyParsingError| LinkError::BadAddress(format!("{id}: {e}")))?;
    let mut transports = Vec::new();
    for a in addrs {
        // A malformed address is skipped rather than fatal: the others, or the
        // relay, may still reach the desktop.
        if let Ok(sa) = a.parse::<SocketAddr>() {
            transports.push(TransportAddr::Ip(sa));
        }
    }
    if let Some(r) = relay {
        transports.push(TransportAddr::Relay(parse_relay(r)?));
    }
    Ok(EndpointAddr::from_parts(id, transports))
}
