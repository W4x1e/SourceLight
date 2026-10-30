//! The phone's end of the link (docs/MOBILE.md §3): its key, the desktop it
//! paired with, the connection to it, and syncing the library over it.
//!
//! **Where the key lives, for now:** a file in the app's private data folder,
//! which only this app can read on both phones. Moving it into the iOS
//! Keychain and the Android Keystore is a later step; it matters once the
//! phone holds anything worth protecting beyond the pairing itself.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sl_develop::develop::EditStack;
use sl_link::protocol::{ImageSize, PairingOffer, Platform, Refusal, Request, Response};
use sl_link::{new_secret_key, PairedDesktop, Phone, Relay, Session};
use tauri::{Emitter, Manager, State};

use crate::cache::{Cache, Kind};
use crate::replica::{self, Replica, Tile, Track};
use crate::track;
use tauri_plugin_gallery::GalleryExt;
use tauri_plugin_photos::PhotosExt;
use tauri_plugin_shoot_track::{ShootTrackExt, TrackStatus};

/// Rows per page of the change feed: about 100 KB of JSON.
const PAGE: u32 = 1000;

/// What is kept between launches.
#[derive(Serialize, Deserialize)]
struct Saved {
    key: [u8; 32],
    desktop: Option<PairedDesktop>,
}

pub struct Link {
    /// The app's data folder.
    dir: PathBuf,
    file: PathBuf,
    saved: Mutex<Saved>,
    /// The endpoint, started on first use. Always with relays on: the phone
    /// cannot know in advance whether it is at home.
    phone: tokio::sync::Mutex<Option<Phone>>,
    /// The connection, when there is one. Cloned out for each request, so a
    /// screenful of thumbnails is fetched at once rather than in turn.
    session: Mutex<Option<Session>>,
    replica: Mutex<Replica>,
    cache: Cache,
    /// One sync at a time: a second waits for the first rather than both
    /// paging the same changes.
    syncing: tokio::sync::Mutex<()>,
    /// Edits to library photos go one at a time: a second would carry a
    /// base the first has just changed.
    sending_edits: tokio::sync::Mutex<()>,
    /// What became of edits sent since the last sync's summary, which says
    /// so: most go straight after they are made, not with a sync.
    edits_unsaid: Mutex<EditsSent>,
    /// Library photos whose edit from here went on a copy, and the copy:
    /// the rest of that editing goes there too, rather than making a copy
    /// with every change. Forgotten when the photo is opened again.
    copies: Mutex<HashMap<i64, i64>>,
}

/// What a sync did with edits to library photos.
#[derive(Debug, Default, Clone, Copy)]
struct EditsSent {
    /// Kept on their photos.
    sent: u32,
    /// Kept on a copy, as the photo had been edited on the computer too.
    copied: u32,
}

impl Link {
    pub fn load(dir: PathBuf) -> Result<Self, String> {
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("link.json");
        let saved = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| Saved {
                key: new_secret_key(),
                desktop: None,
            });
        let replica = Replica::open(&dir.join("library.db")).map_err(|e| e.to_string())?;
        let link = Link {
            dir: dir.clone(),
            file,
            saved: Mutex::new(saved),
            phone: tokio::sync::Mutex::new(None),
            session: Mutex::new(None),
            replica: Mutex::new(replica),
            cache: Cache::new(&dir),
            syncing: tokio::sync::Mutex::new(()),
            sending_edits: tokio::sync::Mutex::new(()),
            edits_unsaid: Mutex::default(),
            copies: Mutex::default(),
        };
        link.save();
        Ok(link)
    }

    fn save(&self) {
        let saved = self.saved.lock().unwrap();
        if let Ok(bytes) = serde_json::to_vec(&*saved) {
            let _ = std::fs::write(&self.file, bytes);
        }
    }

    async fn phone(&self) -> Result<tokio::sync::MutexGuard<'_, Option<Phone>>, String> {
        let mut phone = self.phone.lock().await;
        if phone.is_none() {
            let key = self.saved.lock().unwrap().key;
            *phone = Some(
                Phone::start(key, Relay::Public)
                    .await
                    .map_err(|e| e.to_string())?,
            );
        }
        Ok(phone)
    }

    fn session(&self) -> Option<Session> {
        self.session.lock().unwrap().clone()
    }

    /// A dead connection is dropped, so the next request reconnects.
    fn drop_session(&self) {
        *self.session.lock().unwrap() = None;
    }

    /// One of a photo's images: from the cache, or fetched from the desktop
    /// and cached. `None` when it is not cached and the desktop cannot be
    /// asked.
    pub async fn image(&self, kind: Kind, id: i64) -> Option<Vec<u8>> {
        if let Some(jpeg) = self.cache.get(kind, id) {
            return Some(jpeg);
        }
        let session = self.session()?;
        let size = match kind {
            Kind::Thumb => ImageSize::Thumb,
            Kind::Preview => ImageSize::Preview,
        };
        match session.request(&Request::Image { id, size }, &[]).await {
            Ok((Response::Image, jpeg)) if !jpeg.is_empty() => {
                self.cache.put(kind, id, &jpeg);
                Some(jpeg)
            }
            Ok(_) => None,
            Err(_) => {
                self.drop_session();
                None
            }
        }
    }

    /// Photo `id`'s preview as edited, its edits' tag being `tag`: from the
    /// cache, or rendered by the desktop and cached. `None` when it is not
    /// cached and the desktop cannot be asked, or is from before it could
    /// render one; the page shows the preview then.
    pub async fn edited(&self, id: i64, tag: &str) -> Option<Vec<u8>> {
        if let Some(jpeg) = self.cache.get_edited(id, tag) {
            return Some(jpeg);
        }
        let session = self.session()?;
        let request = Request::Image {
            id,
            size: ImageSize::Edited,
        };
        match session.request(&request, &[]).await {
            Ok((Response::Image, jpeg)) if !jpeg.is_empty() => {
                self.cache.put_edited(id, tag, &jpeg);
                Some(jpeg)
            }
            Ok(_) => None,
            Err(_) => {
                self.drop_session();
                None
            }
        }
    }

    /// Pulls the desktop's changes until there are none, sends the phone's
    /// decisions and tracks, and saves what the desktop has sent.
    async fn sync(&self, app: &tauri::AppHandle) -> Result<SyncSummary, String> {
        let _one = self.syncing.lock().await;
        let session = self.session().ok_or("Not connected.")?;
        let started = Instant::now();
        let mut changed = 0u32;
        loop {
            let since = self
                .replica
                .lock()
                .unwrap()
                .cursor()
                .map_err(|e| e.to_string())?;
            let (response, _) = session
                .request(&Request::Changes { since, limit: PAGE }, &[])
                .await
                .map_err(|e| e.to_string())?;
            let Response::Changes {
                catalogue,
                head,
                rows,
                removed,
                next,
                more,
            } = response
            else {
                return Err(format!("unexpected answer to changes: {response:?}"));
            };
            {
                let mut replica = self.replica.lock().unwrap();
                let known = replica.catalogue().map_err(|e| e.to_string())?;
                // Another library, or this one went back in time (a restored
                // backup): start the copy again.
                if known.as_deref() != Some(catalogue.as_str()) || since > head {
                    replica.reset(&catalogue).map_err(|e| e.to_string())?;
                    self.cache.clear();
                    continue;
                }
                replica
                    .apply(&rows, &removed, next)
                    .map_err(|e| e.to_string())?;
            }
            for id in &removed {
                self.cache.forget(*id);
            }
            changed += (rows.len() + removed.len()) as u32;
            if !more {
                break;
            }
        }
        self.send_decisions(&session).await?;
        self.send_tracks(&session).await?;
        let (received, receive_failed) = self.receive(app, &session).await?;
        let added = self.send_adds(app, &session).await?;
        self.send_edits(app, &session).await?;
        let edits = std::mem::take(&mut *self.edits_unsaid.lock().unwrap());
        let total = {
            let mut replica = self.replica.lock().unwrap();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            replica.set_last_synced(now).map_err(|e| e.to_string())?;
            replica.count().map_err(|e| e.to_string())?
        };
        Ok(SyncSummary {
            total,
            changed,
            received,
            receive_failed,
            added,
            edits_sent: edits.sent,
            edits_copied: edits.copied,
            millis: started.elapsed().as_millis() as u32,
        })
    }

    /// What the editor needs to open library photo `id` (step 6): its
    /// preview, and the edit to open with. That is an edit made here and
    /// not yet sent, else the computer's, kept from last time while its tag
    /// still matches, else fetched. Errors the person can act on are codes
    /// the page words (`mobile.libraryEdit.*`).
    pub async fn library_edit(&self, id: i64) -> Result<(Vec<u8>, Option<String>), String> {
        let (tag, on, kept) = {
            let replica = self.replica.lock().unwrap();
            let (tag, on) = replica
                .edit_state(id)
                .map_err(|e| e.to_string())?
                .ok_or("gone")?;
            (tag, on, replica.lib_edit(id).map_err(|e| e.to_string())?)
        };
        let stack = match kept {
            Some(e) if e.pending => e.stack,
            Some(e) if on == replica::EDIT_PHONE && e.base_tag == tag => e.base,
            _ if on == replica::EDIT_RAW || on == replica::EDIT_COMPUTER => {
                return Err(not_here(on));
            }
            // Not edited, as far as the phone knows: if the computer has
            // edited it since, the edit goes on a copy when it is sent.
            _ if on == replica::EDIT_PHONE && tag.is_none() => {
                self.replica
                    .lock()
                    .unwrap()
                    .lib_fetched(id, None, None, on)
                    .map_err(|e| e.to_string())?;
                None
            }
            _ => self.fetch_edits(id).await?,
        };
        let preview = self.image(Kind::Preview, id).await.ok_or("needsComputer")?;
        Ok((preview, stack))
    }

    async fn fetch_edits(&self, id: i64) -> Result<Option<String>, String> {
        let session = self.session().ok_or("needsComputer")?;
        match session.request(&Request::Edits { id }, &[]).await {
            Ok((
                Response::Edits {
                    edits,
                    tag,
                    edit_on,
                },
                _,
            )) => {
                let on = replica::edit_on_code(edit_on);
                self.replica
                    .lock()
                    .unwrap()
                    .lib_fetched(id, edits.as_deref(), tag.as_deref(), on)
                    .map_err(|e| e.to_string())?;
                if on == replica::EDIT_PHONE {
                    Ok(edits)
                } else {
                    Err(not_here(on))
                }
            }
            Ok((
                Response::Refused {
                    reason: Refusal::NotFound,
                },
                _,
            )) => Err("gone".into()),
            // A computer from before editing on the phone.
            Ok((
                Response::Refused {
                    reason: Refusal::BadRequest,
                },
                _,
            )) => Err("updateComputer".into()),
            Ok(_) => Err("needsComputer".into()),
            Err(_) => {
                self.drop_session();
                Err("needsComputer".into())
            }
        }
    }

    /// Keeps the edit made here to library photo `id` now, to send. One the
    /// same as the computer's has nothing to send.
    pub fn keep_library_edit(&self, id: i64, stack: &EditStack) -> Result<(), String> {
        let json = if stack.is_default() {
            None
        } else {
            Some(serde_json::to_string(stack).map_err(|e| e.to_string())?)
        };
        let mut replica = self.replica.lock().unwrap();
        let base: EditStack = replica
            .lib_edit(id)
            .map_err(|e| e.to_string())?
            .and_then(|e| e.base)
            .and_then(|b| serde_json::from_str(&b).ok())
            .unwrap_or_default();
        replica
            .lib_keep(id, json.as_deref(), *stack != base)
            .map_err(|e| e.to_string())
    }

    /// Sends edits to library photos straight away when connected; otherwise
    /// they go with the next sync.
    pub async fn send_edits_now(&self, app: &tauri::AppHandle) {
        if let Some(session) = self.session() {
            if self.send_edits(app, &session).await.is_err() {
                self.drop_session();
            }
        }
    }

    /// The copy library photo `id`'s edits from here went on, if they did.
    pub fn copy_for(&self, id: i64) -> Option<i64> {
        self.copies.lock().unwrap().get(&id).copied()
    }

    /// Photo `id` is being opened again: from its own edits, not the copy's.
    pub fn forget_copy(&self, id: i64) {
        self.copies.lock().unwrap().remove(&id);
    }

    /// Sends the edits made here to library photos, each with the edit it
    /// started from, so the computer can tell whether the photo was edited
    /// there meanwhile (then the phone's goes on a copy). A computer that
    /// can't take them now keeps them waiting for the next sync. What became
    /// of them waits in `edits_unsaid` for the next sync's summary; an edit
    /// put on a copy is also said at once (`library-edit-copied`, with the
    /// photo's id), for the editor.
    async fn send_edits(&self, app: &tauri::AppHandle, session: &Session) -> Result<(), String> {
        let _one = self.sending_edits.lock().await;
        let result = self.send_each_edit(app, session).await;
        let (Ok(done) | Err((done, _))) = &result;
        let mut unsaid = self.edits_unsaid.lock().unwrap();
        unsaid.sent += done.sent;
        unsaid.copied += done.copied;
        result.map(|_| ()).map_err(|(_, e)| e)
    }

    async fn send_each_edit(
        &self,
        app: &tauri::AppHandle,
        session: &Session,
    ) -> Result<EditsSent, (EditsSent, String)> {
        let mut done = EditsSent::default();
        let pending = self
            .replica
            .lock()
            .unwrap()
            .lib_pending()
            .map_err(|e| (done, e.to_string()))?;
        for (id, base, stack) in pending {
            let request = Request::SetEdits {
                id,
                edits: stack.clone(),
                base,
            };
            let (response, _) = session
                .request(&request, &[])
                .await
                .map_err(|e| (done, e.to_string()))?;
            let mut replica = self.replica.lock().unwrap();
            let kept = match response {
                Response::EditsSaved { copy: None, tag } => {
                    done.sent += 1;
                    replica.lib_sent(id, stack.as_deref(), tag.as_deref())
                }
                Response::EditsSaved {
                    copy: Some(copy),
                    tag,
                } => {
                    done.copied += 1;
                    // A copy of a copy is still one for the photo it came from.
                    let photo = self
                        .copies
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|(_, c)| **c == id)
                        .map_or(id, |(p, _)| *p);
                    self.copies.lock().unwrap().insert(photo, copy);
                    let _ = app.emit("library-edit-copied", photo);
                    replica.lib_moved(id, copy, stack.as_deref(), tag.as_deref())
                }
                // Gone from the library: nowhere for it to go.
                Response::Refused {
                    reason: Refusal::NotFound,
                } => replica.lib_forget(id),
                // A computer from before editing on the phone, or one that
                // can't just now: they wait.
                Response::Refused { .. } => return Ok(done),
                other => return Err((done, format!("unexpected answer to edits: {other:?}"))),
            };
            kept.map_err(|e| (done, e.to_string()))?;
        }
        Ok(done)
    }

    /// Sends the phone photos waiting for the library (Add to library),
    /// oldest first, each as stored and with its edits. One the phone or the
    /// desktop can't handle is marked as not added and the rest carry on; a
    /// dropped connection stops here, and the next sync carries on from it.
    async fn send_adds(&self, app: &tauri::AppHandle, session: &Session) -> Result<u32, String> {
        let editor = app.state::<crate::edit::Editor>();
        let waiting = editor.waiting_adds()?;
        if waiting.is_empty() {
            return Ok(0);
        }
        let dir = self.dir.join("adding");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut added = 0;
        for photo in waiting {
            let file = dir.join(photo.to_string());
            let stored = app.photos().file(photo, &file).await;
            let bytes = stored
                .as_ref()
                .map_err(|e| e.clone())
                .and_then(|_| std::fs::read(&file).map_err(|e| e.to_string()));
            let _ = std::fs::remove_file(&file);
            let (stored, bytes) = match (stored, bytes) {
                (Ok(stored), Ok(bytes)) => (stored, bytes),
                (Err(e), _) | (_, Err(e)) => {
                    editor.mark_add_failed(photo, &e)?;
                    continue;
                }
            };
            let request = Request::AddPhoto {
                name: stored.name,
                taken_ms: stored.taken_ms,
                edits: editor.edits_json(photo)?,
            };
            match session
                .request(&request, &bytes)
                .await
                .map_err(|e| e.to_string())?
                .0
            {
                Response::Added { id, .. } => {
                    editor.mark_added(photo, id)?;
                    added += 1;
                }
                // A desktop from before Add to library: they wait for one
                // that knows it.
                Response::Refused {
                    reason: Refusal::BadRequest,
                } => return Ok(added),
                Response::Refused { .. } => {
                    editor.mark_add_failed(photo, "The computer couldn't add this photo.")?
                }
                other => return Err(format!("unexpected answer to add: {other:?}")),
            }
        }
        Ok(added)
    }

    /// Saves the photos sent from the desktop (Send to phone) into the
    /// gallery. The desktop is told after each one, so a dropped connection
    /// costs at most one photo saved twice. If the gallery won't take one,
    /// this stops and says why; the rest stay on the desktop for next time.
    async fn receive(
        &self,
        app: &tauri::AppHandle,
        session: &Session,
    ) -> Result<(u32, Option<String>), String> {
        let (response, _) = session
            .request(&Request::Outbox, &[])
            .await
            .map_err(|e| e.to_string())?;
        let files = match response {
            Response::Outbox { files } => files,
            // A desktop from before Send to phone.
            Response::Refused {
                reason: Refusal::BadRequest,
            } => return Ok((0, None)),
            other => return Err(format!("unexpected answer to outbox: {other:?}")),
        };
        if files.is_empty() {
            return Ok((0, None));
        }
        let incoming = self.dir.join("incoming");
        std::fs::create_dir_all(&incoming).map_err(|e| e.to_string())?;
        let mut saved = 0;
        for file in files {
            let name = file_name(&file.name);
            // Not a photo the gallery takes: nothing to fetch, but off the list.
            if tauri_plugin_gallery::mime_type(&name).is_some() {
                let (response, bytes) = session
                    .request(&Request::OutboxFile { id: file.id }, &[])
                    .await
                    .map_err(|e| e.to_string())?;
                match response {
                    Response::File => {
                        let temp = incoming.join(format!("{}-{name}", file.id));
                        std::fs::write(&temp, &bytes).map_err(|e| e.to_string())?;
                        let result = app.gallery().save(&temp, &name).await.map(|_| ());
                        let _ = std::fs::remove_file(&temp);
                        if let Err(e) = result {
                            return Ok((saved, Some(e)));
                        }
                        saved += 1;
                    }
                    // Gone from the desktop: nothing to save, but off the list.
                    Response::Refused {
                        reason: Refusal::NotFound,
                    } => {}
                    other => return Err(format!("unexpected answer to outbox file: {other:?}")),
                }
            }
            session
                .request(&Request::Received { ids: vec![file.id] }, &[])
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok((saved, None))
    }

    /// Sends the tracks the computer does not have yet, oldest first.
    async fn send_tracks(&self, session: &Session) -> Result<(), String> {
        let unsent: Vec<Track> = {
            let replica = self.replica.lock().unwrap();
            let mut all = replica.tracks().map_err(|e| e.to_string())?;
            all.retain(|t| !t.sent);
            all.reverse();
            all
        };
        for t in unsent {
            let Ok(gpx) = std::fs::read(track::here(&self.dir, &t.file)) else {
                continue;
            };
            let request = Request::Track {
                name: t.name.clone(),
                started_ms: t.started_ms,
                ended_ms: t.ended_ms,
            };
            match session
                .request(&request, &gpx)
                .await
                .map_err(|e| e.to_string())?
                .0
            {
                Response::TrackSaved { .. } => self
                    .replica
                    .lock()
                    .unwrap()
                    .track_sent(t.id)
                    .map_err(|e| e.to_string())?,
                other => return Err(format!("track not taken: {other:?}")),
            }
        }
        Ok(())
    }

    async fn send_decisions(&self, session: &Session) -> Result<(), String> {
        let (verdicts, last) = self
            .replica
            .lock()
            .unwrap()
            .pending()
            .map_err(|e| e.to_string())?;
        if verdicts.is_empty() {
            return Ok(());
        }
        match session
            .request(&Request::Verdicts { verdicts }, &[])
            .await
            .map_err(|e| e.to_string())?
            .0
        {
            Response::Applied { .. } => self
                .replica
                .lock()
                .unwrap()
                .sent(last)
                .map_err(|e| e.to_string()),
            other => Err(format!("decisions not applied: {other:?}")),
        }
    }
}

/// Why library photo editing isn't offered here, as the code the page words.
fn not_here(edit_on: u8) -> String {
    match edit_on {
        replica::EDIT_RAW => "raw",
        replica::EDIT_COMPUTER => "computerOnly",
        _ => "updateComputer",
    }
    .into()
}

/// The file's own name, never a path the desktop chose: the last part, with
/// nothing that is not printable.
fn file_name(sent: &str) -> String {
    let last = sent.rsplit(['/', '\\']).next().unwrap_or(sent);
    let clean: String = last.chars().filter(|c| !c.is_control()).collect();
    match clean.trim() {
        "" | "." | ".." => "photo".into(),
        _ => clean,
    }
}

fn platform() -> Platform {
    if cfg!(target_os = "android") {
        Platform::Android
    } else if cfg!(target_os = "ios") {
        Platform::Ios
    } else {
        Platform::Other
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkStatus {
    /// The paired desktop's name, if any.
    desktop: Option<String>,
    connected: bool,
    relayed: bool,
    /// "android", "ios" or "other": the interface's wording depends on it.
    platform: &'static str,
    /// When the phone last synced with the computer, ms since the epoch.
    last_synced_ms: Option<i64>,
}

#[tauri::command]
pub async fn link_status(link: State<'_, Link>) -> Result<LinkStatus, String> {
    let desktop = link
        .saved
        .lock()
        .unwrap()
        .desktop
        .as_ref()
        .map(|d| d.name.clone());
    let session = link.session();
    Ok(LinkStatus {
        desktop,
        connected: session.is_some(),
        relayed: session.as_ref().is_some_and(|s| s.is_relayed()),
        last_synced_ms: link.replica.lock().unwrap().last_synced().ok().flatten(),
        platform: match platform() {
            Platform::Android => "android",
            Platform::Ios => "ios",
            Platform::Other => "other",
        },
    })
}

/// What is waiting here for the next sync, for the sync sheet: ratings and
/// flags, edits to library photos, photos to add, and GPS tracks.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    decisions: usize,
    edits: usize,
    adds: usize,
    tracks: usize,
}

#[tauri::command]
pub async fn link_waiting(
    link: State<'_, Link>,
    editor: State<'_, crate::edit::Editor>,
) -> Result<Waiting, String> {
    let (decisions, edits, tracks) = {
        let replica = link.replica.lock().unwrap();
        let decisions = replica.pending().map_err(|e| e.to_string())?.0.len();
        let edits = replica.lib_pending().map_err(|e| e.to_string())?.len();
        let tracks = replica
            .tracks()
            .map_err(|e| e.to_string())?
            .iter()
            .filter(|t| !t.sent)
            .count();
        (decisions, edits, tracks)
    };
    let adds = editor.waiting_adds()?.len();
    Ok(Waiting {
        decisions,
        edits,
        adds,
        tracks,
    })
}

/// Pairs with the desktop whose code was scanned or pasted. Resolves once
/// someone at the desktop answers.
#[tauri::command]
pub async fn pair(
    link: State<'_, Link>,
    code: String,
    device_name: String,
) -> Result<String, String> {
    let offer = PairingOffer::from_uri(&code).map_err(|e| e.to_string())?;
    let desktop = {
        let phone = link.phone().await?;
        let phone = phone.as_ref().expect("started above");
        phone
            .pair(&offer, device_name.trim(), platform())
            .await
            .map_err(|e| e.to_string())?
    };
    let name = desktop.name.clone();
    link.saved.lock().unwrap().desktop = Some(desktop);
    link.save();
    link.drop_session();
    Ok(name)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connected {
    desktop_name: String,
    relayed: bool,
    millis: u32,
}

/// Connects to the paired desktop and says hello.
#[tauri::command]
pub async fn connect(link: State<'_, Link>) -> Result<Connected, String> {
    let desktop = link
        .saved
        .lock()
        .unwrap()
        .desktop
        .clone()
        .ok_or("Not paired with a computer yet.")?;
    let started = Instant::now();
    let session = {
        let phone = link.phone().await?;
        let phone = phone.as_ref().expect("started above");
        phone.connect(&desktop).await.map_err(|e| e.to_string())?
    };
    let name = session
        .hello(env!("CARGO_PKG_VERSION"), "phone")
        .await
        .map_err(|e| e.to_string())?;
    let relayed = session.is_relayed();
    *link.session.lock().unwrap() = Some(session);
    Ok(Connected {
        desktop_name: name,
        relayed,
        millis: started.elapsed().as_millis() as u32,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pinged {
    millis: f64,
    relayed: bool,
}

#[tauri::command]
pub async fn ping(link: State<'_, Link>) -> Result<Pinged, String> {
    let session = link.session().ok_or("Not connected.")?;
    match session.ping().await {
        Ok(rtt) => Ok(Pinged {
            millis: rtt.as_secs_f64() * 1000.0,
            relayed: session.is_relayed(),
        }),
        Err(e) => {
            link.drop_session();
            Err(e.to_string())
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    /// Photos on the phone now.
    total: u32,
    /// Rows added, changed or removed by this sync.
    changed: u32,
    /// Photos sent from the desktop and saved to the gallery by this sync.
    received: u32,
    /// Why the gallery would not take one, if it would not.
    receive_failed: Option<String>,
    /// Phone photos added to the library by this sync.
    added: u32,
    /// Edits to library photos the computer took, on the photos themselves
    /// or (edited there too) on copies.
    edits_sent: u32,
    edits_copied: u32,
    millis: u32,
}

/// Brings the phone's copy up to date, sends its decisions and tracks, and
/// saves photos sent from the desktop.
#[tauri::command]
pub async fn sync(app: tauri::AppHandle, link: State<'_, Link>) -> Result<SyncSummary, String> {
    let result = link.sync(&app).await;
    if result.is_err() {
        link.drop_session();
    }
    result
}

/// The library as the phone has it, newest first: `[id, flag, rating,
/// edited, editOn]` per photo, compact because a big library is tens of
/// thousands of them.
#[tauri::command]
pub fn photos(link: State<'_, Link>) -> Result<Vec<Tile>, String> {
    link.replica
        .lock()
        .unwrap()
        .tiles()
        .map_err(|e| e.to_string())
}

/// Library photo `id`'s file name, for the loupe; none if it isn't here.
#[tauri::command]
pub fn library_photo_name(link: State<'_, Link>, id: i64) -> Result<Option<String>, String> {
    link.replica
        .lock()
        .unwrap()
        .filename(id)
        .map_err(|e| e.to_string())
}

/// The names the computer's search box recognises, as JSON, for reading a
/// query on the phone the same way. Fails with a code the page words.
#[tauri::command]
pub async fn library_vocabulary(link: State<'_, Link>) -> Result<String, String> {
    let session = link.session().ok_or("needsComputer")?;
    match session.request(&Request::Vocabulary, &[]).await {
        Ok((Response::Vocabulary { json }, _)) => Ok(json),
        Ok((
            Response::Refused {
                reason: Refusal::BadRequest,
            },
            _,
        )) => Err("updateComputer".into()),
        Ok(_) => Err("failed".into()),
        Err(_) => {
            link.drop_session();
            Err("needsComputer".into())
        }
    }
}

/// What a search found: library photos this phone has, best first when
/// `ranked`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    ids: Vec<i64>,
    ranked: bool,
}

/// Searches the library on the computer with `filter`, the desktop's photo
/// filter as JSON. Photos the phone hasn't (copies, ones not synced yet) are
/// left out. Fails with a code the page words.
#[tauri::command]
pub async fn library_search(link: State<'_, Link>, filter: String) -> Result<Found, String> {
    let session = link.session().ok_or("needsComputer")?;
    match session.request(&Request::Search { filter }, &[]).await {
        Ok((Response::Found { ids, ranked }, _)) => {
            let have: std::collections::HashSet<i64> = link
                .replica
                .lock()
                .unwrap()
                .tiles()
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|t| t.0)
                .collect();
            Ok(Found {
                ids: ids.into_iter().filter(|id| have.contains(id)).collect(),
                ranked,
            })
        }
        Ok((
            Response::Refused {
                reason: Refusal::BadRequest,
            },
            _,
        )) => Err("updateComputer".into()),
        Ok(_) => Err("failed".into()),
        Err(_) => {
            link.drop_session();
            Err("needsComputer".into())
        }
    }
}

/// A pick (1), reject (-1) or neither (0), made on the phone. Kept at once,
/// and sent straight away if connected; otherwise with the next sync.
#[tauri::command]
pub async fn set_flag(link: State<'_, Link>, id: i64, flag: i8) -> Result<(), String> {
    link.replica
        .lock()
        .unwrap()
        .set_flag(id, flag.clamp(-1, 1))
        .map_err(|e| e.to_string())?;
    if let Some(session) = link.session() {
        // Not an error for the tap if this fails: the decision is kept and
        // goes with the next sync.
        if link.send_decisions(&session).await.is_err() {
            link.drop_session();
        }
    }
    Ok(())
}

/// A rating, 0 to 5, made on the phone. Kept at once, and sent straight away
/// if connected; otherwise with the next sync.
#[tauri::command]
pub async fn set_rating(link: State<'_, Link>, id: i64, rating: u8) -> Result<(), String> {
    link.replica
        .lock()
        .unwrap()
        .set_rating(id, rating.min(5))
        .map_err(|e| e.to_string())?;
    if let Some(session) = link.session() {
        if link.send_decisions(&session).await.is_err() {
            link.drop_session();
        }
    }
    Ok(())
}

/// The recording, if any, and every track recorded on this phone.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tracks {
    status: TrackStatus,
    tracks: Vec<Track>,
}

#[tauri::command]
pub async fn tracks(app: tauri::AppHandle, link: State<'_, Link>) -> Result<Tracks, String> {
    let status = app.shoot_track().status().await?;
    let tracks = link
        .replica
        .lock()
        .unwrap()
        .tracks()
        .map_err(|e| e.to_string())?;
    Ok(Tracks { status, tracks })
}

/// Starts recording a track, asking for location first if Android has not.
#[tauri::command]
pub async fn track_start(
    app: tauri::AppHandle,
    link: State<'_, Link>,
) -> Result<TrackStatus, String> {
    let plugin = app.shoot_track();
    if !plugin.request_permissions().await? {
        return Err("Location permission was not given.".into());
    }
    let file = track::new_recording(&link.dir);
    plugin
        .start(&file.to_string_lossy(), track::INTERVAL_MS)
        .await
}

/// Stops recording, keeps the track as GPX named `name`, and sends it to the
/// computer straight away if connected (otherwise with the next sync).
#[tauri::command]
pub async fn track_stop(
    app: tauri::AppHandle,
    link: State<'_, Link>,
    name: String,
) -> Result<Tracks, String> {
    let status = app.shoot_track().stop().await?;
    if let Some(file) = status.file.as_deref() {
        // By name: the plugin's note of the file may be from before iOS
        // moved the app's folder.
        if let Some(done) =
            track::finish(&track::here(&link.dir, file), name.trim()).map_err(|e| e.to_string())?
        {
            let gpx = done.gpx.file_name().unwrap_or_default().to_string_lossy();
            link.replica
                .lock()
                .unwrap()
                .add_track(
                    &gpx,
                    name.trim(),
                    done.started_ms,
                    done.ended_ms,
                    done.points,
                )
                .map_err(|e| e.to_string())?;
        }
    }
    if let Some(session) = link.session() {
        if link.send_tracks(&session).await.is_err() {
            link.drop_session();
        }
    }
    let tracks = link
        .replica
        .lock()
        .unwrap()
        .tracks()
        .map_err(|e| e.to_string())?;
    Ok(Tracks {
        status: app.shoot_track().status().await?,
        tracks,
    })
}

/// Scans the desktop's pairing code with the camera. `None` when the person
/// backs out.
#[tauri::command]
pub async fn scan_qr(app: tauri::AppHandle, prompt: String) -> Result<Option<String>, String> {
    use tauri_plugin_qr_scan::QrScanExt;
    app.qr_scan().scan(&prompt).await
}

/// Forgets the desktop and the phone's copy of its library. Its list still
/// shows this phone until it is removed there.
#[tauri::command]
pub async fn forget(link: State<'_, Link>) -> Result<(), String> {
    if let Some(session) = link.session.lock().unwrap().take() {
        session.close();
    }
    link.saved.lock().unwrap().desktop = None;
    link.save();
    link.replica
        .lock()
        .unwrap()
        .reset("")
        .map_err(|e| e.to_string())?;
    link.cache.clear();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::file_name;

    /// What the desktop sends is a name, and only ever used as one.
    #[test]
    fn a_sent_name_is_only_a_name() {
        assert_eq!(file_name("IMG_0001.jpg"), "IMG_0001.jpg");
        assert_eq!(file_name("../../shared_prefs/x.jpg"), "x.jpg");
        assert_eq!(file_name("C:\\Users\\a\\b.jpg"), "b.jpg");
        assert_eq!(file_name("bad\u{0}name.jpg"), "badname.jpg");
        assert_eq!(file_name(".."), "photo");
        assert_eq!(file_name(""), "photo");
    }
}
