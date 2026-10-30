//! What went wrong, in a form the interface can word in the user's language.
//!
//! A command's error used to reach the interface as the English cause chain —
//! "writing C:\…\IMG_2043.tmp: Access is denied. (os error 5)" — and the
//! interface shows its words in five languages. The chain is written by
//! whatever failed (the OS, SQLite, LibRaw, the network), so there is no fixed
//! sentence to translate. What *is* fixed is the kind of failure underneath:
//! a missing file, a full disk, a catalogue another program has locked. So
//! [`describe`] walks the chain, names the root cause's kind, and sends that
//! alongside the English:
//!
//! ```json
//! {"kind":"accessDenied","file":"IMG_2043.tmp","text":"writing C:\\…: Access is denied. (os error 5)"}
//! ```
//!
//! The interface words `kind` (docs/I18N.md) and keeps `text` for the log and
//! for diagnostics. A failure it cannot classify goes as the plain English
//! chain, as before — and so does cancelling a job, which the interface
//! recognises by its exact text (`jobs::CANCELLED`).

use std::error::Error as StdError;
use std::io::ErrorKind;

use serde::Serialize;

/// The payload the interface reads. Field names are the interface's contract.
#[derive(Serialize)]
struct Described<'a> {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<u16>,
    text: &'a str,
}

/// An error for the interface: classified where the cause is recognised,
/// otherwise its English cause chain.
pub fn describe(e: &anyhow::Error) -> String {
    let text = format!("{e:#}");
    encode(&text, e.chain())
}

/// [`describe`] for an error that is not an `anyhow::Error`.
pub fn describe_std(e: &(dyn StdError + 'static)) -> String {
    let text = chain_text(e);
    encode(&text, std::iter::successors(Some(e), |&e| e.source()))
}

fn chain_text(e: &(dyn StdError + 'static)) -> String {
    let mut text = e.to_string();
    let mut next = e.source();
    while let Some(cause) = next {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        next = cause.source();
    }
    text
}

fn encode<'a>(text: &str, chain: impl Iterator<Item = &'a (dyn StdError + 'static)>) -> String {
    let chain: Vec<_> = chain.collect();
    if chain
        .iter()
        .any(|c| c.downcast_ref::<crate::jobs::Cancelled>().is_some())
    {
        return text.to_owned();
    }
    // The root cause says what went wrong; the contexts above it say what was
    // being done. Classify from the root outwards.
    let Some((kind, status)) = chain.iter().rev().find_map(|c| classify(*c)) else {
        return text.to_owned();
    };
    let file = chain.iter().find_map(|c| file_in(&c.to_string()));
    serde_json::to_string(&Described {
        kind,
        file,
        status,
        text,
    })
    .unwrap_or_else(|_| text.to_owned())
}

/// The kind of one error in a chain, if it is one the interface can word.
pub fn classify(e: &(dyn StdError + 'static)) -> Option<(&'static str, Option<u16>)> {
    if let Some(io) = e.downcast_ref::<std::io::Error>() {
        return Some((io_kind(io), None));
    }
    if let Some(e) = e.downcast_ref::<rusqlite::Error>() {
        return Some((sqlite_kind(e), None));
    }
    if let Some(e) = e.downcast_ref::<reqwest::Error>() {
        let status = e.status().map(|s| s.as_u16());
        let kind = if e.is_timeout() {
            "timedOut"
        } else if e.is_connect() {
            "offline"
        } else if status.is_some() {
            "httpStatus"
        } else {
            "network"
        };
        return Some((kind, status));
    }
    if let Some(e) = e.downcast_ref::<image::ImageError>() {
        return Some((image_kind(e), None));
    }
    #[cfg(feature = "libraw")]
    if let Some(e) = e.downcast_ref::<crate::raw::libraw::LibRawError>() {
        return Some((libraw_kind(e.code), None));
    }
    #[cfg(feature = "ai")]
    if e.downcast_ref::<ort::Error>().is_some() {
        return Some(("aiRuntime", None));
    }
    None
}

fn io_kind(e: &std::io::Error) -> &'static str {
    // The OS's own codes first: they tell apart what `ErrorKind` lumps
    // together, a file another program holds open above all.
    if let Some(code) = e.raw_os_error() {
        #[cfg(windows)]
        match code {
            2 | 3 => return "notFound",
            5 => return "accessDenied",
            // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION
            32 | 33 => return "fileInUse",
            // ERROR_WRITE_PROTECT
            19 => return "readOnly",
            // ERROR_NOT_READY, ERROR_DEV_NOT_EXIST: a drive that is not there
            21 | 55 => return "driveMissing",
            // ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL
            39 | 112 => return "diskFull",
            _ => {}
        }
        #[cfg(unix)]
        match code {
            libc::ENOSPC => return "diskFull",
            libc::EROFS => return "readOnly",
            libc::EBUSY | libc::ETXTBSY => return "fileInUse",
            _ => {}
        }
        #[cfg(not(any(windows, unix)))]
        let _ = code;
    }
    match e.kind() {
        ErrorKind::NotFound => "notFound",
        ErrorKind::PermissionDenied => "accessDenied",
        ErrorKind::AlreadyExists => "alreadyExists",
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded => "diskFull",
        ErrorKind::ReadOnlyFilesystem => "readOnly",
        ErrorKind::ResourceBusy => "fileInUse",
        ErrorKind::TimedOut => "timedOut",
        ErrorKind::ConnectionRefused
        | ErrorKind::ConnectionReset
        | ErrorKind::ConnectionAborted
        | ErrorKind::NotConnected
        | ErrorKind::HostUnreachable
        | ErrorKind::NetworkUnreachable
        | ErrorKind::NetworkDown => "offline",
        ErrorKind::InvalidData | ErrorKind::UnexpectedEof => "fileDamaged",
        ErrorKind::OutOfMemory => "outOfMemory",
        _ => "io",
    }
}

fn sqlite_kind(e: &rusqlite::Error) -> &'static str {
    use rusqlite::ErrorCode as C;
    match e.sqlite_error_code() {
        Some(C::DatabaseBusy | C::DatabaseLocked) => "catalogueBusy",
        Some(C::DatabaseCorrupt | C::NotADatabase) => "catalogueDamaged",
        Some(C::ReadOnly) => "catalogueReadOnly",
        Some(C::DiskFull) => "diskFull",
        Some(C::CannotOpen) => "catalogueCannotOpen",
        Some(C::OutOfMemory) => "outOfMemory",
        _ => "catalogue",
    }
}

fn image_kind(e: &image::ImageError) -> &'static str {
    match e {
        image::ImageError::Unsupported(_) => "imageUnsupported",
        image::ImageError::Decoding(_) => "fileDamaged",
        image::ImageError::Limits(_) => "outOfMemory",
        image::ImageError::IoError(io) => io_kind(io),
        _ => "image",
    }
}

/// LibRaw's error codes (libraw_const.h, `LibRaw_errors`).
#[cfg(feature = "libraw")]
fn libraw_kind(code: i32) -> &'static str {
    match code {
        -2 => "rawUnsupported",
        -100_008 => "fileDamaged",
        -100_009 => "io",
        -100_007 | -100_012 | -100_013 => "outOfMemory",
        _ => "raw",
    }
}

/// The file an error's context names, if it names one: "writing C:\a\b.tmp"
/// is `b.tmp`. The context owns the text up to its end, so a path runs from
/// where it starts to there. Only the name is sent — the folders are the
/// user's business and make the message long.
fn file_in(context: &str) -> Option<String> {
    let bytes = context.as_bytes();
    let start = (0..bytes.len()).find(|&i| {
        let at_word =
            i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\'' || bytes[i - 1] == b'"';
        at_word
            && (bytes[i..].starts_with(b"\\\\")
                || (bytes.len() > i + 2
                    && bytes[i].is_ascii_alphabetic()
                    && bytes[i + 1] == b':'
                    && (bytes[i + 2] == b'\\' || bytes[i + 2] == b'/'))
                || (bytes[i] == b'/'
                    && bytes.get(i + 1).is_some_and(|b| b.is_ascii_alphanumeric())))
    })?;
    let path = context[start..].trim_end_matches(['\'', '"', '.', ')', ']']);
    // Both separators on every platform: `Path` splits on `\` only on Windows,
    // and a Windows path can reach a Mac in a message (a moved catalogue).
    let name = path.rsplit(['/', '\\']).next()?.to_string();
    Some(name).filter(|n| !n.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn json(s: &str) -> Value {
        serde_json::from_str(s).expect("a classified error is JSON")
    }

    #[test]
    fn a_missing_file_says_so_and_names_it() {
        let e = std::io::Error::from(ErrorKind::NotFound);
        let e = anyhow::Error::new(e).context(r"reading C:\Photos\2024\IMG_2043.ARW");
        let v = json(&describe(&e));
        assert_eq!(v["kind"], "notFound");
        assert_eq!(v["file"], "IMG_2043.ARW");
        assert!(v["text"]
            .as_str()
            .unwrap()
            .starts_with(r"reading C:\Photos"));
    }

    #[test]
    fn the_root_cause_decides_not_the_context() {
        let e = anyhow::Error::new(std::io::Error::from(ErrorKind::PermissionDenied))
            .context("writing /Users/anna/Pictures/out.jpg")
            .context("exporting");
        let v = json(&describe(&e));
        assert_eq!(v["kind"], "accessDenied");
        assert_eq!(v["file"], "out.jpg");
    }

    #[cfg(windows)]
    #[test]
    fn a_file_another_program_holds_is_in_use() {
        let e = anyhow::Error::new(std::io::Error::from_raw_os_error(32));
        assert_eq!(json(&describe(&e))["kind"], "fileInUse");
        let e = anyhow::Error::new(std::io::Error::from_raw_os_error(112));
        assert_eq!(json(&describe(&e))["kind"], "diskFull");
    }

    #[test]
    fn a_busy_catalogue_is_busy() {
        let e = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            Some("database is locked".into()),
        );
        assert_eq!(json(&describe_std(&e))["kind"], "catalogueBusy");
        let e = anyhow::Error::new(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
            None,
        ));
        assert_eq!(json(&describe(&e))["kind"], "catalogueDamaged");
    }

    #[test]
    fn cancelling_stays_the_plain_word() {
        let e = anyhow::Error::new(crate::jobs::Cancelled).context("tagging");
        assert_eq!(describe(&e), format!("tagging: {}", crate::jobs::CANCELLED));
    }

    #[test]
    fn what_it_cannot_classify_goes_as_english() {
        let e = anyhow::anyhow!("model has no inputs").context("loading the tagger");
        assert_eq!(describe(&e), "loading the tagger: model has no inputs");
    }

    #[test]
    fn a_file_name_is_found_only_where_a_path_starts() {
        assert_eq!(
            file_in(r"cannot write D:\Out\a b.tif").as_deref(),
            Some("a b.tif")
        );
        assert_eq!(
            file_in("finishing '/tmp/x/y.dng'").as_deref(),
            Some("y.dng")
        );
        assert_eq!(file_in("encoding PNG"), None);
        assert_eq!(file_in("ratio 3/2 failed"), None);
    }
}
