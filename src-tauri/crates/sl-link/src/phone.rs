//! The phone's end: pairs with a desktop, then connects to it.

use std::time::{Duration, Instant};

use iroh::endpoint::Connection;
use iroh::Endpoint;
use serde::{Deserialize, Serialize};
use sl_protocol::{PairingOffer, Platform, Refusal, Request, Response, ALPN, PROTOCOL_VERSION};

use crate::frame::{read_frame, write_frame, Watched};
use crate::{bind, endpoint_addr, LinkError, Relay, TestOptions, APPROVAL_TIMEOUT};

/// How long a request may go without moving a byte before it is given up. A
/// connection can stop carrying data and stay open, and a request waiting on
/// it would wait for ever, holding up everything queued behind it; given up,
/// the phone drops the connection and makes another.
pub const STALL: Duration = Duration::from_secs(30);

/// A desktop this phone has paired with: what it stores, and dials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedDesktop {
    /// The desktop's public key. The connection fails unless the other end
    /// holds the matching secret key.
    pub id: String,
    pub name: String,
    /// Last known direct addresses, `ip:port`.
    pub addrs: Vec<String>,
    /// Last known relay, for reaching it away from home.
    pub relay: Option<String>,
}

/// A running phone end.
pub struct Phone {
    endpoint: Endpoint,
}

impl Phone {
    /// Starts the phone's endpoint. A phone only dials, so it accepts nothing.
    pub async fn start(secret_key: [u8; 32], relay: Relay) -> Result<Self, LinkError> {
        Self::start_with(secret_key, relay, TestOptions::default()).await
    }

    pub(crate) async fn start_with(
        secret_key: [u8; 32],
        relay: Relay,
        test: TestOptions,
    ) -> Result<Self, LinkError> {
        let endpoint = bind(&secret_key, &relay, Vec::new(), &test).await?;
        Ok(Phone { endpoint })
    }

    /// This phone's public key.
    pub fn id(&self) -> String {
        self.endpoint.id().to_string()
    }

    /// Pairs with the desktop whose code was scanned. Resolves once the person
    /// at the desktop approves, which can take a while.
    pub async fn pair(
        &self,
        offer: &PairingOffer,
        device_name: &str,
        platform: Platform,
    ) -> Result<PairedDesktop, LinkError> {
        let desktop = PairedDesktop {
            id: offer.desktop.clone(),
            name: offer.name.clone(),
            addrs: offer.addrs.clone(),
            relay: offer.relay.clone(),
        };
        let session = self.connect(&desktop).await?;
        let response = session
            .request(
                &Request::Pair {
                    token: offer.token.clone(),
                    device_name: device_name.to_string(),
                    platform,
                },
                &[],
            )
            .await?
            .0;
        session.close();
        match response {
            Response::Paired { desktop_name } => Ok(PairedDesktop {
                name: desktop_name,
                ..desktop
            }),
            Response::Refused { reason } => Err(LinkError::Refused(reason)),
            other => Err(LinkError::Protocol(format!(
                "unexpected answer to pairing: {other:?}"
            ))),
        }
    }

    /// Connects to a paired desktop. Says nothing yet: call
    /// [`Session::hello`] first.
    pub async fn connect(&self, desktop: &PairedDesktop) -> Result<Session, LinkError> {
        let addr = endpoint_addr(&desktop.id, &desktop.addrs, desktop.relay.as_deref())?;
        let connection = self
            .endpoint
            .connect(addr, ALPN)
            .await
            .map_err(|e| LinkError::Network(e.to_string()))?;
        Ok(Session { connection })
    }

    /// Closes the endpoint and every connection on it.
    pub async fn shutdown(self) {
        self.endpoint.close().await;
    }
}

/// One connection to a desktop. Cheap to clone: clones share the connection,
/// so many requests (a screenful of thumbnails) can run at once.
#[derive(Clone)]
pub struct Session {
    connection: Connection,
}

impl Session {
    /// Sends one request and waits for its answer. Requests may run
    /// concurrently; each has its own stream.
    pub async fn request(
        &self,
        request: &Request,
        body: &[u8],
    ) -> Result<(Response, Vec<u8>), LinkError> {
        // Pairing waits on the person at the desktop, saying nothing meanwhile.
        let limit = match request {
            Request::Pair { .. } => APPROVAL_TIMEOUT + STALL,
            _ => STALL,
        };
        let (send, recv) = tokio::time::timeout(limit, self.connection.open_bi())
            .await
            .map_err(|_| LinkError::Network("the desktop stopped answering".into()))?
            .map_err(|e| LinkError::Network(e.to_string()))?;
        let mut send = Watched::new(send, limit);
        let mut recv = Watched::new(recv, limit);
        write_frame(&mut send, request, body).await?;
        send.get_mut()
            .finish()
            .map_err(|e| LinkError::Network(e.to_string()))?;
        Ok(read_frame(&mut recv).await?)
    }

    /// Introduces this phone. Returns the desktop's current name.
    pub async fn hello(&self, app_version: &str, device_name: &str) -> Result<String, LinkError> {
        let request = Request::Hello {
            protocol: PROTOCOL_VERSION,
            app_version: app_version.to_string(),
            device_name: device_name.to_string(),
        };
        match self.request(&request, &[]).await?.0 {
            Response::Welcome {
                protocol,
                desktop_name,
            } => {
                if protocol < PROTOCOL_VERSION {
                    return Err(LinkError::Refused(Refusal::UpdateDesktop));
                }
                Ok(desktop_name)
            }
            Response::Refused { reason } => Err(LinkError::Refused(reason)),
            other => Err(LinkError::Protocol(format!(
                "unexpected answer to hello: {other:?}"
            ))),
        }
    }

    /// Measures a round trip.
    pub async fn ping(&self) -> Result<Duration, LinkError> {
        let start = Instant::now();
        match self.request(&Request::Ping, &[]).await?.0 {
            Response::Pong => Ok(start.elapsed()),
            Response::Refused { reason } => Err(LinkError::Refused(reason)),
            other => Err(LinkError::Protocol(format!(
                "unexpected answer to ping: {other:?}"
            ))),
        }
    }

    /// Whether the connection currently runs over a relay rather than
    /// directly. Shown on the phone, since a relayed sync is slower.
    pub fn is_relayed(&self) -> bool {
        !self
            .connection
            .paths()
            .iter()
            .any(|p| p.is_selected() && p.is_ip())
    }

    /// Closes the connection.
    pub fn close(self) {
        self.connection.close(0u32.into(), b"bye");
    }
}
