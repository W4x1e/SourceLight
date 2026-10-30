//! Develop on the phone (docs/MOBILE.md section 4, steps 4 and 6): the
//! phone's own photos, and the library's non-raw ones, edited with the
//! desktop's own pipeline, on the phone's GPU.
//!
//! One photo is open at a time, on the GPU, with its undo history. Its edits
//! are kept as settings after every change: a phone photo's in `edits`, a
//! library photo's in the library copy, to send to the computer. No photo is
//! ever written. Save copy renders a phone photo at full size into the
//! gallery. A library photo is edited on its preview, which is what the
//! phone has of it.
//!
//! The pipeline is the desktop's own, from the shared `sl-develop` crate.
//! The GPU device is made as the GPU test makes it (`sl_gpu_test::bench`).
//!
//! Each opening gets a token, which the page sends back with every change.
//! An editor that is closing can still be sending its last change, and its
//! Close, after the next one opened: the token keeps those off the photo
//! now open (`CLOSED`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sl_develop::develop::auto::{self, AutoTone, AutoWb};
use sl_develop::develop::eyedropper::neutral_point;
use sl_develop::develop::geometry::ViewPort;
use sl_develop::develop::lut_packs::{self, LookInfo};
use sl_develop::develop::presets::{self, PresetGroupInfo};
use sl_develop::develop::stack::{Group, CURRENT_VERSION};
use sl_develop::develop::{EditStack, History};
use sl_develop::gpu::pipeline::{DevelopSession, Pipeline};
use sl_develop::gpu::Gpu;
use tauri::http::Response;
use tauri::{Manager, State};
use tauri_plugin_gallery::GalleryExt;
use tauri_plugin_photos::PhotosExt;

use crate::edits::Edits;

/// The longest side a photo is edited at. A 12 MP phone photo is edited at
/// full size; a 50 MP one at this, which keeps it to about 130 MB on the GPU.
const EDIT_EDGE: u32 = 4096;
/// JPEG quality of a saved copy.
const SAVE_QUALITY: u8 = 92;
/// What a change from an editor whose photo has gone says: the page opens it
/// again.
const CLOSED: &str = "closed";
/// The longest side of the render Auto measures, as on the desktop.
const AUTO_EDGE: u32 = 320;
/// The furthest a frame zooms in: screen pixels per pixel of the photo.
const MOST_ZOOM: f32 = 4.0;

pub struct Editor {
    inner: Arc<Mutex<Inner>>,
    edits: Mutex<Edits>,
}

#[derive(Default)]
struct Inner {
    /// Made on first use and kept: compiling the shaders takes a moment.
    engine: Option<Engine>,
    open: Option<Open>,
    /// Film looks made so far, by file name: making one takes a moment.
    looks: HashMap<String, Option<Vec<f32>>>,
    /// The last token given out.
    tokens: u64,
}

impl Inner {
    /// The GPU and the develop pipeline, made the first time they are wanted.
    /// Making them takes the better part of a second on a phone (the shaders
    /// compile), so `edit_warm` makes them ahead of time.
    fn engine(&mut self) -> Result<&Engine, String> {
        if self.engine.is_none() {
            let gpu = sl_gpu_test::bench::phone_gpu().map_err(err)?;
            let pipeline = gpu
                .checked("preparing editing", || Ok(Pipeline::new(&gpu)))
                .map_err(err)?;
            self.engine = Some(Engine { gpu, pipeline });
        }
        Ok(self.engine.as_ref().expect("made above"))
    }

    fn next_token(&mut self) -> u64 {
        self.tokens += 1;
        self.tokens
    }

    /// The open photo, if `token` is the editor's that opened it.
    fn open_for(&mut self, token: u64) -> Result<&mut Open, String> {
        self.open
            .as_mut()
            .filter(|o| o.token == token)
            .ok_or_else(|| CLOSED.to_string())
    }

    /// Puts film look `name` on the open photo's GPU session, if it isn't
    /// there already; an empty name, or one this build doesn't make, takes
    /// it off.
    fn use_look(&mut self, name: &str) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if open.look == name {
            return;
        }
        let texels = if name.is_empty() {
            None
        } else {
            self.looks
                .entry(name.to_string())
                .or_insert_with(|| lut_packs::texels(name))
                .clone()
        };
        open.session.lut_texels = texels;
        open.look = name.to_string();
    }
}

struct Engine {
    gpu: Gpu,
    pipeline: Pipeline,
}

/// Which photo is open.
#[derive(Debug, Clone, PartialEq)]
enum Source {
    /// One of the phone's own.
    Device {
        id: i64,
        modified: i64,
        name: String,
        taken_ms: i64,
    },
    /// One of the library's.
    Library { id: i64 },
}

struct Open {
    source: Source,
    session: DevelopSession,
    history: History,
    /// The film look on the GPU session now.
    look: String,
    /// The opening's, which the page sends with each change.
    token: u64,
}

impl Editor {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let edits = Edits::open(&dir.join("device.db")).map_err(|e| e.to_string())?;
        Ok(Editor {
            inner: Arc::default(),
            edits: Mutex::new(edits),
        })
    }

    /// Photo `photo`'s edits as JSON, for Add to library.
    pub fn edits_json(&self, photo: i64) -> Result<Option<String>, String> {
        self.edits
            .lock()
            .unwrap()
            .get(photo)
            .map_err(|e| e.to_string())
    }

    pub fn waiting_adds(&self) -> Result<Vec<i64>, String> {
        self.edits
            .lock()
            .unwrap()
            .waiting()
            .map_err(|e| e.to_string())
    }

    pub fn mark_added(&self, photo: i64, library_id: i64) -> Result<(), String> {
        self.edits
            .lock()
            .unwrap()
            .added(photo, library_id)
            .map_err(|e| e.to_string())
    }

    pub fn mark_add_failed(&self, photo: i64, error: &str) -> Result<(), String> {
        self.edits
            .lock()
            .unwrap()
            .add_failed(photo, error)
            .map_err(|e| e.to_string())
    }

    /// Keeps the open photo's current edits, or forgets them when there are
    /// none left. A library photo's go to the computer straight away when it
    /// is connected, and with the next sync when not; once they went on a
    /// copy (the photo was edited on the computer too), to that copy.
    fn keep(&self, app: &tauri::AppHandle, open: &mut Open) -> Result<(), String> {
        if let Source::Library { id } = open.source {
            if let Some(copy) = app.state::<crate::link::Link>().copy_for(id) {
                open.source = Source::Library { id: copy };
            }
        }
        let stack = open.history.current();
        match &open.source {
            Source::Device { id, modified, .. } => {
                let json = if stack.is_default() {
                    None
                } else {
                    Some(serde_json::to_string(stack).map_err(|e| e.to_string())?)
                };
                self.edits
                    .lock()
                    .unwrap()
                    .put(*id, *modified, json.as_deref())
                    .map_err(|e| e.to_string())
            }
            Source::Library { id } => {
                app.state::<crate::link::Link>()
                    .keep_library_edit(*id, stack)?;
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    app.state::<crate::link::Link>().send_edits_now(&app).await;
                });
                Ok(())
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    /// Sent back with every change to this opening.
    token: u64,
    stack: EditStack,
    /// The photo upright, before any crop or turn: what crop maths need.
    width: u32,
    height: u32,
    can_undo: bool,
    can_redo: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    stack: EditStack,
    can_undo: bool,
    can_redo: bool,
}

fn opened(open: &Open) -> Opened {
    Opened {
        token: open.token,
        stack: open.history.current().clone(),
        width: open.session.width,
        height: open.session.height,
        can_undo: open.history.can_undo(),
        can_redo: open.history.can_redo(),
    }
}

fn step(open: &Open) -> Step {
    Step {
        stack: open.history.current().clone(),
        can_undo: open.history.can_undo(),
        can_redo: open.history.can_redo(),
    }
}

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// The open photo, if it is `source`'s, as the page opens it. It gets a new
/// token: an editor that had it before may still be closing.
fn already_open(editor: &Editor, is: impl Fn(&Source) -> bool) -> Option<Opened> {
    let mut inner = editor.inner.lock().unwrap();
    let token = inner.next_token();
    let open = inner.open.as_mut().filter(|o| is(&o.source))?;
    open.token = token;
    Some(opened(open))
}

/// Where a photo waits to be decoded. One photo at a time: whatever an
/// earlier one left goes.
fn fresh_edit_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("edit");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Gets the GPU and the develop pipeline ready, if they aren't, off the main
/// thread. The page asks as a photo opens in the loupe, so that tapping Edit
/// doesn't wait for the shaders: about 0.4 to 0.6 s on a Pixel 8 Pro.
#[tauri::command]
pub async fn edit_warm(editor: State<'_, Editor>) -> Result<(), String> {
    let inner = editor.inner.clone();
    tauri::async_runtime::spawn_blocking(move || inner.lock().unwrap().engine().map(|_| ()))
        .await
        .map_err(|e| e.to_string())?
}

/// Opens phone photo `id` for editing, with the edits it already has.
#[tauri::command]
pub async fn edit_open(
    app: tauri::AppHandle,
    editor: State<'_, Editor>,
    id: i64,
    modified: i64,
) -> Result<Opened, String> {
    let same = |s: &Source| matches!(s, Source::Device { id: i, modified: m, .. } if *i == id && *m == modified);
    if let Some(open) = already_open(&editor, same) {
        return Ok(open);
    }
    let file = fresh_edit_dir(&app)?.join(format!("{id}_{modified}"));
    let original = app.photos().original(id, EDIT_EDGE, &file).await?;
    let saved = editor
        .edits
        .lock()
        .unwrap()
        .get(id)
        .map_err(|e| e.to_string())?;
    // Settings a newer build wrote, or that no longer parse, open as none.
    let stack: EditStack = saved
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let source = Source::Device {
        id,
        modified,
        name: original.name,
        taken_ms: original.taken_ms,
    };
    open_on_gpu(editor.inner.clone(), file, stack, source).await
}

/// Opens library photo `id` for editing (step 6): its preview, from the
/// phone's cache or the computer, with its edits. Fails with a code the page
/// words when it can't be edited here (`mobile.libraryEdit.*`).
#[tauri::command]
pub async fn edit_open_library(
    app: tauri::AppHandle,
    editor: State<'_, Editor>,
    id: i64,
) -> Result<Opened, String> {
    let link = app.state::<crate::link::Link>();
    if link.copy_for(id).is_none() {
        if let Some(open) = already_open(&editor, |s| *s == Source::Library { id }) {
            return Ok(open);
        }
    }
    link.forget_copy(id);
    let (preview, saved) = link.library_edit(id).await?;
    // Edits this build can't read are never opened as none: saving would
    // replace them on the computer.
    let stack: EditStack = match saved {
        Some(json) => serde_json::from_str(&json).map_err(|_| "updatePhone")?,
        None => EditStack::default(),
    };
    if stack.version > CURRENT_VERSION {
        return Err("updatePhone".into());
    }
    let file = fresh_edit_dir(&app)?.join(format!("library_{id}.jpg"));
    std::fs::write(&file, preview).map_err(|e| e.to_string())?;
    open_on_gpu(editor.inner.clone(), file, stack, Source::Library { id }).await
}

/// Decodes `file` onto the GPU as the open photo, with `stack` as its edit
/// so far, and deletes the file.
async fn open_on_gpu(
    inner: Arc<Mutex<Inner>>,
    file: PathBuf,
    stack: EditStack,
    source: Source,
) -> Result<Opened, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut inner = inner.lock().unwrap();
        // The last photo off the GPU before the next goes on.
        inner.open = None;
        let engine = inner.engine()?;
        let img = sl_develop::raster::decode(&file).map_err(err)?;
        let session = engine
            .gpu
            .checked("opening the photo", || {
                engine.pipeline.prepare_rendered(&engine.gpu, &img)
            })
            .map_err(err)?;
        drop(img);
        let _ = std::fs::remove_file(&file);
        let open = Open {
            source,
            session,
            history: History::new(stack),
            look: String::new(),
            token: inner.next_token(),
        };
        let out = opened(&open);
        inner.open = Some(open);
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Runs `f` on the history of `token`'s opening and keeps where it leaves
/// the edit, off the main thread: a frame being drawn holds the photo, and
/// the main thread waiting for it is the app not responding.
async fn on_history(
    app: tauri::AppHandle,
    token: u64,
    f: impl FnOnce(&mut History) + Send + 'static,
) -> Result<Step, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let editor = app.state::<Editor>();
        let mut inner = editor.inner.lock().unwrap();
        let open = inner.open_for(token)?;
        f(&mut open.history);
        Ok(step(open))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Records `stack` as the open photo's next step. Steps stay with the
/// editor until Done (`edit_done`) keeps them; Cancel closes without.
#[tauri::command]
pub async fn edit_commit(
    app: tauri::AppHandle,
    token: u64,
    stack: EditStack,
) -> Result<Step, String> {
    on_history(app, token, move |h| h.push("Edit", stack)).await
}

#[tauri::command]
pub async fn edit_undo(app: tauri::AppHandle, token: u64) -> Result<Step, String> {
    on_history(app, token, |h| {
        let _ = h.undo();
    })
    .await
}

#[tauri::command]
pub async fn edit_redo(app: tauri::AppHandle, token: u64) -> Result<Step, String> {
    on_history(app, token, |h| {
        let _ = h.redo();
    })
    .await
}

/// Runs `f` on the open photo off the main thread, if `token`'s editor still
/// has it, with film look `look` on its GPU session.
async fn on_open<T: Send + 'static>(
    editor: &Editor,
    token: u64,
    look: String,
    f: impl FnOnce(&Engine, &Open) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let inner = editor.inner.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut inner = inner.lock().unwrap();
        inner.open_for(token)?;
        inner.use_look(&look);
        let (Some(engine), Some(open)) = (inner.engine.as_ref(), inner.open.as_ref()) else {
            return Err(CLOSED.to_string());
        };
        f(engine, open)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Which Auto: the tone sliders, or white balance.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Auto {
    Tone,
    Wb,
}

/// `stack` with Auto worked out on the open photo, as the desktop's Auto
/// tone and Auto WB do it: measured on a small render with those fields
/// neutral, so pressing it twice gives the same. Auto WB leaves the balance
/// as it is when too little of the photo is near grey to trust. Not kept:
/// the page makes it a change like any other.
#[tauri::command]
pub async fn edit_auto(
    editor: State<'_, Editor>,
    token: u64,
    stack: EditStack,
    kind: Auto,
) -> Result<EditStack, String> {
    let look = stack.lut.name.clone();
    on_open(&editor, token, look, move |engine, open| {
        let measurement = match kind {
            Auto::Tone => AutoTone::neutral_measurement(&stack),
            Auto::Wb => AutoWb::neutral_measurement(&stack),
        };
        let (w, h) = open.session.framed_proxy(&measurement, AUTO_EDGE);
        let pixels = engine
            .gpu
            .checked("measuring the photo", || {
                engine
                    .pipeline
                    .render_to_pixels(&engine.gpu, &open.session, &measurement, w, h)
            })
            .map_err(err)?;
        let mut target = stack;
        match kind {
            Auto::Tone => auto::auto_tone(&auto::stats_from_pixels(&pixels)).apply_to(&mut target),
            Auto::Wb => auto::auto_wb(&auto::wb_stats_from_pixels(&pixels)).apply_to(&mut target),
        }
        Ok(target)
    })
    .await
}

/// `stack` with the white balance that turns the point (`x`, `y`) grey, in
/// the framed photo from 0 to 1: the desktop's eyedropper. Not kept.
#[tauri::command]
pub async fn edit_pick_wb(
    editor: State<'_, Editor>,
    token: u64,
    stack: EditStack,
    x: f32,
    y: f32,
) -> Result<EditStack, String> {
    let look = stack.lut.name.clone();
    on_open(&editor, token, look, move |engine, open| {
        let wb = engine
            .gpu
            .checked("picking the white balance", || {
                neutral_point(&engine.gpu, &engine.pipeline, &open.session, &stack, x, y)
            })
            .map_err(err)?;
        let mut target = stack;
        wb.apply_to(&mut target);
        Ok(target)
    })
    .await
}

/// `onto` with the copied edit `copied` laid over it, as the desktop's Paste
/// does by default: everything that means the same on another photo, and
/// not the crop, masks or spots, which fit only the photo they were made on.
#[tauri::command]
pub fn edit_paste(copied: EditStack, onto: EditStack) -> EditStack {
    let groups: Vec<Group> = Group::ALL
        .into_iter()
        .filter(|g| g.copied_by_default())
        .collect();
    copied.take_groups(&onto, &groups)
}

/// The desktop's built-in presets, by group, in its order.
#[tauri::command]
pub fn edit_presets() -> Vec<PresetGroupInfo> {
    presets::catalogue()
}

/// The desktop's built-in film looks, in its order. A look is the edit's
/// `lut`, by file name, as on the desktop.
#[tauri::command]
pub fn edit_looks() -> Vec<LookInfo> {
    lut_packs::catalogue()
}

/// `stack` with built-in preset `id` on it at `amount` (1 as designed), as
/// the desktop applies one: it replaces the creative treatment and keeps
/// exposure, white balance and crop. An empty `id` takes the preset off.
#[tauri::command]
pub fn edit_preset(stack: EditStack, id: String, amount: f32) -> Result<EditStack, String> {
    if id.is_empty() {
        return Ok(presets::remove(&stack));
    }
    presets::apply(&id, amount, &stack).ok_or_else(|| format!("no preset called {id}"))
}

/// Photos with edits, for the grid's marks.
#[tauri::command]
pub fn edit_edited(editor: State<'_, Editor>) -> Result<Vec<i64>, String> {
    editor
        .edits
        .lock()
        .unwrap()
        .edited()
        .map_err(|e| e.to_string())
}

/// One of the phone's photos, as Add to library knows it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAdd {
    photo: i64,
    /// The library photo it became; none while it waits.
    library_id: Option<i64>,
    /// Why it could not be added, if it could not.
    error: Option<String>,
}

/// Puts these phone photos in line for the library (Add to library). They go
/// on the next sync with the computer. Returns how many are newly waiting.
#[tauri::command]
pub fn device_add(editor: State<'_, Editor>, photos: Vec<i64>) -> Result<u32, String> {
    editor
        .edits
        .lock()
        .unwrap()
        .queue_adds(&photos)
        .map_err(|e| e.to_string())
}

/// Every phone photo waiting for the library, added to it, or not added.
#[tauri::command]
pub fn device_adds(editor: State<'_, Editor>) -> Result<Vec<DeviceAdd>, String> {
    let adds = editor
        .edits
        .lock()
        .unwrap()
        .adds()
        .map_err(|e| e.to_string())?;
    Ok(adds
        .into_iter()
        .map(|(photo, library_id, error)| DeviceAdd {
            photo,
            library_id,
            error,
        })
        .collect())
}

/// Done: keeps the open photo's edits as they stand. A phone photo's go in
/// its settings; a library photo's go to the computer (straight away when
/// connected, otherwise with the next sync). Until now they were only steps
/// in the editor, so Cancel, or the app going, leaves the photo as it was.
#[tauri::command]
pub async fn edit_done(app: tauri::AppHandle, token: u64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let editor = app.state::<Editor>();
        let mut inner = editor.inner.lock().unwrap();
        let open = inner.open_for(token)?;
        editor.keep(&app, open)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Lets the open photo go, freeing the GPU memory it holds, if `token`'s
/// editor still has it. Off the main thread, as a frame may be drawing.
#[tauri::command]
pub async fn edit_close(app: tauri::AppHandle, token: u64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let editor = app.state::<Editor>();
        let mut inner = editor.inner.lock().unwrap();
        if inner.open_for(token).is_ok() {
            inner.open = None;
        }
    })
    .await
    .map_err(|e| e.to_string())
}

/// Renders the open photo at full size, as edited, and saves it to the
/// gallery beside the original. Returns the new file's name.
#[tauri::command]
pub async fn edit_save(
    app: tauri::AppHandle,
    editor: State<'_, Editor>,
    token: u64,
) -> Result<String, String> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("edit-save");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let inner = editor.inner.clone();
    let (file, name, id, taken_ms) = tauri::async_runtime::spawn_blocking(move || {
        let mut inner = inner.lock().unwrap();
        let look = inner.open_for(token)?.history.current().lut.name.clone();
        inner.use_look(&look);
        let engine = inner.engine.as_ref().ok_or(CLOSED)?;
        let open = inner.open.as_ref().ok_or(CLOSED)?;
        let Source::Device {
            id, name, taken_ms, ..
        } = &open.source
        else {
            return Err("A library photo is saved from the computer.".into());
        };
        let stack = open.history.current();
        let (w, h) = open.session.framed_size(stack);
        let rgba = engine
            .gpu
            .checked("saving the photo", || {
                engine
                    .pipeline
                    .render_to_pixels(&engine.gpu, &open.session, stack, w, h)
            })
            .map_err(err)?;
        let jpeg = encode(&rgba, w, h, SAVE_QUALITY, true)?;
        let name = copy_name(name);
        let file = dir.join(&name);
        std::fs::write(&file, jpeg).map_err(|e| e.to_string())?;
        Ok::<_, String>((file, name, *id, *taken_ms))
    })
    .await
    .map_err(|e| e.to_string())??;
    // The original's date, camera and place go with the copy. A copy without
    // them is still worth saving, so a failure here is not the save's.
    let _ = app.photos().copy_metadata(id, &file, taken_ms).await;
    let saved = app.gallery().save(&file, &name).await;
    let _ = std::fs::remove_file(&file);
    saved
}

/// The saved copy's name: the original's, marked as edited, as a JPEG.
fn copy_name(original: &str) -> String {
    let stem = Path::new(original)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("photo");
    format!("{stem}-edit.jpg")
}

/// `editframe://localhost/<w>x<h>.<quality>.<stack>`: the open photo with
/// `stack` (base64url JSON) applied, fitted inside `w` × `h` device pixels,
/// as a JPEG. The whole edit travels in the name, so a frame never depends
/// on what was asked before it.
///
/// Zoomed in, `?k=<times>&cx=<x>&cy=<y>`: `k` times the size that fits,
/// around the spot (`cx`, `cy`) from 0 to 1 across the framed photo, as the
/// desktop's viewport draws it. Only the part that shows is rendered.
pub fn serve_frame(app: &tauri::AppHandle, path: &str, query: &str) -> Response<Vec<u8>> {
    let frame = parse_frame(path, query).and_then(|f| render_frame(app, &f));
    let builder = Response::builder()
        .header("Access-Control-Allow-Origin", "*")
        .header("Cache-Control", "no-store");
    match frame {
        Ok(jpeg) => builder.header("Content-Type", "image/jpeg").body(jpeg),
        Err(e) => builder.status(500).body(e.into_bytes()),
    }
    .expect("a response builds")
}

#[derive(Debug)]
struct FrameRequest {
    width: u32,
    height: u32,
    quality: u8,
    stack: EditStack,
    /// Zoomed in: times the size that fits, and the spot in the middle.
    view: Option<(f32, [f32; 2])>,
}

/// The view a frame's query asks for, if it zooms in at all.
fn parse_view(query: &str) -> Result<Option<(f32, [f32; 2])>, String> {
    let mut k = None;
    let mut centre = [0.5f32, 0.5];
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (name, value) = pair.split_once('=').ok_or("not a view")?;
        let value: f32 = value.parse().map_err(|_| "not a view")?;
        if !value.is_finite() {
            return Err("not a view".into());
        }
        match name {
            "k" => k = Some(value),
            "cx" => centre[0] = value.clamp(0.0, 1.0),
            "cy" => centre[1] = value.clamp(0.0, 1.0),
            _ => {}
        }
    }
    Ok(k.filter(|k| *k > 1.0).map(|k| (k.min(64.0), centre)))
}

fn parse_frame(path: &str, query: &str) -> Result<FrameRequest, String> {
    let mut parts = path.trim_start_matches('/').splitn(3, '.');
    let (size, quality, stack) = match (parts.next(), parts.next(), parts.next()) {
        (Some(s), Some(q), Some(st)) => (s, q, st),
        _ => return Err("not a frame".into()),
    };
    let (w, h) = size.split_once('x').ok_or("not a frame size")?;
    let json = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(stack)
        .map_err(|e| e.to_string())?;
    Ok(FrameRequest {
        width: w.parse::<u32>().map_err(|e| e.to_string())?.clamp(16, 8192),
        height: h.parse::<u32>().map_err(|e| e.to_string())?.clamp(16, 8192),
        quality: quality
            .parse::<u8>()
            .map_err(|e| e.to_string())?
            .clamp(40, 95),
        stack: serde_json::from_slice(&json).map_err(|e| e.to_string())?,
        view: parse_view(query)?,
    })
}

fn render_frame(app: &tauri::AppHandle, f: &FrameRequest) -> Result<Vec<u8>, String> {
    let editor = app.state::<Editor>();
    let mut inner = editor.inner.lock().unwrap();
    inner.use_look(&f.stack.lut.name);
    let (Some(engine), Some(open)) = (inner.engine.as_ref(), inner.open.as_ref()) else {
        return Err("No photo is open.".into());
    };
    let (fw, fh) = open.session.framed_size(&f.stack);
    if let Some((k, centre)) = f.view {
        let fit = (f.width as f32 / fw.max(1) as f32).min(f.height as f32 / fh.max(1) as f32);
        let port = ViewPort {
            zoom: (fit * k).min(MOST_ZOOM),
            centre,
            width: f.width,
            height: f.height,
        }
        .fitted((fw, fh));
        let rgba = engine
            .gpu
            .checked("rendering", || {
                engine.pipeline.render_view(
                    &engine.gpu,
                    &open.session,
                    &f.stack,
                    port.width,
                    port.height,
                    Some(&port),
                    false,
                )
            })
            .map_err(err)?;
        return encode(&rgba, port.width, port.height, f.quality, false);
    }
    let scale = (f.width as f64 / fw as f64)
        .min(f.height as f64 / fh as f64)
        .min(1.0);
    let edge = ((fw.max(fh) as f64 * scale).round() as u32).max(1);
    let (w, h) = open.session.framed_proxy(&f.stack, edge);
    let rgba = engine
        .gpu
        .checked("rendering", || {
            engine
                .pipeline
                .render_to_pixels(&engine.gpu, &open.session, &f.stack, w, h)
        })
        .map_err(err)?;
    encode(&rgba, w, h, f.quality, false)
}

/// A JPEG of `rgba`, straight from the RGBA the GPU gives back. Frames for
/// the screen keep colour at half resolution (4:2:0, as cameras save) and a
/// saved copy at full (4:4:4, `full_colour`).
///
/// On a Pixel 8 Pro, encoding was the slowest part of getting a frame on
/// screen: about 40 ms with image's encoder for a 1344 × 896 frame, against
/// under 30 ms to render it. This encoder takes about a quarter of that.
/// Sending raw pixels instead was slower still (docs/MOBILE.md, step 1).
fn encode(
    rgba: &[u8],
    width: u32,
    height: u32,
    quality: u8,
    full_colour: bool,
) -> Result<Vec<u8>, String> {
    let side =
        |v: u32| u16::try_from(v).map_err(|_| format!("a {v} px side is too long for a JPEG"));
    let mut out = Vec::new();
    let mut encoder = jpeg_encoder::Encoder::new(&mut out, quality);
    encoder.set_sampling_factor(if full_colour {
        jpeg_encoder::SamplingFactor::R_4_4_4
    } else {
        jpeg_encoder::SamplingFactor::F_2_2
    });
    encoder
        .encode(
            rgba,
            side(width)?,
            side(height)?,
            jpeg_encoder::ColorType::Rgba,
        )
        .map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(json: &str) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
    }

    #[test]
    fn a_frame_names_its_size_quality_and_edit() {
        let f = parse_frame(&format!("/1080x720.80.{}", b64(r#"{"exposure":0.5}"#)), "").unwrap();
        assert_eq!((f.width, f.height, f.quality), (1080, 720, 80));
        assert_eq!(f.stack.exposure, 0.5);
        assert_eq!(f.view, None);
        // `{}` is the unedited photo: every field at its default.
        let before = parse_frame(&format!("1080x720.80.{}", b64("{}")), "").unwrap();
        assert!(before.stack.is_default());
        // Out of range is held in range, not refused.
        let f = parse_frame(&format!("99999x1.5.{}", b64("{}")), "").unwrap();
        assert_eq!((f.width, f.height, f.quality), (8192, 16, 40));
        assert!(parse_frame("1080x720.80", "").is_err());
        assert!(parse_frame("big.80.e30", "").is_err());
        assert!(parse_frame(&format!("1080x720.80.{}", b64("not json")), "").is_err());
    }

    #[test]
    fn a_paste_brings_the_look_and_leaves_the_crop() {
        let mut copied = EditStack {
            exposure: 0.7,
            warmth: 0.2,
            ..Default::default()
        };
        copied.geometry.straighten = 5.0;
        copied.lut.name = "Portra".into();
        let mut onto = EditStack::default();
        onto.geometry.quarter_turns = 1;
        let pasted = edit_paste(copied, onto.clone());
        assert_eq!((pasted.exposure, pasted.warmth), (0.7, 0.2));
        assert_eq!(pasted.lut.name, "Portra");
        assert_eq!(pasted.geometry, onto.geometry);
    }

    #[test]
    fn auto_is_asked_for_by_name() {
        assert_eq!(
            serde_json::from_str::<Auto>(r#""tone""#).unwrap(),
            Auto::Tone
        );
        assert_eq!(serde_json::from_str::<Auto>(r#""wb""#).unwrap(), Auto::Wb);
        assert!(serde_json::from_str::<Auto>(r#""colour""#).is_err());
    }

    #[test]
    fn a_zoomed_frame_names_how_far_in_and_where() {
        assert_eq!(parse_view(""), Ok(None));
        assert_eq!(
            parse_view("k=2.5&cx=0.25&cy=0.75"),
            Ok(Some((2.5, [0.25, 0.75])))
        );
        // Fitting, or less, is no zoom at all.
        assert_eq!(parse_view("k=1&cx=0.2"), Ok(None));
        // The spot stays on the photo.
        assert_eq!(parse_view("k=3&cx=-1&cy=9"), Ok(Some((3.0, [0.0, 1.0]))));
        assert!(parse_view("k=lots").is_err());
        assert!(parse_view("k=NaN").is_err());
    }

    #[test]
    fn a_frame_encodes_as_a_jpeg_of_its_size() {
        // A small gradient, with an odd size as frames often have.
        let (w, h) = (37u32, 21u32);
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|i| [(i % 256) as u8, (i / 3 % 256) as u8, 128, 255])
            .collect();
        for full_colour in [false, true] {
            let jpeg = encode(&rgba, w, h, 80, full_colour).unwrap();
            let back = image::load_from_memory(&jpeg).unwrap();
            assert_eq!((back.width(), back.height()), (w, h));
        }
        assert!(encode(&rgba, 70_000, 1, 80, false).is_err());
    }

    #[test]
    fn a_saved_copy_is_named_after_the_original() {
        assert_eq!(
            copy_name("PXL_20261003_101500.jpg"),
            "PXL_20261003_101500-edit.jpg"
        );
        assert_eq!(copy_name("IMG_0001.HEIC"), "IMG_0001-edit.jpg");
        assert_eq!(copy_name(""), "photo-edit.jpg");
    }
}
