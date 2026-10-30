//! The phone link on the desktop (docs/MOBILE.md §3): this catalogue's key,
//! the phones paired with it, and the listener they connect to.
//!
//! **Built only with the `phone-link` feature** until the phone app exists, so
//! a release neither listens on the network nor carries iroh. Without it every
//! command still exists (the bindings do not change with features) but reports
//! the link as unavailable, and Settings hides the Phone tab.
//!
//! **What lives in the catalogue:** the desktop's secret key, whether the link
//! is on, whether away-from-home is on, and the paired phones. Keeping the key
//! with the catalogue means a catalogue moved to a new computer keeps its
//! phones, and a restored backup brings back the phones it had.

use std::path::Path;

use anyhow::Result;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use specta::Type;

use crate::db;
use crate::develop::EditStack;

/// Settings keys.
pub const ENABLED: &str = "phone.enabled";
pub const AWAY_FROM_HOME: &str = "phone.awayFromHome";
const SECRET_KEY: &str = "phone.secretKey";
/// Where photos added from a phone go; unset for the default.
const IMPORT_FOLDER: &str = "phone.importFolder";
const CATALOGUE_ID: &str = "catalogue.id";
/// The name of the virtual copy a phone's edits go on when the photo was
/// edited on the computer meanwhile.
const FROM_PHONE: &str = "From phone";
/// The undo step a phone's edit makes in Develop, when the photo is open.
pub const EDITED_ON_PHONE: &str = "Edited on phone";

/// One photo as a phone keeps it (sl-protocol's `PhotoRow`, which only
/// exists in a build with the phone link).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneRow {
    pub id: i64,
    pub filename: String,
    pub captured: Option<String>,
    /// As shown: turned for EXIF orientations 5 to 8.
    pub width: u32,
    pub height: u32,
    pub rating: u8,
    pub flag: i8,
    /// See [`edit_tag`].
    pub edit_tag: Option<String>,
    pub edit_on: EditOn,
}

/// Where a photo can be edited, for a phone (sl-protocol's `EditOn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditOn {
    /// The phone too: it edits the preview, and shows the edit as the
    /// computer does.
    Phone,
    /// A raw file, developed from the raw data, which the phone's preview is
    /// not.
    Raw,
    /// Its edits need something only the computer has.
    Computer,
}

/// Where photo `path`, with edits `settings` (as stored), can be edited.
pub fn edit_on(path: &str, settings: Option<&str>) -> EditOn {
    if !crate::raster::is_raster(Path::new(path)) {
        return EditOn::Raw;
    }
    match settings.map(EditStack::from_json) {
        None => EditOn::Phone,
        Some(Ok(stack)) if phone_can_show(&stack) => EditOn::Phone,
        // Edits this build can't read are left to the computer.
        Some(_) => EditOn::Computer,
    }
}

/// Whether a phone shows this edit as the computer does: everything it needs
/// is in the edit itself. Erasing fills from files beside the catalogue, AI
/// noise reduction runs a model, and an imported LUT is a file in the LUTs
/// folder. The built-in film looks are made in code, so a phone has them.
pub fn phone_can_show(stack: &EditStack) -> bool {
    stack.version <= crate::develop::stack::CURRENT_VERSION
        && stack.erases.is_empty()
        && stack.detail.ai_denoise <= 0.0
        && (stack.lut.name.is_empty()
            || crate::develop::lut_packs::describe(&stack.lut.name).is_some())
}

/// A short name for a photo's edits as stored, which changes when they do;
/// none when there are none. A phone keeps it beside edits it fetched, so it
/// knows from the change feed whether they are still current.
pub fn edit_tag(settings: Option<&str>) -> Option<String> {
    settings.map(|json| crate::hashing::sha256_hex(json.as_bytes())[..16].to_string())
}

/// A library photo's edits as stored, and where it can be edited, for a
/// phone about to edit it. `None` when there is no such photo, or it is
/// missing. A copy counts: a phone whose edit went on one carries on there.
pub fn photo_edits(conn: &Connection, id: i64) -> Result<Option<(Option<String>, EditOn)>> {
    let row: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT path, develop_settings FROM photos WHERE id = ?1 AND missing = 0",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(row.map(|(path, settings)| {
        let on = edit_on(&path, settings.as_deref());
        (settings, on)
    }))
}

/// Where a phone's edits to a library photo go.
#[derive(Debug, Clone, PartialEq)]
pub enum PhoneEdit {
    /// On the photo: nobody changed it since the phone fetched its edits.
    Photo(EditStack),
    /// On a new virtual copy: the photo was edited on the computer since,
    /// and those edits stay on it.
    Copy(EditStack),
}

/// Decides where a phone's edits to photo `id` go (see [`PhoneEdit`]).
/// `base` is the edits the phone started from, as it was sent them; `edits`
/// is the phone's edit now, laid over `base` so a setting the phone's app is
/// too old to know is kept. `None` when there is no such photo.
pub fn phone_edit(
    conn: &Connection,
    id: i64,
    edits: Option<&str>,
    base: Option<&str>,
) -> Result<Option<PhoneEdit>> {
    let Some((current, on)) = photo_edits(conn, id)? else {
        return Ok(None);
    };
    anyhow::ensure!(on != EditOn::Raw, "a raw file is edited on the computer");
    let stack = laid_over(base, edits)?;
    Ok(Some(if same_edit(current.as_deref(), base) {
        PhoneEdit::Photo(stack)
    } else {
        PhoneEdit::Copy(stack)
    }))
}

/// Keeps a phone's edits as [`phone_edit`] decided, when the photo isn't
/// open in Develop. Returns the copy's id when it made one.
pub fn keep_phone_edit(conn: &mut Connection, id: i64, edit: &PhoneEdit) -> Result<Option<i64>> {
    match edit {
        PhoneEdit::Photo(stack) => {
            db::set_develop_settings(conn, id as i32, stack)?;
            Ok(None)
        }
        PhoneEdit::Copy(stack) => {
            let copy = db::create_variant(conn, id as i32, Some(FROM_PHONE))?;
            db::set_develop_settings(conn, copy, stack)?;
            Ok(Some(i64::from(copy)))
        }
    }
}

/// Whether two stored edits are the same edit; none is no edit.
fn same_edit(a: Option<&str>, b: Option<&str>) -> bool {
    let read = |s: Option<&str>| -> Result<EditStack> {
        s.map(EditStack::from_json)
            .transpose()
            .map(Option::unwrap_or_default)
    };
    match (read(a), read(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// The phone's edit laid over the one it started from: fields the phone
/// sent replace the base's, and fields it doesn't know stay. No edit is no
/// edit, whatever the base had.
fn laid_over(base: Option<&str>, edits: Option<&str>) -> Result<EditStack> {
    let Some(edits) = edits else {
        return Ok(EditStack::default());
    };
    let mut merged = match base.map(serde_json::from_str::<serde_json::Value>) {
        Some(Ok(v)) if v.is_object() => v,
        _ => serde_json::Value::Object(Default::default()),
    };
    lay_over(&mut merged, serde_json::from_str(edits)?);
    EditStack::from_json(&merged.to_string())
}

fn lay_over(into: &mut serde_json::Value, from: serde_json::Value) {
    match (into, from) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            for (k, v) in b {
                lay_over(a.entry(k).or_insert(serde_json::Value::Null), v);
            }
        }
        (slot, v) => *slot = v,
    }
}

/// The `phone-track` event: a phone sent a GPS track, kept as a GPX file,
/// for the Places workspace's track dialog to place photos from.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PhoneTrack {
    pub path: String,
    pub name: String,
    pub points: u32,
}

/// Keeps a phone's GPS track beside the catalogue (`tracks/`), where a later
/// change to the camera's clock can move photos along it again.
/// Refuses a file the desktop's track reader finds no points in.
pub fn save_phone_track(
    data_dir: &Path,
    started_ms: i64,
    gpx: &[u8],
) -> Result<(std::path::PathBuf, u32)> {
    let text = std::str::from_utf8(gpx)?;
    let points = crate::gpx::parse(text).map_err(|e| anyhow::anyhow!(e))?;
    if points.is_empty() {
        anyhow::bail!("the track has no timed points");
    }
    let dir = data_dir.join("tracks");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("phone-{started_ms}.gpx"));
    std::fs::write(&path, gpx)?;
    Ok((path, points.len().min(u32::MAX as usize) as u32))
}

/// Where exports wait for a phone (Send to phone).
pub fn outbox_dir(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("outbox")
}

/// A new folder for one Send to phone batch to export into.
pub fn start_outbox_batch(data_dir: &Path) -> Result<std::path::PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dir = outbox_dir(data_dir).join(stamp.to_string());
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Puts what a batch exported into `batch` on the list phones fetch from.
/// Only folders inside the outbox are accepted, so a page cannot offer any
/// other file on the computer to a phone.
pub fn queue_outbox_batch(conn: &Connection, data_dir: &Path, batch: &Path) -> Result<u32> {
    let outbox = outbox_dir(data_dir).canonicalize()?;
    let batch = batch.canonicalize()?;
    if !batch.starts_with(&outbox) || batch == outbox {
        anyhow::bail!("not a Send to phone folder: {}", batch.display());
    }
    let mut queued = 0;
    let mut entries: Vec<_> = std::fs::read_dir(&batch)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let image = path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "tif" | "tiff"
            )
        });
        if !image || !path.is_file() {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let name = entry.file_name().to_string_lossy().into_owned();
        db::outbox_add(conn, &path.to_string_lossy(), &name, size as i64)?;
        queued += 1;
    }
    Ok(queued)
}

/// A phone has these: forget them and delete their files, and the batch
/// folders they leave empty.
pub fn outbox_delivered(conn: &mut Connection, ids: &[i64]) -> Result<u32> {
    let paths = db::outbox_take(conn, ids)?;
    for path in &paths {
        let path = Path::new(path);
        let _ = std::fs::remove_file(path);
        if let Some(parent) = path.parent() {
            // Fails, harmlessly, while the folder still holds files.
            let _ = std::fs::remove_dir(parent);
        }
    }
    Ok(paths.len() as u32)
}

/// Where photos added from a phone go (Add to library): the folder chosen in
/// Settings → Phone, else a `From phone` folder in the first library folder,
/// else one in `fallback` (Pictures/SourceLight) when there are none.
pub fn import_folder(conn: &Connection, fallback: &Path) -> Result<std::path::PathBuf> {
    if let Some(chosen) = db::get_setting(conn, IMPORT_FOLDER)?.filter(|s| !s.trim().is_empty()) {
        return Ok(chosen.into());
    }
    let folders = db::list_library_folders(conn)?;
    let base = match folders.folders.iter().min_by_key(|f| f.id) {
        Some(first) => std::path::PathBuf::from(&first.path),
        None => fallback.to_path_buf(),
    };
    Ok(base.join("From phone"))
}

/// Whether a folder was chosen for photos from a phone, as opposed to the
/// default.
pub fn import_folder_chosen(conn: &Connection) -> Result<bool> {
    Ok(db::get_setting(conn, IMPORT_FOLDER)?.is_some_and(|s| !s.trim().is_empty()))
}

/// Where photos added from a phone go, for Settings → Phone.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PhoneImportFolder {
    pub path: String,
    /// Chosen in Settings, rather than the default.
    pub chosen: bool,
}

/// Chooses where photos added from a phone go, or with `None` goes back to
/// the default.
pub fn set_import_folder(conn: &Connection, folder: Option<&str>) -> Result<()> {
    db::set_setting(conn, IMPORT_FOLDER, folder.unwrap_or(""))
}

/// A photo sent from a phone (Add to library), filed in `folder` and taken
/// into the library like any other: the scan reads it, makes its thumbnail
/// and preview, and its position finds a town. The phone's edits, if any,
/// become its edits here. Returns its id, and whether it was already in the
/// library: the same bytes sent again find the photo they became instead of
/// a second copy.
pub fn import_photo(
    conn: &mut Connection,
    folder: &Path,
    cache_dir: &Path,
    name: &str,
    bytes: &[u8],
    edits: Option<&str>,
) -> Result<(i64, bool)> {
    anyhow::ensure!(!bytes.is_empty(), "the phone sent an empty file");
    // Only what the scan reads, so a phone can't leave a program or a
    // shortcut in the library, nor a name Windows opens as a device.
    let name = file_name(name);
    anyhow::ensure!(
        crate::scan::is_photo_file(Path::new(&name)) && !is_device_name(&name),
        "the phone sent {name}, which isn't a photo SourceLight can add"
    );
    // One add at a time: the link serves requests concurrently, and two
    // phones adding the same bytes, or an IMG_0001 each, must not both find
    // them new or the name free.
    static ADDING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one_at_a_time = crate::sync::lock_recover(&ADDING);
    let sha = crate::hashing::sha256_hex(bytes);
    if let Some(id) = db::phone_import(conn, &sha)? {
        return Ok((id, true));
    }
    std::fs::create_dir_all(folder)?;
    // Never over a file that is there, however it got there.
    let file = loop {
        let file = unused_path(folder, &name);
        match crate::fsutil::write_new(&file, bytes) {
            Ok(()) => break file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    };

    // Filed under the library folder it is in, so it shows there; a folder
    // outside the library becomes a library folder of its own.
    let folder_str = folder.to_string_lossy().into_owned();
    let library = db::list_library_folders(conn)?
        .folders
        .into_iter()
        .filter(|f| folder.starts_with(&f.path))
        .max_by_key(|f| f.path.len());
    let library_id = match library {
        Some(f) => f.id,
        None => db::folder_for_scan(conn, &folder_str)?.0,
    };
    crate::scan::scan_library(conn, folder, cache_dir, |_, _, _| {})?;
    db::assign_folder(conn, library_id, &folder_str)?;
    db::place_from_gps(conn)?;

    let id = db::photo_id_for_path(conn, &file.to_string_lossy())?
        .ok_or_else(|| anyhow::anyhow!("the scan could not read {}", file.display()))?;
    if let Some(stack) = edits
        .and_then(|json| serde_json::from_str::<crate::develop::EditStack>(json).ok())
        .filter(|s| !s.is_default())
    {
        db::apply_stack_to(conn, &[id as i32], &stack)?;
    }
    db::phone_import_add(conn, &sha, id)?;
    Ok((id, false))
}

/// A file name from a phone, never a path: the last part, printable only.
fn file_name(sent: &str) -> String {
    let last = sent.rsplit(['/', '\\']).next().unwrap_or(sent);
    let clean: String = last
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect();
    match clean.trim() {
        "" | "." | ".." => "photo.jpg".into(),
        trimmed => trimmed.to_string(),
    }
}

/// Whether Windows takes `name` for a device whatever its extension:
/// `NUL.jpg` is the null device, `COM1.ARW` a serial port.
fn is_device_name(name: &str) -> bool {
    let base = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end()
        .to_ascii_uppercase();
    matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|port| {
        base.strip_prefix(port).is_some_and(|n| {
            let mut n = n.chars();
            matches!(n.next(), Some('0'..='9' | '¹' | '²' | '³')) && n.next().is_none()
        })
    })
}

/// `name` in `dir`, or `name (2)` and so on when that is taken: a phone's
/// IMG_0001 beside another's is kept, not replaced.
fn unused_path(dir: &Path, name: &str) -> std::path::PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let p = Path::new(name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let ext = p.extension().and_then(|e| e.to_str());
    (2..)
        .map(|n| match ext {
            Some(ext) => dir.join(format!("{stem} ({n}).{ext}")),
            None => dir.join(format!("{stem} ({n})")),
        })
        .find(|c| !c.exists())
        .expect("some number is free")
}

/// This catalogue's id, made on first use. A phone that sees a different one
/// than last time is looking at another library and starts its copy afresh.
pub fn catalogue_id(conn: &Connection) -> Result<String> {
    if let Some(id) = db::get_setting(conn, CATALOGUE_ID)? {
        return Ok(id);
    }
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no OS random source: {e}"))?;
    let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    db::set_setting(conn, CATALOGUE_ID, &id)?;
    Ok(id)
}

/// Applies decisions made on a phone: `(photo, flag, rating)`, a `None` left
/// alone. Copies are not on the phone, so only originals change. Returns how
/// many photos changed.
///
/// The phone's decision wins over the desktop's, in the order the phone made
/// them. Per-field change times, so a later decision on either side wins,
/// come with editing on both sides at once (docs/MOBILE.md, "What syncs").
pub fn apply_verdicts(
    conn: &mut Connection,
    verdicts: &[(i64, Option<i8>, Option<u8>)],
) -> Result<u32> {
    let tx = conn.transaction()?;
    let mut changed = 0;
    for &(id, flag, rating) in verdicts {
        let mut hit = false;
        if let Some(flag) = flag {
            hit |= tx.execute(
                "UPDATE photos SET flag = ?2 WHERE id = ?1 AND variant = 0",
                rusqlite::params![id, i64::from(flag.clamp(-1, 1))],
            )? > 0;
        }
        if let Some(rating) = rating {
            hit |= tx.execute(
                "UPDATE photos SET rating = ?2 WHERE id = ?1 AND variant = 0",
                rusqlite::params![id, i64::from(rating.min(5))],
            )? > 0;
        }
        changed += u32::from(hit);
    }
    tx.commit()?;
    Ok(changed)
}

/// Our own relays, once they run (docs/MOBILE.md, "At home and away"). While
/// this is empty, away-from-home uses n0's public relays: fine for
/// development, not for a release, as they are rate-limited and may change.
pub const RELAYS: &[&str] = &[];

/// A phone paired with this catalogue, for the list in Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PairedPhone {
    /// The phone's public key.
    pub id: String,
    pub name: String,
    /// "ios", "android" or "other".
    pub platform: String,
    /// SQLite `datetime('now')`, UTC.
    pub paired_at: String,
    pub last_seen: Option<String>,
}

/// Where the link stands, for Settings → Phone.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PhoneLinkStatus {
    /// False in a build without the phone link: Settings hides the tab.
    pub available: bool,
    pub enabled: bool,
    pub away_from_home: bool,
    /// Listening now.
    pub running: bool,
    /// Why the link is not running when it should be.
    pub error: Option<String>,
    pub phones: Vec<PairedPhone>,
}

/// A pairing code to show.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PhonePairing {
    /// The link the QR code holds, also shown for copying.
    pub uri: String,
    /// The QR code, as an SVG document: dark modules on white, as scanners
    /// expect, whatever the app's theme.
    pub qr_svg: String,
    pub expires_in_secs: u32,
    /// Whether the code carries a relay, so a phone can pair from elsewhere.
    pub reachable_away: bool,
}

/// The `phone-pair-request` event: a phone presented the code and waits for
/// the person at the desktop to allow it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PhonePairRequest {
    pub id: String,
    pub device_name: String,
    pub platform: String,
}

/// This catalogue's link key, made on first use.
pub fn secret_key(conn: &Connection) -> Result<[u8; 32]> {
    if let Some(stored) = db::get_setting(conn, SECRET_KEY)? {
        if let Ok(bytes) = STANDARD.decode(stored.trim()) {
            if let Ok(key) = <[u8; 32]>::try_from(bytes.as_slice()) {
                return Ok(key);
            }
        }
        // A damaged key cannot be repaired; a new one means pairing again,
        // which is what would have to happen anyway.
    }
    // Any 32 random bytes are a valid Ed25519 secret key.
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| anyhow::anyhow!("no OS random source: {e}"))?;
    db::set_setting(conn, SECRET_KEY, &STANDARD.encode(key))?;
    Ok(key)
}

fn flag(conn: &Connection, key: &str, default: bool) -> Result<bool> {
    Ok(match db::get_setting(conn, key)?.as_deref() {
        Some("1") => true,
        Some("0") => false,
        _ => default,
    })
}

pub fn enabled(conn: &Connection) -> Result<bool> {
    flag(conn, ENABLED, false)
}

/// On unless turned off: someone who pairs a phone expects it to work away
/// from home too, and the switch says what the relay can see.
pub fn away_from_home(conn: &Connection) -> Result<bool> {
    flag(conn, AWAY_FROM_HOME, true)
}

pub fn set_flags(conn: &Connection, enabled: bool, away_from_home: bool) -> Result<()> {
    db::set_setting(conn, ENABLED, if enabled { "1" } else { "0" })?;
    db::set_setting(conn, AWAY_FROM_HOME, if away_from_home { "1" } else { "0" })?;
    Ok(())
}

/// The name phones see for this computer.
pub fn computer_name() -> String {
    #[cfg(windows)]
    let name = std::env::var("COMPUTERNAME").ok();
    #[cfg(unix)]
    let name = {
        let mut buf = [0u8; 256];
        // SAFETY: the buffer is valid for its length, and gethostname writes
        // at most that many bytes.
        let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
        ok.then(|| {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..end])
                .trim_end_matches(".local")
                .to_string()
        })
    };
    #[cfg(not(any(windows, unix)))]
    let name: Option<String> = None;
    name.filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "SourceLight".to_string())
}

/// The QR code for a pairing link.
pub fn qr_svg(uri: &str) -> Result<String> {
    use qrcode::render::svg;
    use qrcode::{EcLevel, QrCode};
    // Medium error correction: survives glare on a screen without making the
    // code so dense that an older phone camera struggles.
    let code = QrCode::with_error_correction_level(uri.as_bytes(), EcLevel::M)?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(256, 256)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

/// The link's state while the app runs.
#[derive(Default)]
pub struct State {
    #[cfg(feature = "phone-link")]
    live: live::Live,
}

/// Where the link stands now.
pub fn status(state: &State, conn: &Connection) -> Result<PhoneLinkStatus> {
    let (running, error) = {
        #[cfg(feature = "phone-link")]
        {
            state.live.running()
        }
        #[cfg(not(feature = "phone-link"))]
        {
            let _ = state;
            (false, None)
        }
    };
    Ok(PhoneLinkStatus {
        available: cfg!(feature = "phone-link"),
        enabled: enabled(conn)?,
        away_from_home: away_from_home(conn)?,
        running,
        error,
        phones: db::paired_phones(conn)?,
    })
}

#[cfg(not(feature = "phone-link"))]
const UNAVAILABLE: &str = "This build was compiled without the phone link.";

/// Starts, stops or restarts the listener to match the settings. Called at
/// launch and whenever they change.
pub async fn apply(state: &State, app: &tauri::AppHandle, db_path: &Path) -> Result<(), String> {
    #[cfg(feature = "phone-link")]
    {
        state.live.apply(app, db_path).await
    }
    #[cfg(not(feature = "phone-link"))]
    {
        let _ = (state, app, db_path);
        Ok(())
    }
}

/// Turns the link on if it is not, and makes a new pairing code.
pub async fn start_pairing(
    state: &State,
    app: &tauri::AppHandle,
    db_path: &Path,
) -> Result<PhonePairing, String> {
    #[cfg(feature = "phone-link")]
    {
        state.live.start_pairing(app, db_path).await
    }
    #[cfg(not(feature = "phone-link"))]
    {
        let _ = (state, app, db_path);
        Err(UNAVAILABLE.to_string())
    }
}

/// Retires the pairing code on screen, and declines anyone still waiting.
pub async fn cancel_pairing(state: &State) {
    #[cfg(feature = "phone-link")]
    state.live.cancel_pairing().await;
    #[cfg(not(feature = "phone-link"))]
    let _ = state;
}

/// The person at the desktop's answer to a `phone-pair-request`.
pub fn answer_pairing(state: &State, phone: &str, allow: bool) {
    #[cfg(feature = "phone-link")]
    state.live.answer(phone, allow);
    #[cfg(not(feature = "phone-link"))]
    let _ = (state, phone, allow);
}

/// Forgets a phone. It is refused from its very next request.
pub fn remove_phone(state: &State, conn: &Connection, phone: &str) -> Result<()> {
    db::remove_paired_phone(conn, phone)?;
    #[cfg(feature = "phone-link")]
    state.live.forget(phone);
    #[cfg(not(feature = "phone-link"))]
    let _ = state;
    Ok(())
}

#[cfg(feature = "phone-link")]
mod live {
    use std::collections::{HashMap, HashSet};
    use std::future::Future;
    use std::path::{Path, PathBuf};
    use std::pin::Pin;
    use std::sync::{Arc, Mutex, RwLock};

    use sl_link::protocol::{ImageSize, OutboxEntry, PhotoRow, Refusal, Request, Response};
    use sl_link::{Approver, Desktop, DesktopConfig, PairCandidate, PeerStore, Relay, Service};
    use tauri::{Emitter, Manager};
    use tokio::sync::oneshot;

    use super::{PhonePairRequest, PhonePairing, PhoneTrack};
    use crate::db;
    use crate::sync::lock_recover;

    /// Phones waiting for an answer, by public key.
    type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>;

    #[derive(Default)]
    pub struct Live {
        desktop: tokio::sync::Mutex<Option<Running>>,
        pending: Pending,
        /// The last start's failure, shown in Settings.
        error: Mutex<Option<String>>,
        /// Mirrors `desktop.is_some()` for the synchronous status call.
        running: std::sync::atomic::AtomicBool,
        peers: Mutex<Option<Arc<CataloguePeers>>>,
    }

    struct Running {
        desktop: Desktop,
        relay: Relay,
    }

    fn relay(away: bool) -> Relay {
        if !away {
            Relay::Off
        } else if super::RELAYS.is_empty() {
            Relay::Public
        } else {
            Relay::Custom(super::RELAYS.iter().map(|r| r.to_string()).collect())
        }
    }

    impl Live {
        pub fn running(&self) -> (bool, Option<String>) {
            (
                self.running.load(std::sync::atomic::Ordering::Relaxed),
                lock_recover(&self.error).clone(),
            )
        }

        pub async fn apply(&self, app: &tauri::AppHandle, db_path: &Path) -> Result<(), String> {
            let (on, away) = {
                let conn = db::connect(db_path).map_err(|e| e.to_string())?;
                (
                    super::enabled(&conn).map_err(|e| e.to_string())?,
                    super::away_from_home(&conn).map_err(|e| e.to_string())?,
                )
            };
            let mut slot = self.desktop.lock().await;
            let wanted = on.then(|| relay(away));
            if slot.as_ref().map(|r| &r.relay) == wanted.as_ref() {
                return Ok(());
            }
            if let Some(old) = slot.take() {
                old.desktop.shutdown().await;
            }
            self.running
                .store(false, std::sync::atomic::Ordering::Relaxed);
            let Some(relay) = wanted else {
                *lock_recover(&self.error) = None;
                return Ok(());
            };
            match self.start(app, db_path, relay.clone()).await {
                Ok(desktop) => {
                    *slot = Some(Running { desktop, relay });
                    self.running
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    *lock_recover(&self.error) = None;
                    Ok(())
                }
                Err(e) => {
                    *lock_recover(&self.error) = Some(e.clone());
                    Err(e)
                }
            }
        }

        async fn start(
            &self,
            app: &tauri::AppHandle,
            db_path: &Path,
            relay: Relay,
        ) -> Result<Desktop, String> {
            let (secret_key, phones) = {
                let conn = db::connect(db_path).map_err(|e| e.to_string())?;
                let key = super::secret_key(&conn).map_err(|e| e.to_string())?;
                let phones = db::paired_phones(&conn).map_err(|e| e.to_string())?;
                (key, phones)
            };
            let peers = Arc::new(CataloguePeers {
                db_path: db_path.to_path_buf(),
                paired: RwLock::new(phones.into_iter().map(|p| p.id).collect()),
                app: app.clone(),
            });
            *lock_recover(&self.peers) = Some(peers.clone());
            Desktop::start(DesktopConfig {
                secret_key,
                relay,
                name: super::computer_name(),
                peers,
                approver: Arc::new(AskInWindow {
                    app: app.clone(),
                    pending: self.pending.clone(),
                }),
                service: Arc::new(Library {
                    db_path: db_path.to_path_buf(),
                    app: app.clone(),
                }),
                offer_ttl: sl_link::OFFER_TTL,
            })
            .await
            .map_err(|e| e.to_string())
        }

        pub async fn start_pairing(
            &self,
            app: &tauri::AppHandle,
            db_path: &Path,
        ) -> Result<PhonePairing, String> {
            {
                let conn = db::connect(db_path).map_err(|e| e.to_string())?;
                if !super::enabled(&conn).map_err(|e| e.to_string())? {
                    let away = super::away_from_home(&conn).map_err(|e| e.to_string())?;
                    super::set_flags(&conn, true, away).map_err(|e| e.to_string())?;
                }
            }
            self.apply(app, db_path).await?;
            let slot = self.desktop.lock().await;
            let running = slot
                .as_ref()
                .ok_or("The phone link is not running.".to_string())?;
            let offer = running.desktop.offer().await;
            let uri = offer.to_uri();
            Ok(PhonePairing {
                qr_svg: super::qr_svg(&uri).map_err(|e| e.to_string())?,
                uri,
                expires_in_secs: sl_link::OFFER_TTL.as_secs() as u32,
                reachable_away: offer.relay.is_some(),
            })
        }

        pub async fn cancel_pairing(&self) {
            if let Some(running) = self.desktop.lock().await.as_ref() {
                running.desktop.cancel_offer();
            }
            // Dropping the senders answers "no" to anyone still waiting.
            lock_recover(&self.pending).clear();
        }

        pub fn answer(&self, phone: &str, allow: bool) {
            if let Some(tx) = lock_recover(&self.pending).remove(phone) {
                let _ = tx.send(allow);
            }
        }

        pub fn forget(&self, phone: &str) {
            if let Some(peers) = lock_recover(&self.peers).as_ref() {
                peers
                    .paired
                    .write()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(phone);
            }
        }
    }

    /// The paired phones, kept in the catalogue and mirrored in memory: the
    /// link asks on every request, and a thumbnail fetch is many requests.
    struct CataloguePeers {
        db_path: PathBuf,
        paired: RwLock<HashSet<String>>,
        app: tauri::AppHandle,
    }

    impl PeerStore for CataloguePeers {
        fn is_paired(&self, phone: &str) -> bool {
            self.paired
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .contains(phone)
        }

        fn add(&self, phone: &PairCandidate) -> Result<(), String> {
            let conn = db::connect(&self.db_path).map_err(|e| e.to_string())?;
            db::add_paired_phone(
                &conn,
                &phone.id,
                &phone.device_name,
                platform_name(phone.platform),
            )
            .map_err(|e| e.to_string())?;
            self.paired
                .write()
                .unwrap_or_else(|p| p.into_inner())
                .insert(phone.id.clone());
            let _ = self.app.emit("phone-link-changed", ());
            Ok(())
        }

        fn seen(&self, phone: &str) {
            if let Ok(conn) = db::connect(&self.db_path) {
                let _ = db::paired_phone_seen(&conn, phone);
            }
            let _ = self.app.emit("phone-link-changed", ());
        }
    }

    fn platform_name(p: sl_link::protocol::Platform) -> &'static str {
        use sl_link::protocol::Platform;
        match p {
            Platform::Ios => "ios",
            Platform::Android => "android",
            Platform::Other => "other",
        }
    }

    /// A phone's library requests: the change feed, images, and its
    /// decisions. Answered from the catalogue on a blocking thread, as SQLite
    /// and file reads block.
    struct Library {
        db_path: PathBuf,
        app: tauri::AppHandle,
    }

    impl Service for Library {
        fn handle(
            &self,
            _phone: String,
            request: Request,
            body: Vec<u8>,
        ) -> Pin<Box<dyn Future<Output = (Response, Vec<u8>)> + Send>> {
            let db_path = self.db_path.clone();
            let app = self.app.clone();
            Box::pin(async move {
                tokio::task::spawn_blocking(move || answer(&db_path, &app, request, body))
                    .await
                    .unwrap_or_else(|_| refuse(Refusal::Failed))
            })
        }
    }

    fn refuse(reason: Refusal) -> (Response, Vec<u8>) {
        (Response::Refused { reason }, Vec::new())
    }

    fn answer(
        db_path: &Path,
        app: &tauri::AppHandle,
        request: Request,
        body: Vec<u8>,
    ) -> (Response, Vec<u8>) {
        let Ok(mut conn) = db::connect(db_path) else {
            return refuse(Refusal::Failed);
        };
        match request {
            Request::Changes { since, limit } => {
                let (Ok(catalogue), Ok(page)) = (
                    super::catalogue_id(&conn),
                    db::changes_since(&conn, since, limit),
                ) else {
                    return refuse(Refusal::Failed);
                };
                let rows = page
                    .rows
                    .into_iter()
                    .map(|r| PhotoRow {
                        id: r.id,
                        filename: r.filename,
                        captured: r.captured,
                        width: r.width,
                        height: r.height,
                        rating: r.rating,
                        flag: r.flag,
                        edit_tag: r.edit_tag,
                        edit_on: protocol_edit_on(r.edit_on),
                    })
                    .collect();
                let response = Response::Changes {
                    catalogue,
                    head: page.head,
                    rows,
                    removed: page.removed,
                    next: page.next,
                    more: page.more,
                };
                (response, Vec::new())
            }
            Request::Image {
                id,
                size: ImageSize::Edited,
            } => match super::photo_edits(&conn, id) {
                Ok(Some((Some(settings), _))) => {
                    let tag = super::edit_tag(Some(&settings)).unwrap_or_default();
                    let state = app.state::<crate::AppState>();
                    match crate::commands::export::render_edited(app, &state, id as i32, &tag)
                        .and_then(|file| std::fs::read(file).map_err(|e| e.to_string()))
                    {
                        Ok(jpeg) => (Response::Image, jpeg),
                        Err(e) => {
                            eprintln!("phone link: photo {id} as edited: {e}");
                            refuse(Refusal::Failed)
                        }
                    }
                }
                // Not edited: the preview is the photo as it is.
                Ok(Some((None, _))) => match db::photo_image_path(&conn, id, true) {
                    Ok(Some(path)) => match std::fs::read(&path) {
                        Ok(jpeg) => (Response::Image, jpeg),
                        Err(_) => refuse(Refusal::NotFound),
                    },
                    Ok(None) => refuse(Refusal::NotFound),
                    Err(_) => refuse(Refusal::Failed),
                },
                Ok(None) => refuse(Refusal::NotFound),
                Err(_) => refuse(Refusal::Failed),
            },
            Request::Image { id, size } => {
                let preview = size == ImageSize::Preview;
                match db::photo_image_path(&conn, id, preview) {
                    Ok(Some(path)) => match std::fs::read(&path) {
                        Ok(jpeg) => (Response::Image, jpeg),
                        Err(_) => refuse(Refusal::NotFound),
                    },
                    Ok(None) => refuse(Refusal::NotFound),
                    Err(_) => refuse(Refusal::Failed),
                }
            }
            Request::Verdicts { verdicts } => {
                let list: Vec<_> = verdicts.iter().map(|v| (v.id, v.flag, v.rating)).collect();
                match super::apply_verdicts(&mut conn, &list) {
                    Ok(applied) => {
                        if applied > 0 {
                            // Undo on the desktop stops here: a step from
                            // before would write its old values over the
                            // phone's newer decisions.
                            let _ = crate::commands::common::beyond_undo(
                                &app.state::<crate::AppState>(),
                                "Picks and ratings from a phone",
                                applied as usize,
                                Ok(()),
                            );
                            // The desktop's grid shows the phone's picks
                            // without waiting for anything else to reload it.
                            let _ = app.emit("phone-changes", applied);
                        }
                        (Response::Applied { applied }, Vec::new())
                    }
                    Err(_) => refuse(Refusal::Failed),
                }
            }
            Request::Track {
                name, started_ms, ..
            } => {
                let data_dir = db_path.parent().unwrap_or(db_path);
                match super::save_phone_track(data_dir, started_ms, &body) {
                    Ok((path, points)) => {
                        let _ = app.emit(
                            "phone-track",
                            PhoneTrack {
                                path: path.display().to_string(),
                                name,
                                points,
                            },
                        );
                        (Response::TrackSaved { points }, Vec::new())
                    }
                    Err(_) => refuse(Refusal::BadRequest),
                }
            }
            Request::Outbox => match db::outbox(&conn) {
                Ok(rows) => {
                    let files = rows
                        .into_iter()
                        .map(|(id, name, size, _)| OutboxEntry {
                            id,
                            name,
                            size: size.max(0) as u64,
                        })
                        .collect();
                    (Response::Outbox { files }, Vec::new())
                }
                Err(_) => refuse(Refusal::Failed),
            },
            Request::OutboxFile { id } => {
                let path = db::outbox(&conn)
                    .ok()
                    .and_then(|rows| rows.into_iter().find(|r| r.0 == id).map(|r| r.3));
                match path.map(std::fs::read) {
                    Some(Ok(bytes)) => (Response::File, bytes),
                    _ => refuse(Refusal::NotFound),
                }
            }
            Request::Received { ids } => match super::outbox_delivered(&mut conn, &ids) {
                Ok(applied) => (Response::Applied { applied }, Vec::new()),
                Err(_) => refuse(Refusal::Failed),
            },
            Request::AddPhoto { name, edits, .. } => {
                if body.is_empty() {
                    return refuse(Refusal::BadRequest);
                }
                let data_dir = db_path.parent().unwrap_or(db_path);
                // The app's own thumbnail folder (`AppState::cache_dir`).
                let cache_dir = data_dir.join("thumbs");
                let fallback = app
                    .path()
                    .picture_dir()
                    .map(|p| p.join("SourceLight"))
                    .unwrap_or_else(|_| data_dir.to_path_buf());
                let added = super::import_folder(&conn, &fallback).and_then(|folder| {
                    super::import_photo(
                        &mut conn,
                        &folder,
                        &cache_dir,
                        &name,
                        &body,
                        edits.as_deref(),
                    )
                });
                match added {
                    Ok((id, existed)) => {
                        if !existed {
                            app.state::<crate::AppState>().bump_search_gen();
                            let _ = app.emit("phone-changes", 1);
                        }
                        (Response::Added { id, existed }, Vec::new())
                    }
                    Err(e) => {
                        eprintln!("phone link: could not add {name}: {e:#}");
                        refuse(Refusal::Failed)
                    }
                }
            }
            Request::Vocabulary => match db::search_vocabulary(&conn)
                .map_err(|e| e.to_string())
                .and_then(|v| serde_json::to_string(&v).map_err(|e| e.to_string()))
            {
                Ok(json) => (Response::Vocabulary { json }, Vec::new()),
                Err(_) => refuse(Refusal::Failed),
            },
            Request::Search { filter } => {
                let Ok(filter) = serde_json::from_str::<crate::models::PhotoFilter>(&filter) else {
                    return refuse(Refusal::BadRequest);
                };
                let state = app.state::<crate::AppState>();
                match crate::commands::phone::phone_search(app, &state, &filter) {
                    Ok((ids, ranked)) => (Response::Found { ids, ranked }, Vec::new()),
                    Err(e) => {
                        eprintln!("phone link: search: {e}");
                        refuse(Refusal::Failed)
                    }
                }
            }
            Request::Edits { id } => match super::photo_edits(&conn, id) {
                Ok(Some((edits, on))) => {
                    let tag = super::edit_tag(edits.as_deref());
                    let edit_on = protocol_edit_on(on);
                    (
                        Response::Edits {
                            edits,
                            tag,
                            edit_on,
                        },
                        Vec::new(),
                    )
                }
                Ok(None) => refuse(Refusal::NotFound),
                Err(_) => refuse(Refusal::Failed),
            },
            Request::SetEdits { id, edits, base } => {
                let edit = match super::phone_edit(&conn, id, edits.as_deref(), base.as_deref()) {
                    Ok(Some(edit)) => edit,
                    Ok(None) => return refuse(Refusal::NotFound),
                    Err(e) => {
                        eprintln!("phone link: edits to photo {id} refused: {e:#}");
                        return refuse(Refusal::BadRequest);
                    }
                };
                let state = app.state::<crate::AppState>();
                let kept = match through_develop(&state, id, &edit) {
                    Some(info) => {
                        db::set_develop_settings(&conn, info.photo_id, &info.stack).map(|()| {
                            let _ = app.emit("develop-updated", info);
                            None
                        })
                    }
                    None => super::keep_phone_edit(&mut conn, id, &edit),
                };
                match kept {
                    Ok(copy) => {
                        if copy.is_some() {
                            state.bump_search_gen();
                        }
                        let _ = app.emit("phone-changes", 1);
                        let tag = super::photo_edits(&conn, copy.unwrap_or(id))
                            .ok()
                            .flatten()
                            .and_then(|(edits, _)| super::edit_tag(edits.as_deref()));
                        (Response::EditsSaved { tag, copy }, Vec::new())
                    }
                    Err(e) => {
                        eprintln!("phone link: could not keep edits to photo {id}: {e:#}");
                        refuse(Refusal::Failed)
                    }
                }
            }
            // Pairing, hello and ping never reach here: the link answers them.
            _ => refuse(Refusal::BadRequest),
        }
    }

    /// A phone's edit to the photo open in Develop goes through Develop: it
    /// is a step its undo can take back, and the page shows it. `None` when
    /// that photo isn't open, or the edit goes on a copy.
    fn through_develop(
        state: &crate::AppState,
        id: i64,
        edit: &super::PhoneEdit,
    ) -> Option<crate::develop::DevelopInfo> {
        let super::PhoneEdit::Photo(stack) = edit else {
            return None;
        };
        let mut guard = state.develop.lock().ok()?;
        let dev = guard
            .as_mut()
            .filter(|d| d.open_photo_id() == Some(id as i32))?;
        match dev.apply(super::EDITED_ON_PHONE, stack.clone()) {
            Ok(info) => Some(info),
            Err(e) => {
                eprintln!("phone link: Develop could not take the phone's edit: {e:#}");
                None
            }
        }
    }

    fn protocol_edit_on(on: super::EditOn) -> sl_link::protocol::EditOn {
        use sl_link::protocol::EditOn;
        match on {
            super::EditOn::Phone => EditOn::Phone,
            super::EditOn::Raw => EditOn::Raw,
            super::EditOn::Computer => EditOn::Computer,
        }
    }

    /// Asks in the window: emits `phone-pair-request` and waits for
    /// `answer_phone_pairing`.
    struct AskInWindow {
        app: tauri::AppHandle,
        pending: Pending,
    }

    impl Approver for AskInWindow {
        fn approve(&self, phone: PairCandidate) -> Pin<Box<dyn Future<Output = bool> + Send>> {
            let (tx, rx) = oneshot::channel();
            lock_recover(&self.pending).insert(phone.id.clone(), tx);
            let _ = self.app.emit(
                "phone-pair-request",
                PhonePairRequest {
                    id: phone.id,
                    device_name: phone.device_name,
                    platform: platform_name(phone.platform).to_string(),
                },
            );
            // A dropped sender (the dialog closed, the code was cancelled) is
            // a "no".
            Box::pin(async move { rx.await.unwrap_or(false) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalogue() -> Connection {
        db::open_memory().unwrap()
    }

    #[test]
    fn the_key_is_made_once_and_kept() {
        let conn = catalogue();
        let first = secret_key(&conn).unwrap();
        assert_eq!(secret_key(&conn).unwrap(), first);
        assert_ne!(first, [0; 32]);
    }

    #[test]
    fn a_damaged_key_is_replaced() {
        let conn = catalogue();
        db::set_setting(&conn, SECRET_KEY, "not base64!").unwrap();
        let key = secret_key(&conn).unwrap();
        assert_eq!(secret_key(&conn).unwrap(), key);
    }

    #[test]
    fn the_link_starts_off_with_away_from_home_on() {
        let conn = catalogue();
        assert!(!enabled(&conn).unwrap());
        assert!(away_from_home(&conn).unwrap());
        set_flags(&conn, true, false).unwrap();
        assert!(enabled(&conn).unwrap());
        assert!(!away_from_home(&conn).unwrap());
    }

    #[test]
    fn phones_are_listed_seen_and_removed() {
        let conn = catalogue();
        let state = State::default();
        db::add_paired_phone(&conn, "key-a", "James's iPhone", "ios").unwrap();
        db::add_paired_phone(&conn, "key-b", "Pixel", "android").unwrap();
        let phones = status(&state, &conn).unwrap().phones;
        assert_eq!(phones.len(), 2);
        assert!(phones.iter().all(|p| p.last_seen.is_none()));

        db::paired_phone_seen(&conn, "key-b").unwrap();
        let phones = db::paired_phones(&conn).unwrap();
        let pixel = phones.iter().find(|p| p.id == "key-b").unwrap();
        assert!(pixel.last_seen.is_some());

        // Pairing the same phone again renames it rather than adding a row.
        db::add_paired_phone(&conn, "key-a", "Work iPhone", "ios").unwrap();
        let phones = db::paired_phones(&conn).unwrap();
        assert_eq!(phones.len(), 2);
        assert!(phones.iter().any(|p| p.name == "Work iPhone"));

        remove_phone(&state, &conn, "key-a").unwrap();
        let phones = db::paired_phones(&conn).unwrap();
        assert_eq!(phones.len(), 1);
        assert_eq!(phones[0].id, "key-b");
    }

    fn photo(conn: &Connection, id: i64, name: &str) {
        conn.execute(
            "INSERT INTO photos (id, path, filename, file_hash, capture_date, width, height,
                                 orientation, indexed_at)
             VALUES (?1, ?2, ?3, 'h', '2026-09-18 10:00:00', 6000, 4000, 1, 'now')",
            rusqlite::params![id, format!("C:/p/{name}"), name],
        )
        .unwrap();
    }

    #[test]
    fn the_catalogue_id_is_made_once() {
        let conn = catalogue();
        let id = catalogue_id(&conn).unwrap();
        assert_eq!(id.len(), 32);
        assert_eq!(catalogue_id(&conn).unwrap(), id);
    }

    /// The change feed (migration 49): a first sync gets everything, later
    /// ones only what changed, and deletions arrive as removals.
    #[test]
    fn the_change_feed_pages_changes_and_removals() {
        let conn = catalogue();
        for i in 1..=5 {
            photo(&conn, i, &format!("{i}.ARW"));
        }
        let first = db::changes_since(&conn, -1, 3).unwrap();
        assert_eq!(first.rows.len(), 3);
        assert!(first.more);
        let rest = db::changes_since(&conn, first.next, 3).unwrap();
        assert_eq!(rest.rows.len(), 2);
        assert!(!rest.more);
        assert_eq!(rest.next, rest.head);
        let cursor = rest.next;

        // Nothing changed: nothing to send.
        let none = db::changes_since(&conn, cursor, 100).unwrap();
        assert!(none.rows.is_empty() && none.removed.is_empty());

        // A flag set on the desktop, a photo deleted, one turned a quarter.
        db::set_flag(&conn, 2, 1).unwrap();
        conn.execute("DELETE FROM photos WHERE id = 3", []).unwrap();
        conn.execute("UPDATE photos SET orientation = 6 WHERE id = 4", [])
            .unwrap();
        // A column the phone does not keep changes nothing for it.
        conn.execute("UPDATE photos SET caption = 'x' WHERE id = 5", [])
            .unwrap();
        let later = db::changes_since(&conn, cursor, 100).unwrap();
        let ids: Vec<i64> = later.rows.iter().map(|r| r.id).collect();
        assert_eq!(ids, [2, 4]);
        assert_eq!(later.rows[0].flag, 1);
        assert_eq!((later.rows[1].width, later.rows[1].height), (4000, 6000));
        assert_eq!(later.removed, [3]);
    }

    fn set_edits(conn: &Connection, id: i64, json: Option<&str>) {
        conn.execute(
            "UPDATE photos SET develop_settings = ?2 WHERE id = ?1",
            rusqlite::params![id, json],
        )
        .unwrap();
    }

    /// Edits move a photo up the feed (migration 52), with a tag that changes
    /// when they do, and where it can be edited.
    #[test]
    fn edits_reach_the_feed_with_where_to_edit() {
        let conn = catalogue();
        photo(&conn, 1, "a.jpg");
        photo(&conn, 2, "b.ARW");
        let first = db::changes_since(&conn, -1, 100).unwrap();
        assert_eq!(first.rows[0].edit_on, EditOn::Phone);
        assert_eq!(first.rows[0].edit_tag, None);
        assert_eq!(first.rows[1].edit_on, EditOn::Raw);
        let cursor = first.next;

        set_edits(&conn, 1, Some(r#"{"exposure":0.5}"#));
        let page = db::changes_since(&conn, cursor, 100).unwrap();
        assert_eq!(page.rows.len(), 1);
        let tag = page.rows[0].edit_tag.clone().unwrap();
        assert_eq!(tag.len(), 16);
        // The same edits saved again tell a phone nothing.
        set_edits(&conn, 1, Some(r#"{"exposure":0.5}"#));
        assert!(db::changes_since(&conn, page.next, 100)
            .unwrap()
            .rows
            .is_empty());

        // Erasing needs the computer's fill files.
        set_edits(
            &conn,
            1,
            Some(r#"{"erases":[{"label":"Bin","pieces":[{"fill":"f"}]}]}"#),
        );
        let page = db::changes_since(&conn, page.next, 100).unwrap();
        assert_ne!(page.rows[0].edit_tag.as_deref(), Some(tag.as_str()));
        assert_eq!(
            edit_on(
                "C:/p/a.jpg",
                Some(r#"{"lut":{"name":"x.cube","intensity":1}}"#)
            ),
            EditOn::Computer
        );
        let film = &crate::develop::lut_packs::catalogue()[0].file;
        let built_in = format!(r#"{{"lut":{{"name":"{film}","intensity":0.8}}}}"#);
        assert_eq!(edit_on("C:/p/a.jpg", Some(&built_in)), EditOn::Phone);
        assert_eq!(
            edit_on("C:/p/a.jpg", Some(r#"{"detail":{"aiDenoise":0.5}}"#)),
            EditOn::Computer
        );
        assert_eq!(edit_on("C:/p/a.jpg", Some("not json")), EditOn::Computer);
        assert_eq!(
            edit_on("C:/p/a.jpg", Some(r#"{"exposure":1}"#)),
            EditOn::Phone
        );
    }

    /// A phone's edit goes on the photo when nobody changed it meanwhile, and
    /// on a virtual copy when the computer did, leaving the computer's edit.
    #[test]
    fn a_phone_edit_never_overwrites_a_newer_one() {
        let mut conn = catalogue();
        photo(&conn, 1, "a.jpg");
        let warm = r#"{"warmth":0.3}"#;

        // Unedited, and still unedited: on the photo.
        let edit = phone_edit(&conn, 1, Some(warm), None).unwrap().unwrap();
        assert!(matches!(edit, PhoneEdit::Photo(ref s) if s.warmth == 0.3));
        assert_eq!(keep_phone_edit(&mut conn, 1, &edit).unwrap(), None);
        let (stored, _) = photo_edits(&conn, 1).unwrap().unwrap();
        let stored = stored.unwrap();

        // Then edited on the computer while the phone edited from `stored`.
        set_edits(&conn, 1, Some(r#"{"contrast":0.4}"#));
        let edit = phone_edit(&conn, 1, Some(r#"{"warmth":-0.2}"#), Some(&stored))
            .unwrap()
            .unwrap();
        let copy = keep_phone_edit(&mut conn, 1, &edit).unwrap().unwrap();
        let original = db::get_develop_settings(&conn, 1).unwrap().unwrap();
        assert_eq!((original.contrast, original.warmth), (0.4, 0.0));
        let theirs = db::get_develop_settings(&conn, copy as i32)
            .unwrap()
            .unwrap();
        assert_eq!(theirs.warmth, -0.2);
        let name: Option<String> = conn
            .query_row(
                "SELECT variant_name FROM photos WHERE id = ?1",
                [copy],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(name.as_deref(), Some(FROM_PHONE));

        // The phone carries on editing on the copy, from what it sent: no
        // second copy.
        let (sent, _) = photo_edits(&conn, copy).unwrap().unwrap();
        let edit = phone_edit(&conn, copy, Some(r#"{"warmth":-0.4}"#), sent.as_deref())
            .unwrap()
            .unwrap();
        assert!(matches!(edit, PhoneEdit::Photo(_)));
        assert_eq!(keep_phone_edit(&mut conn, copy, &edit).unwrap(), None);

        // Taking every edit off leaves the photo unedited.
        let (now, _) = photo_edits(&conn, 1).unwrap().unwrap();
        let edit = phone_edit(&conn, 1, None, now.as_deref()).unwrap().unwrap();
        keep_phone_edit(&mut conn, 1, &edit).unwrap();
        assert_eq!(db::get_develop_settings(&conn, 1).unwrap(), None);
    }

    /// A setting the phone's app doesn't know survives its edit.
    #[test]
    fn a_phone_keeps_what_it_does_not_know() {
        let merged = laid_over(
            Some(r#"{"exposure":0.5,"detail":{"clarity":0.2,"futureDial":3},"futureField":[1]}"#),
            Some(r#"{"exposure":1.0,"detail":{"clarity":0.4}}"#),
        )
        .unwrap();
        assert_eq!(merged.exposure, 1.0);
        assert_eq!(merged.detail.clarity, 0.4);
        let mut base = serde_json::json!({"a": 1, "o": {"x": 1, "y": 2}});
        lay_over(
            &mut base,
            serde_json::json!({"a": 2, "o": {"y": 3}, "n": null}),
        );
        assert_eq!(
            base,
            serde_json::json!({"a": 2, "o": {"x": 1, "y": 3}, "n": null})
        );
    }

    #[test]
    fn phones_cannot_edit_raws_or_unseen_photos() {
        let conn = catalogue();
        photo(&conn, 1, "a.ARW");
        assert!(phone_edit(&conn, 1, Some("{}"), None).is_err());
        assert_eq!(phone_edit(&conn, 9, Some("{}"), None).unwrap(), None);
    }

    #[test]
    fn missing_photos_and_copies_are_removals() {
        let conn = catalogue();
        photo(&conn, 1, "a.ARW");
        photo(&conn, 2, "b.ARW");
        let cursor = db::changes_since(&conn, -1, 100).unwrap().next;
        conn.execute("UPDATE photos SET missing = 1 WHERE id = 1", [])
            .unwrap();
        let page = db::changes_since(&conn, cursor, 100).unwrap();
        assert_eq!(page.removed, [1]);
        assert!(page.rows.is_empty());
    }

    #[test]
    fn verdicts_from_a_phone_change_originals_only() {
        let mut conn = catalogue();
        photo(&conn, 1, "a.ARW");
        photo(&conn, 2, "b.ARW");
        let applied = apply_verdicts(
            &mut conn,
            &[(1, Some(-1), None), (2, None, Some(9)), (99, Some(1), None)],
        )
        .unwrap();
        assert_eq!(applied, 2);
        let (flag, rating): (i64, i64) = conn
            .query_row("SELECT flag, rating FROM photos WHERE id = 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((flag, rating), (-1, 0));
        let rating2: i64 = conn
            .query_row("SELECT rating FROM photos WHERE id = 2", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rating2, 5, "clamped to five stars");
    }

    /// What the phone writes is what the desktop's track reader reads: every
    /// point, at the right time.
    #[test]
    fn a_phone_track_is_kept_and_reads_back() {
        use sl_protocol::track::{parse_recording, to_gpx};
        let csv = "1791019815000,46.5,11.9,4.0,1200.5\n1791019825000,46.51,11.91,4.0,\n";
        let gpx = to_gpx(&parse_recording(csv), "Dolomites");
        let dir = std::env::temp_dir().join(format!("sl-track-{}", std::process::id()));
        let (path, points) = save_phone_track(&dir, 1_791_019_815_000, gpx.as_bytes()).unwrap();
        assert_eq!(points, 2);
        let read = crate::gpx::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(read[0].0, 1_791_019_815);
        assert!((read[1].1 - 46.51).abs() < 1e-6);
        // A file with no timed points is not kept.
        assert!(save_phone_track(&dir, 1, b"<gpx></gpx>").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Send to phone: a batch's files go on the list, a phone fetches them,
    /// and once it has them they and their folder are gone.
    #[test]
    fn the_outbox_queues_and_lets_go() {
        let mut conn = catalogue();
        let data = std::env::temp_dir().join(format!("sl-outbox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let batch = start_outbox_batch(&data).unwrap();
        std::fs::write(batch.join("b.jpg"), b"bb").unwrap();
        std::fs::write(batch.join("a.jpg"), b"a").unwrap();
        std::fs::write(batch.join("notes.txt"), b"not an image").unwrap();
        assert_eq!(queue_outbox_batch(&conn, &data, &batch).unwrap(), 2);
        let rows = db::outbox(&conn).unwrap();
        let names: Vec<&str> = rows.iter().map(|r| r.1.as_str()).collect();
        assert_eq!(names, ["a.jpg", "b.jpg"]);
        assert_eq!(rows[1].2, 2);

        let ids: Vec<i64> = rows.iter().map(|r| r.0).collect();
        assert_eq!(outbox_delivered(&mut conn, &ids).unwrap(), 2);
        assert!(db::outbox(&conn).unwrap().is_empty());
        assert!(!batch.join("a.jpg").exists());
        // Delivering again is a no-op.
        assert_eq!(outbox_delivered(&mut conn, &ids).unwrap(), 0);
        let _ = std::fs::remove_dir_all(data);
    }

    /// Only Send to phone's own folders can be offered to a phone.
    #[test]
    fn only_outbox_folders_are_queued() {
        let conn = catalogue();
        let data = std::env::temp_dir().join(format!("sl-outbox-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(outbox_dir(&data)).unwrap();
        let elsewhere = data.join("Pictures");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("private.jpg"), b"x").unwrap();
        assert!(queue_outbox_batch(&conn, &data, &elsewhere).is_err());
        assert!(queue_outbox_batch(&conn, &data, &outbox_dir(&data)).is_err());
        let sneaky = outbox_dir(&data).join("..").join("Pictures");
        assert!(queue_outbox_batch(&conn, &data, &sneaky).is_err());
        assert!(db::outbox(&conn).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(data);
    }

    /// A JPEG to send, 32 px square of one colour.
    fn phone_jpeg(rgb: [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(32, 32, image::Rgb(rgb));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        out.into_inner()
    }

    /// Add to library: a phone's photo is filed in the import folder, taken
    /// into the library under the folder it is in, with its edits, and the
    /// same file sent again finds it rather than making a copy.
    #[test]
    fn a_phone_photo_joins_the_library_once() {
        let mut conn = catalogue();
        let root = std::env::temp_dir().join(format!("sl-add-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let library = root.join("Photos");
        let cache = root.join("thumbs");
        std::fs::create_dir_all(&library).unwrap();
        let (library_id, _) = db::folder_for_scan(&mut conn, &library.to_string_lossy()).unwrap();
        let folder = import_folder(&conn, &root.join("fallback")).unwrap();
        assert!(folder.starts_with(&library), "{}", folder.display());

        let red = phone_jpeg([200, 30, 30]);
        let (id, existed) = import_photo(
            &mut conn,
            &folder,
            &cache,
            "IMG_0001.jpg",
            &red,
            Some(r#"{"exposure":0.5}"#),
        )
        .unwrap();
        assert!(!existed);
        let (path, folder_id, settings): (String, i32, Option<String>) = conn
            .query_row(
                "SELECT path, folder_id, develop_settings FROM photos WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(Path::new(&path), folder.join("IMG_0001.jpg"));
        assert_eq!(folder_id, library_id);
        assert!(settings.unwrap().contains("\"exposure\":0.5"));

        // The same bytes again: the same photo, no second file.
        let again = import_photo(&mut conn, &folder, &cache, "IMG_0001.jpg", &red, None).unwrap();
        assert_eq!(again, (id, true));
        assert!(!folder.join("IMG_0001 (2).jpg").exists());

        // Another photo with the same name is kept beside it.
        let blue = phone_jpeg([30, 30, 200]);
        let (other, _) =
            import_photo(&mut conn, &folder, &cache, "IMG_0001.jpg", &blue, None).unwrap();
        assert_ne!(other, id);
        assert!(folder.join("IMG_0001 (2).jpg").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    /// Whatever name a phone sends, the file lands in the import folder.
    #[test]
    fn a_phone_names_a_file_never_a_path() {
        assert_eq!(file_name("IMG_0001.HEIC"), "IMG_0001.HEIC");
        assert_eq!(file_name("../../Windows/evil.jpg"), "evil.jpg");
        assert_eq!(file_name("C:\\Users\\x.jpg"), "x.jpg");
        assert_eq!(file_name("a:b*c?.jpg"), "abc.jpg");
        assert_eq!(file_name(".."), "photo.jpg");
    }

    /// Only a photo the scan reads is added, and never under a device name.
    #[test]
    fn a_phone_adds_photos_only() {
        for device in [
            "NUL.jpg",
            "con.ARW",
            "Com1.jpg",
            "LPT9.dng",
            "COM¹.jpg",
            "aux .jpg",
        ] {
            assert!(is_device_name(device), "{device}");
        }
        for fine in [
            "IMG_0001.jpg",
            "CONSOLE.jpg",
            "COM10.jpg",
            "nullish.dng",
            "COM.jpg",
        ] {
            assert!(!is_device_name(fine), "{fine}");
        }
        let mut conn = catalogue();
        let root = crate::test_support::Scratch::new("phone-refused");
        let red = phone_jpeg([200, 30, 30]);
        for refused in ["run.exe", "desktop.ini", "x.lnk", "NUL.jpg", "COM1.jpg"] {
            assert!(
                import_photo(
                    &mut conn,
                    &root.join("in"),
                    &root.join("thumbs"),
                    refused,
                    &red,
                    None
                )
                .is_err(),
                "{refused}"
            );
        }
        assert!(!root.join("in").exists(), "nothing was written");
    }

    #[test]
    fn photos_from_a_phone_go_where_settings_say() {
        let conn = catalogue();
        let fallback = Path::new("/pictures/SourceLight");
        // No library folders yet: beside the fallback.
        assert_eq!(
            import_folder(&conn, fallback).unwrap(),
            fallback.join("From phone")
        );
        set_import_folder(&conn, Some("/elsewhere")).unwrap();
        assert_eq!(
            import_folder(&conn, fallback).unwrap(),
            Path::new("/elsewhere")
        );
        set_import_folder(&conn, None).unwrap();
        assert_eq!(
            import_folder(&conn, fallback).unwrap(),
            fallback.join("From phone")
        );
    }

    #[test]
    fn a_pairing_link_makes_an_svg_qr_code() {
        let svg = qr_svg("sourcelight://pair#eyJkZXNrdG9wIjoiYWJjIn0").unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("#000000"));
    }

    #[test]
    fn the_computer_has_a_name() {
        assert!(!computer_name().trim().is_empty());
    }
}
