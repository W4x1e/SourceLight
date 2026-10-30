//! What SourceLight's desktop and phone say to each other (docs/MOBILE.md §3).
//!
//! Only types and their encodings live here: no networking, no files. The link
//! itself (`sl-link`) moves these over iroh, and both apps depend on this crate,
//! so the two sides cannot disagree about a message's shape.
//!
//! Every message is JSON. A request or response may carry a body of raw bytes
//! after it (a thumbnail, a GPX track); the framing for that is in `sl-link`.
//! JSON keeps the control messages readable in a log and lets either side add a
//! field with `#[serde(default)]` without breaking an older peer.

#![warn(clippy::undocumented_unsafe_blocks)]

pub mod track;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};

/// The ALPN both sides offer. A peer speaking anything else never gets past the
/// QUIC handshake. The number moves only with a change an older peer cannot
/// read; additions within a version are new optional fields.
pub const ALPN: &[u8] = b"sourcelight/1";

/// The version inside the ALPN, said again in [`Request::Hello`] so a peer can
/// report "update the phone app" rather than failing obscurely.
pub const PROTOCOL_VERSION: u16 = 1;

/// The scheme of the link the desktop's QR code holds. A phone camera that
/// opens it hands it to the app; anything else ignores it.
const OFFER_PREFIX: &str = "sourcelight://pair#";

/// What the desktop's QR code carries: enough for a phone to find this
/// desktop, prove it was shown the code, and say who it is pairing with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingOffer {
    /// The desktop's public key, as iroh writes an endpoint id. The phone dials
    /// this key and the handshake proves the other end holds it.
    pub desktop: String,
    /// The desktop's addresses on its network, `ip:port`. Dialled directly, so
    /// the phone needs no multicast to find it (which iOS gates behind an
    /// entitlement).
    #[serde(default)]
    pub addrs: Vec<String>,
    /// The relay the desktop is reachable through, when away-from-home is on.
    #[serde(default)]
    pub relay: Option<String>,
    /// The one-time token, base64url. Shown only on the desktop's screen, so
    /// presenting it is what proves the phone was in front of it.
    pub token: String,
    /// The computer's name, for the phone to show before and after pairing.
    pub name: String,
}

/// Why a pairing link could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferError {
    /// Not a SourceLight pairing link at all.
    NotAnOffer,
    /// The right scheme, but the contents are damaged or from a newer app.
    Malformed(String),
}

impl std::fmt::Display for OfferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OfferError::NotAnOffer => write!(f, "this is not a SourceLight pairing code"),
            OfferError::Malformed(why) => write!(f, "this pairing code can't be read: {why}"),
        }
    }
}

impl std::error::Error for OfferError {}

impl PairingOffer {
    /// The link to draw as a QR code.
    pub fn to_uri(&self) -> String {
        // A struct of strings always serialises; nothing here can fail.
        let json = serde_json::to_vec(self).expect("an offer serialises");
        format!("{OFFER_PREFIX}{}", URL_SAFE_NO_PAD.encode(json))
    }

    /// Reads a link scanned from a QR code.
    pub fn from_uri(uri: &str) -> Result<Self, OfferError> {
        let encoded = uri
            .trim()
            .strip_prefix(OFFER_PREFIX)
            .ok_or(OfferError::NotAnOffer)?;
        let json = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|e| OfferError::Malformed(e.to_string()))?;
        serde_json::from_slice(&json).map_err(|e| OfferError::Malformed(e.to_string()))
    }
}

/// Which phone platform a device is, for the desktop's list of paired phones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Platform {
    Ios,
    Android,
    /// A desktop build standing in for a phone: tests and development tools.
    Other,
}

/// What a phone asks. Each request travels on its own QUIC stream and gets
/// exactly one [`Response`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Request {
    /// The first request on a connection from a paired phone.
    #[serde(rename_all = "camelCase")]
    Hello {
        protocol: u16,
        app_version: String,
        device_name: String,
    },
    /// Presents the token from a [`PairingOffer`]. The only request a phone the
    /// desktop does not know may make.
    #[serde(rename_all = "camelCase")]
    Pair {
        token: String,
        device_name: String,
        platform: Platform,
    },
    /// Checks the connection is alive and measures its round trip.
    Ping,
    /// The photos that changed since `since`, a cursor from an earlier
    /// [`Response::Changes`] (or -1 for everything). At most `limit` rows.
    #[serde(rename_all = "camelCase")]
    Changes { since: i64, limit: u32 },
    /// A photo's thumbnail or preview. Answered by [`Response::Image`] with
    /// the JPEG as the body.
    #[serde(rename_all = "camelCase")]
    Image { id: i64, size: ImageSize },
    /// Decisions made on the phone, oldest first.
    #[serde(rename_all = "camelCase")]
    Verdicts { verdicts: Vec<Verdict> },
    /// A GPS track recorded on the phone: the GPX is the frame's body
    /// ([`track::to_gpx`]). The times say which shoot it covers.
    #[serde(rename_all = "camelCase")]
    Track {
        name: String,
        started_ms: i64,
        ended_ms: i64,
    },
    /// The exports waiting for a phone (Send to phone).
    Outbox,
    /// One of them: the file is the frame's body ([`Response::File`]).
    #[serde(rename_all = "camelCase")]
    OutboxFile { id: i64 },
    /// These are saved on the phone: the desktop can let them go.
    #[serde(rename_all = "camelCase")]
    Received { ids: Vec<i64> },
    /// A photo from the phone's own gallery for the library (Add to
    /// library): the file, as stored on the phone, is the frame's body.
    /// `edits` is the phone's edit stack for it, as JSON, if it has one.
    #[serde(rename_all = "camelCase")]
    AddPhoto {
        name: String,
        taken_ms: i64,
        edits: Option<String>,
    },
    /// A library photo's edits, to edit it on the phone.
    #[serde(rename_all = "camelCase")]
    Edits { id: i64 },
    /// The phone's edits to a library photo, as JSON (none for no edits).
    /// `base` is the edits it started from, as [`Response::Edits`] gave
    /// them: if the photo has been edited on the computer since, the phone's
    /// edits go on a virtual copy and the computer's stay where they are.
    #[serde(rename_all = "camelCase")]
    SetEdits {
        id: i64,
        edits: Option<String>,
        base: Option<String>,
    },
    /// The names the computer's search box recognises in a query (people,
    /// places, cameras, keywords), so the phone reads a query the same way.
    Vocabulary,
    /// The photos a search finds: `filter` is the desktop's own photo filter
    /// as JSON, made on the phone from the query as the search box makes it.
    #[serde(rename_all = "camelCase")]
    Search { filter: String },
}

/// An export waiting for a phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxEntry {
    pub id: i64,
    /// The file's name, as exported.
    pub name: String,
    pub size: u64,
}

/// Which of a photo's images.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageSize {
    /// The grid's: 512 px on the long edge.
    Thumb,
    /// The loupe's: 1600 px.
    Preview,
    /// The loupe's, as edited on the computer, raws included. A computer
    /// from before it refuses the request, and the phone shows the preview.
    Edited,
}

/// One photo as the phone keeps it: what the grid, the loupe and culling
/// need, and nothing heavier. New fields must be `#[serde(default)]`, so an
/// older phone keeps reading a newer desktop's rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoRow {
    pub id: i64,
    pub filename: String,
    /// The capture time as the catalogue writes it (`YYYY-MM-DD HH:MM:SS`,
    /// the camera's clock).
    pub captured: Option<String>,
    pub width: u32,
    pub height: u32,
    /// 0 to 5.
    pub rating: u8,
    /// -1 rejected, 0 none, 1 picked.
    pub flag: i8,
    /// Names the photo's edits as they are now, and changes when they do;
    /// none when it has none. A phone keeps it beside the edits it fetched,
    /// to know they are still current.
    #[serde(default)]
    pub edit_tag: Option<String>,
    /// Where it can be edited.
    #[serde(default)]
    pub edit_on: EditOn,
}

/// Where a library photo can be edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EditOn {
    /// On the phone too: a JPEG, PNG, TIFF, HEIC or WebP whose edits the
    /// phone can show. The phone edits the preview.
    Phone,
    /// A raw file: on the computer, which develops it from the raw data.
    Raw,
    /// Its edits need something only the computer has (erasing, AI noise
    /// reduction, a LUT): on the computer.
    Computer,
    /// Not said: a computer from before editing on the phone, or a reason
    /// newer than this phone.
    #[default]
    #[serde(other)]
    Unknown,
}

/// One decision on the phone. A field left out is left alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub id: i64,
    #[serde(default)]
    pub flag: Option<i8>,
    #[serde(default)]
    pub rating: Option<u8>,
}

/// What the desktop answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Response {
    /// The answer to [`Request::Hello`].
    #[serde(rename_all = "camelCase")]
    Welcome {
        protocol: u16,
        desktop_name: String,
    },
    /// Pairing was approved on the desktop. From now on this phone's key is
    /// accepted.
    #[serde(rename_all = "camelCase")]
    Paired {
        desktop_name: String,
    },
    Pong,
    /// The answer to [`Request::Changes`].
    #[serde(rename_all = "camelCase")]
    Changes {
        /// The catalogue's id. A different one than last time means the
        /// phone is looking at another library: start again from -1.
        catalogue: String,
        /// The catalogue's newest cursor. Lower than the phone's means the
        /// catalogue went back in time (a restored backup): start again too.
        head: i64,
        rows: Vec<PhotoRow>,
        /// Photos to drop: deleted, gone missing, or no longer shown.
        removed: Vec<i64>,
        /// Ask from here next time.
        next: i64,
        /// More are waiting: ask again straight away.
        more: bool,
    },
    /// The answer to [`Request::Image`]: the JPEG is the frame's body.
    Image,
    /// The answer to [`Request::Verdicts`]: how many were applied.
    #[serde(rename_all = "camelCase")]
    Applied {
        applied: u32,
    },
    /// The answer to [`Request::Track`]: kept, with this many points.
    #[serde(rename_all = "camelCase")]
    TrackSaved {
        points: u32,
    },
    /// The answer to [`Request::Outbox`].
    #[serde(rename_all = "camelCase")]
    Outbox {
        files: Vec<OutboxEntry>,
    },
    /// The answer to [`Request::OutboxFile`]: the file is the frame's body.
    File,
    /// The answer to [`Request::AddPhoto`]: the photo's id in the library,
    /// and whether the library had it already (the same file, sent again).
    #[serde(rename_all = "camelCase")]
    Added {
        id: i64,
        existed: bool,
    },
    /// The answer to [`Request::Edits`]: the photo's edits as JSON (none
    /// when it has none), their tag, and where it can be edited.
    #[serde(rename_all = "camelCase")]
    Edits {
        edits: Option<String>,
        tag: Option<String>,
        edit_on: EditOn,
    },
    /// The answer to [`Request::SetEdits`]. Kept on the photo, whose edits'
    /// tag is now `tag`; or, when it had been edited on the computer since,
    /// on `copy`, a new virtual copy of it.
    #[serde(rename_all = "camelCase")]
    EditsSaved {
        tag: Option<String>,
        copy: Option<i64>,
    },
    /// The answer to [`Request::Vocabulary`], as JSON.
    #[serde(rename_all = "camelCase")]
    Vocabulary {
        json: String,
    },
    /// The answer to [`Request::Search`]: the photos found, best first when
    /// `ranked` (the query asked what is in the photos), otherwise unordered.
    #[serde(rename_all = "camelCase")]
    Found {
        ids: Vec<i64>,
        ranked: bool,
    },
    /// The request was not carried out, and why.
    Refused {
        reason: Refusal,
    },
}

/// Why the desktop said no. Each is something the phone can explain plainly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Refusal {
    /// This phone's key is not, or is no longer, paired with the desktop.
    NotPaired,
    /// The pairing token is wrong, already used, or expired.
    BadToken,
    /// Someone at the desktop declined the pairing, or did not answer in time.
    Declined,
    /// The phone speaks an older protocol than the desktop accepts.
    UpdatePhone,
    /// The desktop speaks an older protocol than the phone needs.
    UpdateDesktop,
    /// The request was not valid for this connection.
    BadRequest,
    /// No such photo, or it has no image of that size yet.
    NotFound,
    /// The desktop could not carry it out; trying again later may work.
    Failed,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Refusal::NotPaired => "This phone is not paired with that computer.",
            Refusal::BadToken => "That pairing code has expired or was already used.",
            Refusal::Declined => "Pairing was declined on the computer.",
            Refusal::UpdatePhone => "Update SourceLight on this phone.",
            Refusal::UpdateDesktop => "Update SourceLight on the computer.",
            Refusal::BadRequest => "The computer did not understand the request.",
            Refusal::NotFound => "The computer has no such photo.",
            Refusal::Failed => "The computer could not do that just now.",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer() -> PairingOffer {
        PairingOffer {
            desktop: "ae58ff8833241ac82d6ff7611046ed67b5072d142c588d0063e942d9a75502b6".into(),
            addrs: vec!["192.168.1.20:52144".into(), "[fe80::1]:52144".into()],
            relay: Some("https://relay.example.".into()),
            token: "q83vEjRWeJA".into(),
            name: "Studio PC".into(),
        }
    }

    #[test]
    fn an_offer_survives_its_qr_link() {
        let o = offer();
        let uri = o.to_uri();
        assert!(uri.starts_with("sourcelight://pair#"));
        assert_eq!(PairingOffer::from_uri(&uri).unwrap(), o);
        // Scanners sometimes hand over trailing whitespace.
        assert_eq!(PairingOffer::from_uri(&format!(" {uri}\n")).unwrap(), o);
    }

    /// A QR code holds about 2.9 KB in binary mode, and a dense one is slow to
    /// scan. A realistic offer should stay far below that.
    #[test]
    fn an_offer_makes_a_small_qr_code() {
        let mut o = offer();
        o.addrs = (0..6).map(|i| format!("192.168.1.{i}:52144")).collect();
        assert!(o.to_uri().len() < 600, "{}", o.to_uri().len());
    }

    #[test]
    fn other_links_are_not_offers() {
        assert_eq!(
            PairingOffer::from_uri("https://example.com"),
            Err(OfferError::NotAnOffer)
        );
        assert!(matches!(
            PairingOffer::from_uri("sourcelight://pair#!!!"),
            Err(OfferError::Malformed(_))
        ));
    }

    /// A newer desktop may add fields; an older phone must still read the code.
    #[test]
    fn an_offer_with_unknown_fields_still_reads() {
        let json = r#"{"desktop":"k","token":"t","name":"n","futureField":3}"#;
        let uri = format!("sourcelight://pair#{}", URL_SAFE_NO_PAD.encode(json));
        let o = PairingOffer::from_uri(&uri).unwrap();
        assert!(o.addrs.is_empty());
        assert_eq!(o.relay, None);
    }

    #[test]
    fn library_messages_round_trip() {
        let changes = Response::Changes {
            catalogue: "c1".into(),
            head: 42,
            rows: vec![PhotoRow {
                id: 7,
                filename: "DSC0001.ARW".into(),
                captured: Some("2026-09-18 10:00:00".into()),
                width: 6000,
                height: 4000,
                rating: 3,
                flag: -1,
                edit_tag: Some("9f86d081884c7d65".into()),
                edit_on: EditOn::Raw,
            }],
            removed: vec![9],
            next: 42,
            more: false,
        };
        let json = serde_json::to_string(&changes).unwrap();
        assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), changes);

        let verdicts = Request::Verdicts {
            verdicts: vec![Verdict {
                id: 7,
                flag: Some(1),
                rating: None,
            }],
        };
        let json = serde_json::to_string(&verdicts).unwrap();
        assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), verdicts);
        // A verdict that says nothing about the rating leaves it alone.
        let bare: Verdict = serde_json::from_str(r#"{"id":1,"flag":0}"#).unwrap();
        assert_eq!(bare.rating, None);

        assert_eq!(
            serde_json::to_string(&Request::Image {
                id: 3,
                size: ImageSize::Preview
            })
            .unwrap(),
            r#"{"type":"image","id":3,"size":"preview"}"#
        );
    }

    /// A row from a computer before editing on the phone reads as unedited,
    /// edited nowhere known; a reason newer than the phone reads the same.
    #[test]
    fn edit_fields_default_and_tolerate_newer_values() {
        let old = r#"{"id":1,"filename":"a.jpg","captured":null,"width":4,"height":3,"rating":0,"flag":0}"#;
        let row: PhotoRow = serde_json::from_str(old).unwrap();
        assert_eq!(row.edit_tag, None);
        assert_eq!(row.edit_on, EditOn::Unknown);

        let newer = r#"{"id":1,"filename":"a.jpg","captured":null,"width":4,"height":3,"rating":0,"flag":0,"editOn":"tablet"}"#;
        let row: PhotoRow = serde_json::from_str(newer).unwrap();
        assert_eq!(row.edit_on, EditOn::Unknown);

        assert_eq!(serde_json::to_string(&EditOn::Phone).unwrap(), r#""phone""#);
        let set = Request::SetEdits {
            id: 4,
            edits: Some(r#"{"exposure":0.5}"#.into()),
            base: None,
        };
        let json = serde_json::to_string(&set).unwrap();
        assert!(json.starts_with(r#"{"type":"setEdits""#), "{json}");
        assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), set);
    }

    #[test]
    fn messages_are_tagged_json() {
        let r = Request::Pair {
            token: "t".into(),
            device_name: "Phone".into(),
            platform: Platform::Ios,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(
            json,
            r#"{"type":"pair","token":"t","deviceName":"Phone","platform":"ios"}"#
        );
        assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), r);
        let refused = Response::Refused {
            reason: Refusal::NotPaired,
        };
        assert_eq!(
            serde_json::to_string(&refused).unwrap(),
            r#"{"type":"refused","reason":"notPaired"}"#
        );
    }
}
