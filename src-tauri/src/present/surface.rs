//! Drawing straight into a window, rather than through an encoded image.
//!
//! The readback path costs a GPU render (0.3ms), a readback (5ms) and a JPEG
//! encode (49ms) per frame — the encode alone is about 95% of it. Drawing to a
//! surface deletes all three: the develop pass writes into the swapchain and
//! the compositor shows it.
//!
//! **Every platform-specific line lives behind the window handle this is given.**
//! The presenter itself is portable wgpu; what differs between Windows and
//! macOS is only *which* window it is pointed at and how the webview is made to
//! let it through. That is the rule the `present` module exists to enforce.
//!
//! The surface lives on a [`HostedSurface`] — a window of the canvas's own,
//! beneath the webview — not on the app window. That was learned the hard way:
//! Windows clips a parent window's drawing to exclude its children, so a
//! swapchain on the Tauri window presented into a region the WebView2 child
//! covered, and nothing ever appeared. See `host.rs`.
//!
//! Status: **opt-in**, readback remains the default. See `choose_config` for the
//! one decision that would silently corrupt colour if taken wrongly.

use anyhow::{Context, Result};

use super::host::HostedSurface;
use super::{Frame, Presented, Presenter};

/// Picks a swapchain configuration.
///
/// **The format must not be an `*_srgb` one.** The develop shader encodes sRGB
/// itself, as its final stage — handing it an sRGB surface makes the hardware
/// encode a second time and washes the image out. The offscreen path already
/// avoids exactly this by rendering to `Rgba8Unorm` rather than
/// `Rgba8UnormSrgb`, and the same trap is waiting here with no test to catch it
/// unless one is written: the picture still renders, it is simply wrong.
///
/// A free function taking capabilities rather than a method on the surface, so
/// the choice can be tested without a window.
pub fn choose_config(
    caps: &wgpu::SurfaceCapabilities,
    width: u32,
    height: u32,
) -> wgpu::SurfaceConfiguration {
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| !f.is_srgb())
        .unwrap_or_else(|| {
            // Every offered format is sRGB. Taking one means a double encode,
            // but refusing to draw at all is worse, and the first format is
            // what the platform prefers.
            caps.formats
                .first()
                .copied()
                .unwrap_or(wgpu::TextureFormat::Bgra8Unorm)
        });

    wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        // Zero in either axis is a valid window state — minimised, or mid
        // resize — and an invalid surface. Configuring one puts the swapchain
        // into an unrecoverable state rather than failing loudly.
        width: width.max(1),
        height: height.max(1),
        // Vsync. An editor is not a game: tearing while dragging a slider looks
        // like a bug, and rendering faster than the display refreshes spends
        // power to show nobody anything.
        present_mode: wgpu::PresentMode::AutoVsync,
        alpha_mode: caps
            .alpha_modes
            .first()
            .copied()
            .unwrap_or(wgpu::CompositeAlphaMode::Auto),
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    }
}

/// A presenter that owns a window and a swapchain on it.
pub struct SurfacePresenter {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// The develop pipeline built for *this* swapchain's format. A swapchain is
    /// usually `Bgra8Unorm` where the offscreen target is `Rgba8Unorm`, and
    /// wgpu requires the pipeline's declared target to match exactly.
    pipeline: wgpu::RenderPipeline,
    /// The swapchain reported a frame suboptimal: rebuild it once that frame
    /// is presented.
    stale: bool,
    /// Declared last so it is **dropped last**. The surface borrows this
    /// window's handle, and destroying the window first would leave wgpu
    /// holding a swapchain on a window that no longer exists. Rust drops
    /// struct fields in declaration order, so this ordering is the safety
    /// argument behind `HostedSurface::target`.
    host: HostedSurface,
}

impl SurfacePresenter {
    /// Takes ownership of a hosted window and builds a swapchain on it.
    pub fn new(
        gpu: &crate::gpu::Gpu,
        pipeline: &crate::gpu::pipeline::Pipeline,
        host: HostedSurface,
    ) -> Result<Self> {
        // SAFETY: the target borrows `host`'s window handle, and `host` is
        // moved into the returned presenter and dropped after the surface —
        // see the field ordering above.
        let target = unsafe { host.target() }?;
        // SAFETY: as above: the window `target` names outlives the surface.
        let surface = unsafe { gpu.instance.create_surface_unsafe(target) }
            .context("creating a swapchain on the render window")?;
        let caps = surface.get_capabilities(&gpu.adapter);
        if caps.formats.is_empty() {
            anyhow::bail!("the adapter cannot present to this window");
        }

        let (width, height) = host.size();
        let config = choose_config(&caps, width, height);
        surface.configure(&gpu.device, &config);
        let pipeline = pipeline.develop_pipeline_for(gpu, config.format);
        Ok(Self {
            surface,
            config,
            pipeline,
            stale: false,
            host,
        })
    }

    /// The swapchain's current size, so a caller can avoid reconfiguring when
    /// nothing has moved.
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }
}

impl Presenter for SurfacePresenter {
    fn present(&mut self, frame: Frame<'_>) -> Result<Presented> {
        // The swapchain *is* the canvas: the hosted window is sized to it by
        // `place`, so there is no rect to mask out and no scissor to get wrong.

        use wgpu::CurrentSurfaceTexture as Acquired;
        let texture = match self.surface.get_current_texture() {
            Acquired::Success(t) => t,
            // Suboptimal still hands over a usable image, so the frame is drawn
            // and the swapchain rebuilt for the next one. Dropping it would
            // stall the preview through an entire window resize.
            // Rebuilt after this frame is presented, not before: a
            // reconfigure while the frame is still held is a wgpu validation
            // error, which panicked on Vulkan, the one backend that reports it.
            Acquired::Suboptimal(t) => {
                self.stale = true;
                t
            }
            // None of these are failures. A window being resized, minimised,
            // occluded or moved between displays produces them routinely, and
            // the recovery is to reconfigure and skip *one* frame. Treating any
            // of them as an error would close the editor when the user drags
            // the window to a second monitor.
            Acquired::Outdated | Acquired::Lost => {
                self.surface.configure(&frame.gpu.device, &self.config);
                return Ok(Presented::Direct);
            }
            Acquired::Timeout | Acquired::Occluded => return Ok(Presented::Direct),
            other => {
                return Err(anyhow::anyhow!(
                    "could not acquire a swapchain image: {other:?}"
                ))
            }
        };

        frame.pipeline.render_to_surface(
            frame.gpu,
            frame.session,
            frame.stack,
            &texture.texture,
            &self.pipeline,
            frame.origin,
            frame.width,
            frame.height,
            frame.view.as_ref(),
            frame.soft_proof,
            None,
        );
        // The before half over the after, never proofed — as the readback
        // split shows it.
        if let Some((session, before, at)) = frame.split {
            frame.pipeline.render_to_surface(
                frame.gpu,
                session,
                before,
                &texture.texture,
                &self.pipeline,
                frame.origin,
                frame.width,
                frame.height,
                frame.view.as_ref(),
                false,
                Some(at),
            );
        }
        texture.present();
        if std::mem::take(&mut self.stale) {
            self.surface.configure(&frame.gpu.device, &self.config);
        }
        Ok(Presented::Direct)
    }

    fn produces_bytes(&self) -> bool {
        false
    }

    fn place(&mut self, gpu: &crate::gpu::Gpu, x: i32, y: i32, width: u32, height: u32) {
        // The window moves every time regardless — a scroll or a panel resize
        // shifts the canvas without changing its size — but the swapchain is
        // only rebuilt when the size actually changed. Reconfiguring on a pure
        // move would reallocate the images for nothing.
        //
        // Clamped here, at the sink, rather than by each caller: a swapchain
        // past the device's texture limit fails validation, which wgpu
        // answers with a panic on the main thread.
        let limit = gpu.max_edge();
        let (width, height) = (width.min(limit), height.min(limit));
        if !self.host.place(x, y, width, height) {
            return;
        }
        let (width, height) = self.host.size();
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&gpu.device, &self.config);
    }

    fn probe(&mut self, gpu: &crate::gpu::Gpu) -> Result<String> {
        let where_it_is = self.host.describe();

        use wgpu::CurrentSurfaceTexture as Acquired;
        let texture = match self.surface.get_current_texture() {
            Acquired::Success(t) | Acquired::Suboptimal(t) => t,
            other => return Ok(format!("no swapchain image ({other:?}) — {where_it_is}")),
        };

        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("direct probe"),
            });
        // A clear and nothing else: no pipeline, no bind group, no textures.
        // If this does not appear, the render graph is not the problem.
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("direct probe"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 1.0,
                        g: 0.0,
                        b: 1.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        gpu.queue.submit(Some(encoder.finish()));
        texture.present();

        Ok(format!(
            "presented magenta, {}x{} {:?}, alpha {:?} — {where_it_is}",
            self.config.width, self.config.height, self.config.format, self.config.alpha_mode,
        ))
    }

    fn describe(&self) -> &'static str {
        "direct surface"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(formats: Vec<wgpu::TextureFormat>) -> wgpu::SurfaceCapabilities {
        wgpu::SurfaceCapabilities {
            formats,
            present_modes: vec![wgpu::PresentMode::Fifo],
            alpha_modes: vec![wgpu::CompositeAlphaMode::Opaque],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
        }
    }

    /// **The one that matters.** The shader encodes sRGB as its last stage, so
    /// an sRGB surface would encode it again and wash every photo out. The
    /// offscreen path avoids this by rendering to `Rgba8Unorm`; nothing would
    /// have caught the same mistake here.
    #[test]
    fn a_linear_format_is_preferred_over_an_srgb_one() {
        let c = caps(vec![
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Bgra8Unorm,
        ]);
        let config = choose_config(&c, 800, 600);
        assert!(
            !config.format.is_srgb(),
            "chose {:?}, which double-encodes sRGB",
            config.format
        );
    }

    /// A window can legitimately report zero size — minimised, or mid-resize —
    /// and configuring a zero-sized swapchain is invalid. It has to be clamped
    /// rather than passed through.
    #[test]
    fn a_zero_sized_window_still_produces_a_valid_configuration() {
        let c = caps(vec![wgpu::TextureFormat::Bgra8Unorm]);
        for (w, h) in [(0, 0), (0, 600), (800, 0)] {
            let config = choose_config(&c, w, h);
            assert!(config.width >= 1 && config.height >= 1, "{w}x{h}");
        }
    }

    /// If the platform offers nothing but sRGB there is still a picture to
    /// draw; refusing would be worse than a known colour error.
    #[test]
    fn an_srgb_only_platform_still_gets_a_configuration() {
        let c = caps(vec![wgpu::TextureFormat::Bgra8UnormSrgb]);
        assert_eq!(
            choose_config(&c, 100, 100).format,
            wgpu::TextureFormat::Bgra8UnormSrgb
        );
    }

    /// Vsync, not the fastest available mode: an editor tearing while a slider
    /// is dragged reads as a rendering bug.
    #[test]
    fn presentation_waits_for_the_display() {
        let c = caps(vec![wgpu::TextureFormat::Bgra8Unorm]);
        assert_eq!(
            choose_config(&c, 100, 100).present_mode,
            wgpu::PresentMode::AutoVsync
        );
    }
}
