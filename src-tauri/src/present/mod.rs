//! Getting a rendered frame in front of the user.
//!
//! This is deliberately a seam. Today the only implementation reads the frame
//! back and hands the webview an encoded image, which is portable and easy to
//! debug. Later a direct GPU surface will draw into a transparent cutout in the
//! window — far faster, and entirely platform-specific.
//!
//! **The rule: no platform `#[cfg]` outside this module.** wgpu, WGSL, LibRaw
//! and SQLite are all portable; window compositing is not. Confining that to
//! one module is what stops macOS support becoming a rewrite, and it only works
//! if the render graph never learns which presenter it is feeding.

#[cfg(target_os = "macos")]
mod appkit;
pub mod host;
pub mod readback;
pub mod surface;
#[cfg(windows)]
mod win32;

use anyhow::Result;

use crate::develop::EditStack;
use crate::gpu::pipeline::{DevelopSession, Pipeline};
use crate::gpu::Gpu;

/// What a presenter produced.
pub enum Presented {
    /// An encoded image the UI has to fetch and display itself.
    Image {
        bytes: Vec<u8>,
        /// MIME type of `bytes`. Carried here rather than assumed by the
        /// handler: the presenter chooses the format, so it is the only thing
        /// that knows what it produced, and a hard-coded `image/jpeg` would
        /// silently mislabel the frame the moment that choice changed.
        content_type: &'static str,
        width: u32,
        height: u32,
    },
    /// Already drawn to a native surface; the UI has nothing to transfer.
    Direct,
}

impl Presented {
    /// Encoded bytes, if this presenter produces any.
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Image { bytes, .. } => Some(bytes),
            Self::Direct => None,
        }
    }

    /// MIME type for [`Self::bytes`].
    pub fn content_type(&self) -> Option<&'static str> {
        match self {
            Self::Image { content_type, .. } => Some(content_type),
            Self::Direct => None,
        }
    }
}

/// Everything a presenter needs to draw one frame.
pub struct Frame<'a> {
    pub gpu: &'a Gpu,
    pub pipeline: &'a Pipeline,
    pub session: &'a DevelopSession,
    pub stack: &'a EditStack,
    /// Draw size in physical pixels.
    pub width: u32,
    pub height: u32,
    /// Where in the target to draw, in physical pixels.
    ///
    /// `(0, 0)` for both presenters as things stand — readback owns a texture
    /// the size of the frame, and the direct path gives the canvas a window of
    /// its own sized to it. It stays because it is what makes the second true:
    /// without it a surface would have to cover the whole app window and mask
    /// the panels out with a scissor, which is the arrangement that turned out
    /// not to composite at all.
    pub origin: (u32, u32),
    /// What the viewer is looking at. `None` shows the whole frame.
    pub view: Option<crate::develop::geometry::ViewPort>,
    /// Soft-proof (gamut check): paint colours outside the output gamut a marker.
    /// A viewing aid, so it is only ever set on the interactive path.
    pub soft_proof: bool,
    /// The split view: this stack — the unedited photo — drawn from this
    /// session (the photo as decoded, not denoised) over the left of the
    /// frame, up to the column given, with `stack` to its right. Only a
    /// surface draws it; the readback path serves the two halves as two images
    /// and never sets it.
    pub split: Option<(&'a DevelopSession, &'a EditStack, u32)>,
}

pub trait Presenter: Send {
    fn present(&mut self, frame: Frame<'_>) -> Result<Presented>;

    /// Whether [`Self::present`] yields bytes to transfer.
    ///
    /// Asked *before* rendering, not inferred from the result. A direct
    /// presenter draws into a window, so anything that only wants bytes — the
    /// `develop://` handler — must skip the render entirely rather than run it
    /// and throw the outcome away. Driving a swapchain from the webview's
    /// resource thread is how the app aborted: wgpu panics on a validation
    /// error, and a panic crossing the WebView2 COM boundary cannot unwind.
    fn produces_bytes(&self) -> bool {
        true
    }

    /// Tells the presenter where the canvas is, in physical pixels relative to
    /// the app window's client area.
    ///
    /// A no-op for anything that owns its own target — readback allocates a
    /// texture per frame and has nothing to position. A hosted surface moves
    /// its window and, if the size changed, rebuilds its swapchain: **only when
    /// the final size is known**, since reconfiguring on every intermediate
    /// size during a drag-resize reallocates dozens of times a second, and on
    /// DX12 a configure racing an in-flight frame is the classic way to lose the
    /// device. Position and size arrive together because they come from one
    /// `getBoundingClientRect`, and splitting them would let the window sit in
    /// the wrong place for a frame.
    fn place(&mut self, _gpu: &Gpu, _x: i32, _y: i32, _width: u32, _height: u32) {}

    /// Presents one unmistakable flat frame and reports what happened.
    ///
    /// **This exists to separate two failures that look identical.** A canvas
    /// showing nothing can mean the target is not being composited, or that it
    /// is composited fine and the photo did not draw — and the fixes point in
    /// opposite directions. A frame that owes nothing to the render graph, in a
    /// colour no photograph produces, tells the two apart in one look.
    fn probe(&mut self, _gpu: &Gpu) -> Result<String> {
        anyhow::bail!("this presenter draws through the webview, so there is nothing to probe")
    }

    /// Short identifier for diagnostics — which path a frame actually took is
    /// otherwise invisible.
    fn describe(&self) -> &'static str;
}
