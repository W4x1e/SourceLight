//! The develop module's live state: one photo, resident on the GPU.
//!
//! Only one photo is developed at a time, so this is a single slot rather than
//! a cache. Each session holds a demosaiced image in VRAM — tens of megabytes —
//! and keeping several around to speed up a back-and-forth the user may never
//! make is a poor trade against exhausting VRAM on a large library.

use anyhow::{anyhow, Context, Result};

use crate::develop::erase::{self, Erase, Fill, FillStore};
use crate::develop::{EditStack, History};
use crate::gpu::pipeline::{DevelopSession, Pipeline, PROXY_EDGE};
use crate::gpu::Gpu;
use crate::present::{Frame, Presented, Presenter};
use crate::raw::decoder::RawDecoder;

/// How many photos' histories are kept for going back to.
const KEPT_HISTORIES: usize = 50;

/// GPU context plus whichever photo is open.
///
/// The GPU and pipeline are built on first use rather than at startup: someone
/// who only ever browses their library should not pay for device creation, and
/// a machine with no working adapter should still run the organiser.
pub struct DevelopState {
    gpu: Gpu,
    pipeline: Pipeline,
    /// Serves the `develop://` images: the preview, the before, the split.
    presenter: Box<dyn Presenter>,
    /// Draws the live preview straight into a surface beneath the webview,
    /// when direct rendering is on. Kept apart from `presenter` rather than
    /// replacing it, so the views the surface does not draw — the split, a
    /// thumbnail-sized look preview — still have their images, and the
    /// `develop://` handler, on a webview thread, never touches a swapchain.
    direct: Option<Box<dyn Presenter>>,
    open: Option<OpenPhoto>,
    /// Monotonic preview-frame counter, **shared across photos**. The frame
    /// number is a cache-buster in the preview URL, so it must never repeat for a
    /// different render — including when a new photo is opened. Resetting it per
    /// photo made switching to a freshly-opened photo reuse the previous photo's
    /// URL, and the webview served the stale cached image (the filmstrip looked
    /// dead).
    frame_seq: u32,
    /// The histories of photos left this session, so going back to one
    /// keeps its undo steps rather than starting again at "Open".
    /// Most recent last; bounded by [`KEPT_HISTORIES`].
    left: Vec<(i32, History)>,
    /// The edit being dragged, not yet committed, for the `develop://`
    /// preview to draw while the drag lasts, and a counter that names
    /// each one in the preview URL. Cleared when another photo opens.
    draft: Option<EditStack>,
    draft_seq: u32,
    /// Where imported `.cube` LUTs live (`<app data>/luts`). Set by the app layer.
    luts_dir: Option<std::path::PathBuf>,
    /// Resampled LUT texels by filename, so a LUT is parsed and resampled once
    /// per session, not per frame. `None` caches a load/parse failure so a
    /// missing or broken file is not retried every render.
    lut_cache: std::collections::HashMap<String, Option<std::sync::Arc<Vec<f32>>>>,
    /// Where magic erase's fills are kept (`<app data>/patches`). Set by the
    /// app layer.
    fills: Option<FillStore>,
    /// The open photo's fills, read once each.
    fill_cache: std::collections::HashMap<String, std::sync::Arc<Fill>>,
    /// SAM2 models plus the embedding of the photo they were run against. The
    /// encoder is the expensive half and depends only on the image, so it is
    /// computed once and reused for every subsequent click — that is what makes
    /// click-to-select feel interactive rather than a two-second wait per point.
    #[cfg(feature = "ai")]
    sam2: Option<Sam2Cache>,
}

#[cfg(feature = "ai")]
struct Sam2Cache {
    photo_id: i32,
    /// The framing the embedding was made from: after a crop, turn
    /// or flip, a click decoded against the old framing put the mask in the
    /// wrong place.
    geometry: crate::develop::geometry::Geometry,
    /// The erases the embedding saw: one made since would let a click
    /// select what is no longer there.
    erases: Vec<Erase>,
    models: crate::develop::sam2::Sam2,
    embedding: crate::develop::sam2::Embedding,
}

struct OpenPhoto {
    photo_id: i32,
    session: DevelopSession,
    history: History,
    /// The edit this photo had in the catalogue when it was opened (or when a
    /// virtual copy took the session over). Develop saves as it goes, so the
    /// catalogue no longer remembers it — this is what "Save as copy" puts the
    /// original back to.
    baseline: EditStack,
    /// Incremented on every change. The UI puts it in the preview URL so the
    /// webview treats each frame as a distinct resource rather than serving a
    /// cached one.
    frame: u32,
    /// AI noise reduction's result, once it has run on this photo.
    denoise: Option<Denoised>,
    /// The erases the GPU session's linear image holds now.
    erased: Vec<Erase>,
    /// Counts the writes to the GPU session's linear image, so an erase
    /// worked out while it was rewritten (noise reduction's Amount moved, say)
    /// is laid in afresh rather than put up as it was read.
    written: u64,
}

/// The open photo rendered for an AI selection to work on outside the
/// develop lock ([`DevelopState::selection_input`]).
pub struct SelectionInput {
    /// Which photo it is, and how it was framed, to check they still hold
    /// when the result is back ([`DevelopState::still_fits`]).
    pub photo_id: i32,
    geometry: crate::develop::geometry::Geometry,
    /// The erases it shows, which click to select's embedding is kept for.
    #[cfg_attr(not(feature = "ai"), allow(dead_code))]
    erases: Vec<Erase>,
    /// RGBA, 8 bits a channel.
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The open photo's linear image, handed out for AI noise reduction to work
/// on outside the develop lock ([`DevelopState::linear_for_denoise`]).
pub struct LinearImage {
    /// Which photo it is, to check it is still open when the result is back.
    pub photo_id: i32,
    /// RGBA half floats (their bits), in the texture's own order.
    pub pixels: Vec<u16>,
    pub width: u32,
    pub height: u32,
    /// The centre of the view, as fractions of the stored image, when the
    /// viewer asked for one: AI noise reduction starts there.
    pub focus: Option<(f32, f32)>,
}

/// What a magic erase is worked out from, outside the develop lock
/// ([`DevelopState::erase_input`]): where the model will look, and the
/// linear image there as it is now, earlier erases and all.
pub struct EraseInput {
    /// Which photo it is, and the erases already on it, to check they still
    /// hold when the fills are back ([`DevelopState::attach_erase`]).
    pub photo_id: i32,
    erases: Vec<Erase>,
    /// The linear image's [`OpenPhoto::written`] when the region was read.
    written: u64,
    pub plan: erase::Plan,
    pub region: erase::Region,
}

/// The linear image before and after AI noise reduction, so the Amount
/// re-blends the two without running the model again. Held for the open photo
/// only: at 24 MP each is 190 MB of half floats.
struct Denoised {
    original: Vec<u16>,
    denoised: Vec<u16>,
    /// The Amount the GPU session's linear image holds now.
    applied: f32,
}

/// Number of buckets per channel in the preview histogram.
pub const HISTOGRAM_BINS: usize = 64;

/// An RGB histogram of the current render, plus clipping fractions.
///
/// Computed in Rust because the preview reaches the webview over a custom URI
/// scheme, and reading that image's pixels back through a `<canvas>` either
/// fails the cross-origin load or taints the canvas — so the browser cannot
/// build this itself. A small proxy render here is cheap and always readable.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Histogram {
    pub r: Vec<u32>,
    pub g: Vec<u32>,
    pub b: Vec<u32>,
    /// Percent of pixels with a fully-crushed channel, and a fully-blown one.
    pub shadow_clip: f32,
    pub highlight_clip: f32,
}

/// What the UI needs to lay out the develop view.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DevelopInfo {
    pub photo_id: i32,
    /// Full developed size, before the preview is fitted to the window.
    pub width: u32,
    pub height: u32,
    pub stack: EditStack,
    pub frame: u32,
    pub can_undo: bool,
    pub can_redo: bool,
    /// Every step in this photo's history, oldest first, and which one
    /// is applied.
    pub history: Vec<String>,
    pub history_at: u32,
    /// False for a JPEG, PNG or TIFF, which has far less to recover in
    /// the highlights and shadows than a raw. The panel says so.
    pub from_raw: bool,
    /// AI noise reduction has run on this photo this session, so its
    /// Amount takes effect as it moves. A stack with an Amount but no result
    /// yet shows the photo undenoised until it is run again.
    pub denoise_ready: bool,
}

pub use crate::develop::eyedropper::neutral_point;

/// `rect` of the open photo's linear image as it is before any erase: the
/// AI noise reduction blend when it has run, or the image as decoded.
fn base_region(
    pipeline: &Pipeline,
    gpu: &Gpu,
    open: &OpenPhoto,
    rect: erase::Rect,
) -> Result<Vec<u16>> {
    let Some(d) = open.denoise.as_ref() else {
        return pipeline.read_linear_region(gpu, &open.session, rect.as_region(), true);
    };
    let (w, _) = open.session.stored_size();
    let (x, y, rw, rh) = rect.as_region();
    // Nothing is blended until the first refresh after a run.
    let amount = if d.applied.is_finite() {
        d.applied
    } else {
        0.0
    };
    let mut out = Vec::with_capacity((rw * rh * 4) as usize);
    for row in y..y + rh {
        let at = ((row * w + x) * 4) as usize;
        let span = at..at + (rw * 4) as usize;
        out.extend(crate::denoise::blend(
            &d.original[span.clone()],
            &d.denoised[span],
            amount,
        ));
    }
    Ok(out)
}

/// Encodes an RGBA8 buffer as a `data:image/jpeg;base64,…` URL — how a suggestion
/// preview reaches the webview inline (small, so base64 in JSON is fine, and it
/// sidesteps needing another custom-scheme round trip).
fn jpeg_data_url(rgba: &[u8], w: u32, h: u32) -> Result<String> {
    use base64::Engine;
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec())
        .ok_or_else(|| anyhow!("preview buffer size mismatch"))?;
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .to_rgb8()
        .write_to(&mut buf, image::ImageFormat::Jpeg)
        .context("encoding preview jpeg")?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(buf.get_ref());
    Ok(format!("data:image/jpeg;base64,{b64}"))
}

impl DevelopState {
    pub fn new(presenter: Box<dyn Presenter>) -> Result<Self> {
        let gpu = crate::gpu::shared().context("initialising the GPU for develop")?;
        let pipeline = Pipeline::new(&gpu);
        Ok(Self {
            gpu,
            pipeline,
            presenter,
            direct: None,
            open: None,
            frame_seq: 0,
            left: Vec::new(),
            draft: None,
            draft_seq: 0,
            luts_dir: None,
            lut_cache: std::collections::HashMap::new(),
            fills: None,
            fill_cache: std::collections::HashMap::new(),
            #[cfg(feature = "ai")]
            sam2: None,
        })
    }

    /// Tells the session where magic erase's fills are kept. Called once by
    /// the app layer before a photo is developed.
    pub fn set_fills_dir(&mut self, dir: std::path::PathBuf) {
        self.fills = Some(FillStore::new(dir));
    }

    /// Tells the session where imported LUTs live. Called once by the app layer
    /// before a photo is developed.
    pub fn set_luts_dir(&mut self, dir: std::path::PathBuf) {
        if self.luts_dir.as_deref() != Some(dir.as_path()) {
            self.luts_dir = Some(dir);
            self.lut_cache.clear();
        }
    }

    /// Resolves the current stack's LUT to texels and stores them on the GPU
    /// session, so the next render binds the right look. A no-op (clears the LUT)
    /// when nothing is selected, the file is missing, or the directory is unset.
    /// Cheap after the first resolve of a given name, since it is cached.
    fn refresh_lut(&mut self) {
        let name = match &self.open {
            Some(o) => o.history.current().lut.name.clone(),
            None => return,
        };
        let texels = self.resolve_lut(&name);
        if let Some(open) = self.open.as_mut() {
            open.session.lut_texels = texels.map(|arc| (*arc).clone());
        }
    }

    /// Brings the GPU session in line with the applied stack: its LUT, the
    /// AI noise reduction blend, and the erases.
    fn refresh(&mut self) {
        self.refresh_lut();
        self.refresh_denoise();
        self.refresh_erases();
    }

    /// Re-blends AI noise reduction at the stack's Amount and uploads it, if
    /// it has run on this photo and the Amount has moved. Undo, redo and the
    /// history panel come through here as well as the slider. The erases are
    /// laid into the blend before it goes up.
    fn refresh_denoise(&mut self) {
        let Some(open) = self.open.as_ref() else {
            return;
        };
        let current = open.history.current();
        let (amount, erases) = (current.detail.ai_denoise, current.erases.clone());
        if open.denoise.as_ref().is_none_or(|d| d.applied == amount) {
            return;
        }
        self.load_fills(&erases);
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let Some(d) = open.denoise.as_mut() else {
            return;
        };
        let mut blended = crate::denoise::blend(&d.original, &d.denoised, amount);
        let size = open.session.stored_size();
        let cache = &self.fill_cache;
        erase::bake(&mut blended, size, &erases, |name| cache.get(name).cloned());
        match self
            .pipeline
            .write_linear(&self.gpu, &open.session, &blended)
        {
            Ok(()) => {
                d.applied = amount;
                open.erased = erases;
                open.written += 1;
            }
            Err(e) => eprintln!("denoise: could not update the image: {e}"),
        }
    }

    /// Reads every fill `erases` use that is not read yet. One that cannot be
    /// read is said so in the log and left out: its object shows again.
    fn load_fills(&mut self, erases: &[Erase]) {
        let Some(store) = self.fills.as_ref() else {
            if !erases.is_empty() {
                eprintln!("erase: no folder for fills; the photo shows without its erases");
            }
            return;
        };
        for name in erase::fill_names(erases) {
            if self.fill_cache.contains_key(name) {
                continue;
            }
            match store.load(name) {
                Ok(fill) => {
                    self.fill_cache
                        .insert(name.to_string(), std::sync::Arc::new(fill));
                }
                Err(e) => eprintln!("erase: {e:#}"),
            }
        }
    }

    /// Lays the stack's erases into the GPU session's linear image, if they
    /// have changed: only the windows that differ, grown to whole windows
    /// over what was there before any erase ([`erase::dirty_regions`]).
    fn refresh_erases(&mut self) {
        let Some(open) = self.open.as_ref() else {
            return;
        };
        let erases = open.history.current().erases.clone();
        if erases == open.erased {
            return;
        }
        self.load_fills(&erases);
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if open.denoise.is_none() {
            // "Before" keeps showing the photo as decoded, and it is what
            // every erase is laid onto.
            self.pipeline.keep_original(&self.gpu, &mut open.session);
        }
        let size = open.session.stored_size();
        let mut wrote = false;
        for rect in erase::dirty_regions(&open.erased, &erases, size) {
            let base = match base_region(&self.pipeline, &self.gpu, open, rect) {
                Ok(base) => base,
                Err(e) => {
                    eprintln!("erase: could not read the photo: {e:#}");
                    continue;
                }
            };
            let mut region = erase::Region { rect, rgba: base };
            let cache = &self.fill_cache;
            erase::lay_in(&mut region, &erases, |name| cache.get(name).cloned());
            match self.pipeline.write_linear_region(
                &self.gpu,
                &open.session,
                rect.as_region(),
                &region.rgba,
            ) {
                Ok(()) => wrote = true,
                Err(e) => eprintln!("erase: could not update the image: {e:#}"),
            }
        }
        if wrote {
            self.pipeline.rebuild_mips(&self.gpu, &open.session);
            open.written += 1;
        }
        open.erased = erases;
    }

    /// The open photo's linear image, for AI noise reduction to work on
    /// outside the develop lock. `None` when it has already run on this photo.
    /// `focus`, in framed UV, is where the viewer is looking. Without the
    /// erases: they are laid onto the result.
    pub fn linear_for_denoise(&self, focus: Option<(f32, f32)>) -> Result<Option<LinearImage>> {
        let open = self.photo()?;
        if open.denoise.is_some() {
            return Ok(None);
        }
        let (w, h) = open.session.stored_size();
        let pixels =
            self.pipeline
                .read_linear_region(&self.gpu, &open.session, (0, 0, w, h), true)?;
        Ok(Some(LinearImage {
            photo_id: open.photo_id,
            pixels,
            width: w,
            height: h,
            focus: focus.map(|f| open.session.stored_uv(open.history.current(), f)),
        }))
    }

    /// Shows AI noise reduction's result so far while it runs (what it has not
    /// reached yet is the original), if `photo_id` is still open and the run
    /// has not finished. Not a step in history and not kept: the finished
    /// result replaces it, or [`Self::restore_linear`] puts the original back.
    /// The new frame number when it was shown.
    pub fn show_denoise_preview(&mut self, photo_id: i32, pixels: &[u16]) -> Result<Option<u32>> {
        self.replace_linear(photo_id, pixels)
    }

    /// Puts the original linear image back after a run that showed its
    /// progress was cancelled or failed.
    pub fn restore_linear(&mut self, photo_id: i32, original: &[u16]) -> Result<Option<u32>> {
        self.replace_linear(photo_id, original)
    }

    fn replace_linear(&mut self, photo_id: i32, pixels: &[u16]) -> Result<Option<u32>> {
        let frame = self.next_frame();
        let erases = match self.open.as_ref() {
            Some(o) if o.photo_id == photo_id && o.denoise.is_none() => {
                o.history.current().erases.clone()
            }
            _ => return Ok(None),
        };
        self.load_fills(&erases);
        let Some(open) = self.open.as_mut() else {
            return Ok(None);
        };
        self.pipeline.keep_original(&self.gpu, &mut open.session);
        if erases.is_empty() {
            self.pipeline
                .write_linear(&self.gpu, &open.session, pixels)?;
        } else {
            let mut pixels = pixels.to_vec();
            let cache = &self.fill_cache;
            let size = open.session.stored_size();
            erase::bake(&mut pixels, size, &erases, |name| cache.get(name).cloned());
            self.pipeline
                .write_linear(&self.gpu, &open.session, &pixels)?;
        }
        open.erased = erases;
        open.written += 1;
        open.frame = frame;
        Ok(Some(frame))
    }

    /// Takes AI noise reduction's result for `photo_id`. With an `amount`, sets
    /// the stack's Amount as a step in history (the Denoise button); without,
    /// keeps the Amount the stack already has (an edit reopened, or pasted).
    pub fn attach_denoise(
        &mut self,
        photo_id: i32,
        original: Vec<u16>,
        denoised: Vec<u16>,
        amount: Option<f32>,
    ) -> Result<DevelopInfo> {
        let frame = self.next_frame();
        {
            let open = self
                .open
                .as_mut()
                .filter(|o| o.photo_id == photo_id)
                .ok_or_else(|| anyhow!("the photo was closed before noise reduction finished"))?;
            // "Before" keeps showing the photo as decoded.
            self.pipeline.keep_original(&self.gpu, &mut open.session);
            open.denoise = Some(Denoised {
                original,
                denoised,
                // Nothing blended yet: the refresh below writes the first.
                applied: f32::NAN,
            });
            if let Some(amount) = amount {
                let mut stack = open.history.current().clone();
                stack.detail.ai_denoise = amount;
                open.history.push("AI noise reduction", stack);
            }
            open.frame = frame;
        }
        self.refresh();
        self.info()
    }

    /// Returns the resampled texels for a LUT filename, loading and caching on
    /// first use. Empty name, no dir, or a failed load all yield `None`.
    fn resolve_lut(&mut self, name: &str) -> Option<std::sync::Arc<Vec<f32>>> {
        if name.is_empty() {
            return None;
        }
        if let Some(cached) = self.lut_cache.get(name) {
            return cached.clone();
        }
        let dir = self.luts_dir.clone()?;
        let loaded = crate::develop::lut::load_texels(&dir, name).map(std::sync::Arc::new);
        self.lut_cache.insert(name.to_string(), loaded.clone());
        loaded
    }

    /// The next never-before-used preview-frame number.
    fn next_frame(&mut self) -> u32 {
        self.frame_seq = self.frame_seq.wrapping_add(1);
        self.frame_seq
    }

    /// Turns the direct surface on (`Some`) or off, keeping the open photo
    /// and its history.
    ///
    /// Switching is a *display* change: the GPU session and edit stack are
    /// untouched, so toggling back and forth costs nothing and loses nothing.
    /// Dropping the presenter tears its surface down.
    pub fn set_direct(&mut self, direct: Option<Box<dyn Presenter>>) {
        self.direct = direct;
    }

    /// Takes the direct surface out, for the caller to drop outside the
    /// develop lock.
    pub fn take_direct(&mut self) -> Option<Box<dyn Presenter>> {
        self.direct.take()
    }

    pub fn has_direct(&self) -> bool {
        self.direct.is_some()
    }

    /// Tells the direct surface where the canvas is, in physical pixels.
    pub fn place_target(&mut self, x: i32, y: i32, width: u32, height: u32) -> Result<()> {
        let gpu = &self.gpu;
        let direct = self
            .direct
            .as_mut()
            .ok_or_else(|| anyhow!("direct rendering is off"))?;
        direct.place(gpu, x, y, width, height);
        Ok(())
    }

    /// Draws one flat diagnostic frame. Needs no photo open — that is the point.
    pub fn probe_target(&mut self) -> Result<String> {
        let gpu = &self.gpu;
        let direct = self
            .direct
            .as_mut()
            .ok_or_else(|| anyhow!("direct rendering is off"))?;
        direct.probe(gpu)
    }

    pub fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    pub fn pipeline(&self) -> &Pipeline {
        &self.pipeline
    }

    pub fn describe(&self) -> String {
        match &self.direct {
            Some(direct) => format!("{} via {}", self.gpu.info.name, direct.describe()),
            None => format!("{} via {}", self.gpu.info.name, self.presenter.describe()),
        }
    }

    /// Decodes a photo — raw or rendered — and uploads it, replacing whatever
    /// was open.
    pub fn open(
        &mut self,
        photo_id: i32,
        path: &std::path::Path,
        decoder: &dyn RawDecoder,
        stored: Option<EditStack>,
    ) -> Result<DevelopInfo> {
        let source = crate::raster::load(path, decoder)?;
        self.open_source(photo_id, &source, stored)
    }

    /// [`Self::open`] for a photo already decoded: the decode is the slow
    /// half, and it needs neither the GPU nor this state, so the app does it
    /// before taking the develop lock.
    pub fn open_source(
        &mut self,
        photo_id: i32,
        source: &crate::raster::Source,
        stored: Option<EditStack>,
    ) -> Result<DevelopInfo> {
        let session = self.pipeline.prepare_source(&self.gpu, source)?;

        let frame = self.next_frame();
        let stored = stored.unwrap_or_default();
        self.leave();
        // Its history from earlier this session, if the edit is still where
        // that left it; anything else changed it since, and starts afresh.
        let history = match self.left.iter().position(|(id, _)| *id == photo_id) {
            Some(i) if self.left[i].1.current() == &stored => self.left.remove(i).1,
            _ => History::new(stored.clone()),
        };
        // Another photo's drag is not this one's, nor its fills.
        self.draft = None;
        self.fill_cache.clear();
        self.open = Some(OpenPhoto {
            photo_id,
            session,
            history,
            baseline: stored,
            frame,
            denoise: None,
            erased: Vec::new(),
            written: 0,
        });
        self.refresh();
        self.info()
    }

    pub fn close(&mut self) {
        self.leave();
        self.draft = None;
        self.fill_cache.clear();
        // Click to select's models go with Develop: kept, they stayed
        // in memory for the rest of the session.
        #[cfg(feature = "ai")]
        {
            self.sam2 = None;
        }
    }

    /// Lets go of the open photo, keeping its history for a return to it.
    fn leave(&mut self) {
        if let Some(open) = self.open.take() {
            self.left.retain(|(id, _)| *id != open.photo_id);
            self.left.push((open.photo_id, open.history));
            if self.left.len() > KEPT_HISTORIES {
                self.left.remove(0);
            }
        }
    }

    /// Holds the edit being dragged for the preview to draw; `None`
    /// once it is committed or abandoned. Returns the number the preview URL
    /// asks for it by, new for every draft so no cached frame is reused.
    pub fn set_draft(&mut self, draft: Option<EditStack>) -> u32 {
        // A crop or straighten mid-drag draws its masks and spots on the
        // content too, as the commit will place them — not left in the
        // frame until release and then snapping across.
        let draft = draft.map(|mut d| {
            if let Some(open) = self.open.as_ref() {
                let now = open.history.current();
                if d.geometry != now.geometry && d.masks == now.masks && d.spots == now.spots {
                    crate::develop::reframe::reframe(
                        &mut d,
                        &now.geometry,
                        open.session.width,
                        open.session.height,
                    );
                }
            }
            d
        });
        self.draft = draft;
        self.draft_seq = self.draft_seq.wrapping_add(1);
        self.draft_seq
    }

    /// The edit being dragged, if one is held.
    pub fn draft(&self) -> Option<&EditStack> {
        self.draft.as_ref()
    }

    fn photo(&self) -> Result<&OpenPhoto> {
        self.open
            .as_ref()
            .ok_or_else(|| anyhow!("no photo is open"))
    }

    pub fn info(&self) -> Result<DevelopInfo> {
        let open = self.photo()?;
        let stack = open.history.current();
        // The framed size, not the sensor's: the UI lays the canvas out against
        // this, so reporting the uncropped size would letterbox every cropped
        // photo into the wrong aspect ratio.
        let (width, height) = open.session.framed_size(stack);
        Ok(DevelopInfo {
            photo_id: open.photo_id,
            width,
            height,
            stack: open.history.current().clone(),
            frame: open.frame,
            can_undo: open.history.can_undo(),
            can_redo: open.history.can_redo(),
            history: open.history.labels().0,
            history_at: open.history.labels().1 as u32,
            from_raw: open.session.from_raw,
            denoise_ready: open.denoise.is_some(),
        })
    }

    pub fn current_stack(&self) -> Result<EditStack> {
        Ok(self.photo()?.history.current().clone())
    }

    /// The GPU context and pipeline, for rendering a photo *other* than the
    /// open one without disturbing it — the zoom loupe's full-resolution source
    /// (`full_preview::render`). Reused rather than built per request, because
    /// creating a device and compiling the develop shader costs more than the
    /// render itself.
    pub fn gpu_and_pipeline(&self) -> (&Gpu, &Pipeline) {
        (&self.gpu, &self.pipeline)
    }

    pub fn open_photo_id(&self) -> Option<i32> {
        self.open.as_ref().map(|o| o.photo_id)
    }

    /// The edit the open photo had when it was opened — see
    /// [`OpenPhoto::baseline`].
    pub fn baseline_stack(&self) -> Result<EditStack> {
        Ok(self.photo()?.baseline.clone())
    }

    /// Points the open session at another catalogue row showing the same raw
    /// file — a virtual copy just made from it — so the edits that follow land
    /// on the copy. Nothing is decoded again: a virtual copy is the same file.
    /// Undo history carries over; the copy's baseline is what it was made with.
    pub fn rebind(&mut self, photo_id: i32) -> Result<DevelopInfo> {
        let open = self
            .open
            .as_mut()
            .ok_or_else(|| anyhow!("no photo is open"))?;
        open.photo_id = photo_id;
        open.baseline = open.history.current().clone();
        self.info()
    }

    /// Applies a new stack and bumps the frame counter.
    ///
    /// `label` groups an interaction in history. Dragging a slider produces a
    /// stream of these, and `History::push` collapses repeats, but consecutive
    /// distinct values still each land — coalescing a drag into one entry is a
    /// UI concern and belongs where the drag ends, not here.
    pub fn apply(&mut self, label: &str, stack: EditStack) -> Result<DevelopInfo> {
        let frame = self.next_frame();
        {
            let open = self
                .open
                .as_mut()
                .ok_or_else(|| anyhow!("no photo is open"))?;
            open.history.push(label, stack);
            open.frame = frame;
        }
        self.refresh();
        self.info()
    }

    /// [`Self::apply`] for a change of framing: the stack's masks and
    /// spots, placed in the frame the current edit gives, are carried to where
    /// the same content lies in the frame `stack.geometry` gives.
    pub fn reframe(&mut self, label: &str, mut stack: EditStack) -> Result<DevelopInfo> {
        {
            let open = self
                .open
                .as_ref()
                .ok_or_else(|| anyhow!("no photo is open"))?;
            let from = open.history.current().geometry;
            crate::develop::reframe::reframe(
                &mut stack,
                &from,
                open.session.width,
                open.session.height,
            );
        }
        self.apply(label, stack)
    }

    pub fn undo(&mut self) -> Result<DevelopInfo> {
        let frame = self.next_frame();
        {
            let open = self
                .open
                .as_mut()
                .ok_or_else(|| anyhow!("no photo is open"))?;
            open.history.undo();
            open.frame = frame;
        }
        self.refresh();
        self.info()
    }

    /// Jumps to a step in the history panel.
    pub fn go_to(&mut self, index: usize) -> Result<DevelopInfo> {
        let frame = self.next_frame();
        {
            let open = self
                .open
                .as_mut()
                .ok_or_else(|| anyhow!("no photo is open"))?;
            open.history.go_to(index);
            open.frame = frame;
        }
        self.refresh();
        self.info()
    }

    pub fn redo(&mut self) -> Result<DevelopInfo> {
        let frame = self.next_frame();
        {
            let open = self
                .open
                .as_mut()
                .ok_or_else(|| anyhow!("no photo is open"))?;
            open.history.redo();
            open.frame = frame;
        }
        self.refresh();
        self.info()
    }

    /// An RGB histogram of the current edit, from a small proxy render.
    ///
    /// Deliberately of the *whole framed image*, not the zoomed view — a
    /// histogram is a property of the photograph, so panning or zooming must not
    /// change it. Uses the same `render_to_pixels` the preview and export do, at
    /// a small size, since distribution needs samples not resolution.
    ///
    /// `draft` is an edit mid-drag, not yet in history, so the histogram can
    /// follow a slider as the preview does.
    pub fn histogram(&self, draft: Option<&EditStack>) -> Result<Histogram> {
        let open = self.photo()?;
        let stack = draft.unwrap_or_else(|| open.history.current());
        let (w, h) = open.session.framed_proxy(stack, 320);
        let pixels = self
            .pipeline
            .render_to_pixels(&self.gpu, &open.session, stack, w, h)?;

        let mut r = vec![0u32; HISTOGRAM_BINS];
        let mut g = vec![0u32; HISTOGRAM_BINS];
        let mut b = vec![0u32; HISTOGRAM_BINS];
        let (mut shadow, mut high) = (0u32, 0u32);
        let n = (pixels.len() / 4).max(1) as f32;
        let last = HISTOGRAM_BINS - 1;
        for px in pixels.chunks_exact(4) {
            let (rr, gg, bb) = (px[0], px[1], px[2]);
            r[rr as usize * last / 255] += 1;
            g[gg as usize * last / 255] += 1;
            b[bb as usize * last / 255] += 1;
            let max = rr.max(gg).max(bb);
            let min = rr.min(gg).min(bb);
            if max >= 252 {
                high += 1;
            }
            if min <= 3 {
                shadow += 1;
            }
        }
        Ok(Histogram {
            r,
            g,
            b,
            shadow_clip: shadow as f32 / n * 100.0,
            highlight_clip: high as f32 / n * 100.0,
        })
    }

    /// Auto-tone: measure the scene and set exposure/whites/blacks/highlights/
    /// shadows to a balanced starting point, as one undoable step ("Auto Tone").
    ///
    /// Measures a *tone-neutral* render of the current edit (see
    /// [`auto::AutoTone::neutral_measurement`]) so pressing Auto is idempotent —
    /// it reads the scene as shaped by the user's colour/curve choices but not by
    /// any previous Auto — then writes the result onto the *current* stack,
    /// leaving everything else (colour, crop, masks, curves) intact.
    pub fn auto_tone(&mut self) -> Result<DevelopInfo> {
        use crate::develop::auto::{self, AutoTone};
        // Measure and compute inside one borrow (mirroring `histogram`); the
        // owned `target` outlives it so `apply` can take `&mut self` after.
        let target = {
            let open = self.photo()?;
            let current = open.history.current();
            let measurement = AutoTone::neutral_measurement(current);
            let (w, h) = open.session.framed_proxy(&measurement, 320);
            let pixels =
                self.pipeline
                    .render_to_pixels(&self.gpu, &open.session, &measurement, w, h)?;
            let stats = auto::stats_from_pixels(&pixels);
            let mut target = current.clone();
            auto::auto_tone(&stats).apply_to(&mut target);
            target
        };
        self.apply("Auto Tone", target)
    }

    /// Full auto: tone **and** white balance in one undoable step ("Auto").
    ///
    /// Measures a single render with both tone and WB neutralised, computes each
    /// from the same pixels, and applies both — so one click gives a balanced
    /// exposure *and* a corrected cast. Idempotent for the same reason the
    /// individual autos are.
    pub fn auto_all(&mut self) -> Result<DevelopInfo> {
        let target = self.compute_auto_all()?;
        self.apply("Auto", target)
    }

    /// Computes the full-auto (tone + WB) stack over the current edit *without*
    /// committing it — the measurement-and-solve half of [`Self::auto_all`],
    /// shared with the suggestion previews so both see identical results.
    pub fn compute_auto_all(&self) -> Result<EditStack> {
        use crate::develop::auto::{self, AutoTone, AutoWb};
        let open = self.photo()?;
        let current = open.history.current();
        // Neutralise both tone and WB so the measurement is the base scene.
        let measurement = AutoWb::neutral_measurement(&AutoTone::neutral_measurement(current));
        let (w, h) = open.session.framed_proxy(&measurement, 320);
        let pixels =
            self.pipeline
                .render_to_pixels(&self.gpu, &open.session, &measurement, w, h)?;
        let mut target = current.clone();
        auto::auto_tone(&auto::stats_from_pixels(&pixels)).apply_to(&mut target);
        auto::auto_wb(&auto::wb_stats_from_pixels(&pixels)).apply_to(&mut target);
        Ok(target)
    }

    /// Renders live preview chips for suggested edits: the auto baseline plus the
    /// given built-in presets, each applied over the current edit and rendered to
    /// a small JPEG data URL. Structural fields (crop, masks) are preserved, so a
    /// preview looks like what applying it would actually produce.
    pub fn suggest_edits(
        &self,
        preset_ids: &[&str],
    ) -> Result<Vec<crate::develop::suggest::Suggestion>> {
        let base = self.photo()?.history.current().clone();
        let mut out = Vec::new();

        // The auto baseline first — the safe, always-relevant option.
        if let Ok(auto) = self.compute_auto_all() {
            out.push(self.render_suggestion("auto", "auto", "Auto", &auto)?);
        }
        for id in preset_ids {
            if let Some(stack) = crate::develop::presets::apply(id, 1.0, &base) {
                let name = crate::develop::presets::name(id).unwrap_or(id);
                out.push(self.render_suggestion("preset", id, name, &stack)?);
            }
        }
        Ok(out)
    }

    /// Renders a small JPEG preview of `stack` applied to the open photo, as a
    /// `data:image/jpeg;base64,…` URL. Shared by the suggestion chips and the
    /// Styles / Presets galleries, which all want the same thumbnail of a look
    /// on *this* photo before it is committed.
    pub fn render_look_preview(&self, stack: &EditStack) -> Result<String> {
        self.render_preview(stack, 200)
    }

    /// [`Self::render_look_preview`] with its long edge at `edge` pixels: the
    /// editing server shows an AI model a frame big enough to judge.
    pub fn render_preview(&self, stack: &EditStack, edge: u32) -> Result<String> {
        let open = self.photo()?;
        let (w, h) = open.session.framed_proxy(stack, edge);
        let pixels = self
            .pipeline
            .render_to_pixels(&self.gpu, &open.session, stack, w, h)?;
        jpeg_data_url(&pixels, w, h)
    }

    /// Renders one suggestion's small preview and packages it.
    fn render_suggestion(
        &self,
        kind: &str,
        id: &str,
        name: &str,
        stack: &EditStack,
    ) -> Result<crate::develop::suggest::Suggestion> {
        Ok(crate::develop::suggest::Suggestion {
            kind: kind.to_string(),
            id: id.to_string(),
            name: name.to_string(),
            preview: self.render_look_preview(stack)?,
        })
    }

    /// Live thumbnails of the given built-in presets on the open photo, each at
    /// full amount over the current edit — each id paired with its preview.
    /// Asked for a group at a time, as the panel opens one: every preview is a
    /// render. Unknown ids are skipped.
    pub fn preset_previews(&self, ids: &[String]) -> Result<Vec<(String, String)>> {
        let base = self.photo()?.history.current().clone();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(stack) = crate::develop::presets::apply(id, 1.0, &base) {
                out.push((id.clone(), self.render_look_preview(&stack)?));
            }
        }
        Ok(out)
    }

    /// Live thumbnails of every installed creative LUT applied to the open photo —
    /// each LUT's filename paired with its rendered preview, the LUTs gallery's
    /// equivalent of [`Self::preset_previews`]. Each LUT's texels are swapped onto
    /// the session in turn, the current edit is re-rendered with the LUT at full
    /// strength, and the previously-bound LUT is restored at the end.
    pub fn lut_previews(&mut self, names: &[String]) -> Result<Vec<(String, String)>> {
        // Resolve every LUT to texels first: `resolve_lut` mutates the resample
        // cache, so it cannot run while a render borrows the session.
        let mut resolved: Vec<(String, Option<std::sync::Arc<Vec<f32>>>)> =
            Vec::with_capacity(names.len());
        for name in names {
            let texels = self.resolve_lut(name);
            resolved.push((name.clone(), texels));
        }

        // Preview each LUT over the current edit, at full intensity, and remember
        // the texels currently bound so the open photo is left exactly as it was.
        let base = self.photo()?.history.current().clone();
        let saved = self
            .open
            .as_ref()
            .and_then(|o| o.session.lut_texels.clone());

        let mut out = Vec::with_capacity(resolved.len());
        for (name, texels) in resolved {
            let mut stack = base.clone();
            stack.lut = crate::develop::lut::LutRef {
                name: name.clone(),
                intensity: 1.0,
            };
            // Bind this LUT's texels (or none, for one that failed to load — it
            // then previews as the plain edit rather than aborting the gallery).
            if let Some(open) = self.open.as_mut() {
                open.session.lut_texels = texels.as_ref().map(|t| (**t).clone());
            }
            out.push((name, self.render_look_preview(&stack)?));
        }

        if let Some(open) = self.open.as_mut() {
            open.session.lut_texels = saved;
        }
        Ok(out)
    }

    /// Auto white balance: estimate the illuminant cast from the frame's
    /// near-neutral mid-tones (gray-world) and set warmth/tint to counter it, as
    /// one undoable step ("Auto WB").
    ///
    /// Like [`Self::auto_tone`], measures a *WB-neutral* render so it's
    /// idempotent, writes only warmth/tint, and declines (no-op) when the frame
    /// has too little neutral reference to trust.
    pub fn auto_wb(&mut self) -> Result<DevelopInfo> {
        use crate::develop::auto::{self, AutoWb};
        let target = {
            let open = self.photo()?;
            let current = open.history.current();
            let measurement = AutoWb::neutral_measurement(current);
            let (w, h) = open.session.framed_proxy(&measurement, 320);
            let pixels =
                self.pipeline
                    .render_to_pixels(&self.gpu, &open.session, &measurement, w, h)?;
            let stats = auto::wb_stats_from_pixels(&pixels);
            let mut target = current.clone();
            auto::auto_wb(&stats).apply_to(&mut target);
            target
        };
        self.apply("Auto WB", target)
    }

    /// White balance from a point the user says is grey: the eyedropper.
    /// `x`, `y` are in framed-image UV, as the canvas reports clicks.
    pub fn pick_wb(&mut self, x: f32, y: f32) -> Result<DevelopInfo> {
        let target = {
            let open = self.photo()?;
            let current = open.history.current();
            let wb = neutral_point(&self.gpu, &self.pipeline, &open.session, current, x, y)?;
            let mut target = current.clone();
            wb.apply_to(&mut target);
            target
        };
        self.apply("White balance picker", target)
    }

    /// The open photo as it looks now, framed, at `edge` on its long side —
    /// what subject selection and click-to-select's encoder look at. The
    /// render takes milliseconds under the develop lock; the models, which
    /// take seconds, run on it after the lock is released (see
    /// [`Self::still_fits`]).
    pub fn selection_input(&self, edge: u32) -> Result<SelectionInput> {
        let open = self.photo()?;
        let stack = open.history.current();
        let (width, height) = open.session.framed_proxy(stack, edge);
        let pixels =
            self.pipeline
                .render_to_pixels(&self.gpu, &open.session, stack, width, height)?;
        Ok(SelectionInput {
            photo_id: open.photo_id,
            geometry: stack.geometry,
            erases: stack.erases.clone(),
            pixels,
            width,
            height,
        })
    }

    /// What erasing the selection `coverage` (over the framed photo) is
    /// worked out from: the selection carried onto the sensor, where the
    /// model will look, and the linear image there. Milliseconds, so it runs
    /// under the develop lock; the model runs after it is released
    /// ([`erase::run`]), and [`Self::attach_erase`] takes the fills.
    pub fn erase_input(&self, coverage: &erase::Coverage) -> Result<EraseInput> {
        let open = self.photo()?;
        let stack = open.history.current();
        let size = open.session.stored_size();
        let grid = erase::Grid::from_coverage(
            coverage,
            size,
            |uv| open.session.sample_uv(stack, uv),
            |uv| open.session.framed_uv(stack, uv),
        )
        .ok_or_else(|| anyhow!("nothing is selected to erase"))?;
        let plan =
            erase::plan(grid, size).ok_or_else(|| anyhow!("nothing is selected to erase"))?;
        let rect = plan
            .reach(size)
            .ok_or_else(|| anyhow!("the selection is outside the photo"))?;
        let rgba =
            self.pipeline
                .read_linear_region(&self.gpu, &open.session, rect.as_region(), false)?;
        Ok(EraseInput {
            photo_id: open.photo_id,
            erases: stack.erases.clone(),
            written: open.written,
            plan,
            region: erase::Region { rect, rgba },
        })
    }

    /// [`Self::erase_input`] for a selection made with the masking tools,
    /// over the photo as it is framed now.
    pub fn erase_selection(&self, selection: &crate::develop::mask::Mask) -> Result<EraseInput> {
        let open = self.photo()?;
        let (w, h) = open
            .session
            .framed_proxy(open.history.current(), erase::SELECTION_EDGE);
        let data = erase::selection_coverage(selection, w, h)?;
        self.erase_input(&erase::Coverage {
            data: &data,
            width: w,
            height: h,
        })
    }

    /// Takes erase `index` out of the open photo's edit, as a step in
    /// history: the object it removed shows again.
    pub fn remove_erase(&mut self, index: usize) -> Result<DevelopInfo> {
        let mut stack = self.current_stack()?;
        anyhow::ensure!(index < stack.erases.len(), "there is no such erase");
        stack.erases.remove(index);
        self.apply("Remove erase", stack)
    }

    /// `stack`, as the webview sends it, with the open photo's own erases.
    /// Only the backend changes them ([`Self::attach_erase`],
    /// [`Self::remove_erase`]): a slider committed while an erase was being
    /// worked out carries the list from before it, and would drop the new one
    /// without a word.
    pub fn with_own_erases(&self, mut stack: EditStack) -> Result<EditStack> {
        stack.erases = self.photo()?.history.current().erases.clone();
        Ok(stack)
    }

    /// Takes magic erase's fills for the photo `input` was made from, as one
    /// step in history labelled `label`. Refused when another photo is open,
    /// or the erases changed meanwhile (one undone, say): the fills were
    /// worked out against what was there.
    pub fn attach_erase(
        &mut self,
        input: &EraseInput,
        label: &str,
        made: Vec<erase::Made>,
    ) -> Result<DevelopInfo> {
        let open = self.photo()?;
        anyhow::ensure!(
            open.photo_id == input.photo_id,
            "another photo was opened while this one was being erased"
        );
        anyhow::ensure!(
            open.history.current().erases == input.erases,
            "the erases changed while erasing — erase again"
        );
        anyhow::ensure!(!made.is_empty(), "nothing was erased");
        let mut stack = open.history.current().clone();
        let store = self
            .fills
            .as_ref()
            .ok_or_else(|| anyhow!("there is no folder to keep erases in"))?;
        let mut pieces = Vec::with_capacity(made.len());
        for m in made {
            store.save(&m.png)?;
            self.fill_cache
                .insert(m.piece.fill.clone(), std::sync::Arc::new(m.fill));
            pieces.push(m.piece);
        }
        stack.erases.push(Erase {
            label: label.to_string(),
            pieces,
        });
        // The run laid its fills into the region it read, over what the image
        // held then. Unless the image was rewritten since, that region is what
        // laying them in again would give, so it goes up as it is, and the
        // step below finds nothing left to do.
        let open = self
            .open
            .as_mut()
            .ok_or_else(|| anyhow!("no photo is open"))?;
        if open.written == input.written {
            if open.denoise.is_none() {
                self.pipeline.keep_original(&self.gpu, &mut open.session);
            }
            let r = &input.region;
            self.pipeline.write_linear_region(
                &self.gpu,
                &open.session,
                r.rect.as_region(),
                &r.rgba,
            )?;
            self.pipeline.rebuild_mips(&self.gpu, &open.session);
            open.written += 1;
            open.erased = stack.erases.clone();
        }
        self.apply("Erase", stack)
    }

    /// Whether click to select's encoding of the photo, as it is framed and
    /// erased now, is held: a click then needs only the decoder.
    #[cfg(feature = "ai")]
    pub fn sam2_ready(&self) -> bool {
        let Some(open) = self.open.as_ref() else {
            return false;
        };
        let now = open.history.current();
        self.sam2.as_ref().is_some_and(|c| {
            c.photo_id == open.photo_id && c.geometry == now.geometry && c.erases == now.erases
        })
    }

    /// Whether a selection worked out from `input` still fits the photo open
    /// now. The model ran with the develop lock released, and Develop stays
    /// live meanwhile — so another photo may have been opened, or the crop
    /// changed, and a mask made for the old framing would land in the wrong
    /// place.
    pub fn still_fits(&self, input: &SelectionInput) -> Result<()> {
        let open = self.photo()?;
        anyhow::ensure!(
            open.photo_id == input.photo_id,
            "another photo was opened while this one was being selected — select again"
        );
        anyhow::ensure!(
            open.history.current().geometry == input.geometry,
            "the crop changed while selecting — select again"
        );
        Ok(())
    }

    /// Click-to-select from the open photo's cached SAM2 embedding: turns
    /// clicked points (framed UV) into an AI mask with only the
    /// millisecond-scale decoder. `None` until the photo has been encoded —
    /// see [`Self::keep_sam2`].
    #[cfg(feature = "ai")]
    pub fn sam2_decode(
        &mut self,
        points: &[crate::develop::sam2::Point],
    ) -> Result<Option<crate::develop::mask::Mask>> {
        use crate::develop::{ai, sam2};
        let open = self.photo()?;
        let photo_id = open.photo_id;
        let geometry = open.history.current().geometry;
        let erases = open.history.current().erases.clone();
        let (w, h) = open
            .session
            .framed_proxy(open.history.current(), sam2::SAM2.size);
        let Some(cache) = self
            .sam2
            .as_mut()
            .filter(|c| c.photo_id == photo_id && c.geometry == geometry && c.erases == erases)
        else {
            return Ok(None);
        };
        let (cw, ch) = ai::coverage_size(w, h);
        let coverage = cache.models.decode(&cache.embedding, points, cw, ch)?;
        Ok(Some(ai::ai_mask("Selection", cw, ch, &coverage)))
    }

    /// Keeps the SAM2 embedding of `input`, worked out with the develop lock
    /// released, for this click and every later one on the photo.
    #[cfg(feature = "ai")]
    pub fn keep_sam2(
        &mut self,
        input: &SelectionInput,
        models: crate::develop::sam2::Sam2,
        embedding: crate::develop::sam2::Embedding,
    ) -> Result<()> {
        self.still_fits(input)?;
        self.sam2 = Some(Sam2Cache {
            photo_id: input.photo_id,
            geometry: input.geometry,
            erases: input.erases.clone(),
            models,
            embedding,
        });
        Ok(())
    }

    /// The SAM2 models already loaded, for encoding a new photo or framing:
    /// they were read from disk again, seconds, for every photo. The
    /// embedding they held goes with the cache.
    #[cfg(feature = "ai")]
    pub fn take_sam2_models(&mut self) -> Option<crate::develop::sam2::Sam2> {
        self.sam2.take().map(|c| c.models)
    }

    /// Renders the open photo at full sensor resolution and encodes it.
    ///
    /// Deliberately reuses `Pipeline::render_to_pixels` — the same call the
    /// preview makes, differing only in the size passed and, for a wide-gamut
    /// export, the output matrix. If an export ever needs its own render
    /// path, the preview has stopped telling the truth.
    pub fn export(
        &self,
        options: &crate::export::ExportOptions,
        metadata: Option<&crate::export::exif::ExportMetadata>,
    ) -> Result<Vec<u8>> {
        let open = self.photo()?;
        let stack = open.history.current();
        let (width, height) = open.session.framed_size(stack);
        let pixels = self.pipeline.render_for_export(
            &self.gpu,
            &open.session,
            stack,
            width,
            height,
            options.output(),
            options.sixteen_bit(),
        )?;
        crate::export::encode_pixels(&pixels, width, height, options, metadata)
    }

    /// Renders the open photo. `stack` overrides the current one, which is how
    /// the before/after comparison renders the unedited version without
    /// disturbing history.
    pub fn render(
        &mut self,
        max_edge: Option<u32>,
        stack: Option<&EditStack>,
        view: Option<crate::develop::geometry::ViewPort>,
        soft_proof: bool,
    ) -> Result<Presented> {
        self.render_from(max_edge, stack, view, soft_proof, false)
    }

    /// The "Before" of the before/after comparison: `stack` (the unedited
    /// one) over the photo as decoded, not the AI-denoised image that has
    /// replaced it in the session.
    pub fn render_before(
        &mut self,
        max_edge: Option<u32>,
        stack: &EditStack,
        view: Option<crate::develop::geometry::ViewPort>,
        soft_proof: bool,
    ) -> Result<Presented> {
        self.render_from(max_edge, Some(stack), view, soft_proof, true)
    }

    fn render_from(
        &mut self,
        max_edge: Option<u32>,
        stack: Option<&EditStack>,
        view: Option<crate::develop::geometry::ViewPort>,
        soft_proof: bool,
        decoded: bool,
    ) -> Result<Presented> {
        let open = self
            .open
            .as_ref()
            .ok_or_else(|| anyhow!("no photo is open"))?;
        let as_decoded = if decoded {
            open.session.as_decoded()
        } else {
            None
        };
        let session = as_decoded.as_ref().unwrap_or(&open.session);
        // Sized from the stack being rendered, not the current one: the
        // before/after comparison passes an unedited stack, and if that were
        // drawn at the edited crop's size the two views would not line up.
        let showing = stack.unwrap_or_else(|| open.history.current());
        let (width, height, view, origin) = match view {
            Some(v) => {
                let (fitted, origin) = Self::viewed(&open.session, showing, v);
                (fitted.width, fitted.height, Some(fitted), origin)
            }
            None => {
                let (w, h) = open
                    .session
                    .framed_proxy(showing, max_edge.unwrap_or(PROXY_EDGE));
                (w, h, None, (0, 0))
            }
        };
        self.presenter.present(Frame {
            gpu: &self.gpu,
            pipeline: &self.pipeline,
            session,
            stack: showing,
            width,
            height,
            origin,
            view,
            soft_proof,
            split: None,
        })
    }

    /// A viewport's frame: capped at the size the photo occupies (see
    /// `ViewPort::fitted`, without which a zoomed view of a portrait frame in a
    /// landscape canvas comes out stretched), and where it sits, centred, in
    /// the area asked for. Readback allocates a texture of exactly the fitted
    /// size and the page centres it; a surface draws at the origin.
    fn viewed(
        session: &crate::gpu::pipeline::DevelopSession,
        showing: &EditStack,
        v: crate::develop::geometry::ViewPort,
    ) -> (crate::develop::geometry::ViewPort, (u32, u32)) {
        let fitted = v.fitted(session.framed_size(showing));
        let origin = (
            v.width.saturating_sub(fitted.width) / 2,
            v.height.saturating_sub(fitted.height) / 2,
        );
        (fitted, origin)
    }

    /// Draws the open photo into the direct surface, which covers a canvas of
    /// `canvas` physical pixels: at `view`, or — `None` — fitted and centred,
    /// as the page's `object-fit: contain` lays out the readback image.
    ///
    /// `split` is the split view's before stack, drawn over the left half of
    /// the canvas. `before` says `stack` is itself the before (the Before view,
    /// hold-to-peek). Either before draws from the photo as decoded, not the
    /// AI-denoised image that has replaced it, as `render_before` does.
    pub fn render_direct(
        &mut self,
        canvas: (u32, u32),
        stack: Option<&EditStack>,
        view: Option<crate::develop::geometry::ViewPort>,
        soft_proof: bool,
        split: Option<&EditStack>,
        before: bool,
    ) -> Result<()> {
        let open = self
            .open
            .as_ref()
            .ok_or_else(|| anyhow!("no photo is open"))?;
        let showing = stack.unwrap_or_else(|| open.history.current());
        let (cw, ch) = (canvas.0.max(1), canvas.1.max(1));
        let (fw, fh) = open.session.framed_size(showing);
        let fit = (cw as f64 / fw.max(1) as f64).min(ch as f64 / fh.max(1) as f64);
        // Fit-to-window renders a proxy that fits the canvas, the sharp path
        // the readback preview takes. Only a photo smaller than the canvas —
        // where there is no proxy to take, and the page would stretch it —
        // goes through a viewport at the magnification that fills it.
        let view = view.or_else(|| {
            (fit > 1.0).then_some(crate::develop::geometry::ViewPort {
                zoom: fit as f32,
                centre: [0.5, 0.5],
                width: cw,
                height: ch,
            })
        });
        let (width, height, view, origin) = match view {
            Some(v) => {
                let (fitted, origin) = Self::viewed(&open.session, showing, v);
                (fitted.width, fitted.height, Some(fitted), origin)
            }
            None => {
                let edge = (fw.max(fh) as f64 * fit).floor().max(1.0) as u32;
                let (w, h) = open.session.framed_proxy(showing, edge);
                (
                    w,
                    h,
                    None,
                    (cw.saturating_sub(w) / 2, ch.saturating_sub(h) / 2),
                )
            }
        };
        let as_decoded = if before || split.is_some() {
            open.session.as_decoded()
        } else {
            None
        };
        let decoded = as_decoded.as_ref().unwrap_or(&open.session);
        let (gpu, pipeline) = (&self.gpu, &self.pipeline);
        let direct = self
            .direct
            .as_mut()
            .ok_or_else(|| anyhow!("direct rendering is off"))?;
        direct.present(Frame {
            gpu,
            pipeline,
            session: if before { decoded } else { &open.session },
            stack: showing,
            width,
            height,
            origin,
            view,
            soft_proof,
            split: split.map(|b| (decoded, b, cw / 2)),
        })?;
        Ok(())
    }
}
