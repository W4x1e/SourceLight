//! GPU context for the develop pipeline.
//!
//! Deliberately **headless** — no surface, no window. The same render graph has
//! to serve both the interactive preview and headless batch export, so nothing
//! here may assume a swapchain exists. Presenting the result to a screen is a
//! separate concern that sits behind its own seam.

pub mod color;
#[cfg(test)]
pub mod golden;
pub mod pipeline;

use anyhow::{anyhow, Context, Result};

/// An initialised device and queue, plus what adapter we ended up on. A clone
/// is the same device: wgpu's handles are shared.
#[derive(Clone)]
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    /// Kept so a presenter can build a swapchain.
    ///
    /// Offscreen rendering needs neither, and the first version of this struct
    /// dropped both. A surface presenter cannot be added without them: creating
    /// one needs the instance, and choosing its format needs to ask the adapter
    /// what the window actually supports.
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
}

/// The app's device, once one is made: see [`shared`].
static SHARED: std::sync::Mutex<Option<Gpu>> = std::sync::Mutex::new(None);

/// The app's one GPU device, made the first time it's asked for.
///
/// Develop and every export each made their own. A device took about 0.4 s
/// to make, on every Copy image and every batch, and an export's sat beside
/// Develop's for as long as it ran. Each caller still builds its own
/// [`pipeline::Pipeline`], whose tone and look tables a render rewrites, so
/// they don't share those; once the driver has compiled the shader that's
/// about 11 ms.
pub fn shared() -> Result<Gpu> {
    // Held while the device is made, so two first callers make one.
    let mut slot = SHARED.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(gpu) = slot.as_ref() {
        return Ok(gpu.clone());
    }
    let gpu = Gpu::new()?;
    #[cfg(test)]
    MADE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    *slot = Some(gpu.clone());
    Ok(gpu)
}

/// How many devices [`shared`] has made, for its test. wgpu's own equality
/// can't tell: a device's id is only unique within its `Instance`, and each
/// `Gpu::new` makes one.
#[cfg(test)]
static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Makes the next [`shared`] make a new device. For starting Develop afresh
/// after a render panicked, which made it a new device before there was one
/// to share, in case the device was the trouble.
pub fn forget_shared() {
    *SHARED.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

impl Gpu {
    /// Blocking init, for callers outside an async context.
    pub fn new() -> Result<Self> {
        pollster::block_on(Self::new_async())
    }

    pub async fn new_async() -> Result<Self> {
        // No display handle held here. That is not a restriction on surfaces:
        // wgpu only needs one to *discover* a display, and the direct presenter
        // supplies the window and display handles together through
        // `create_surface_unsafe`. Keeping this as it was means adding the
        // presenter cannot disturb the offscreen path every test depends on.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = request_adapter(&instance).await?;
        let info = adapter.get_info();
        let available = adapter.limits();

        // Sixteen-bit unorm targets, where the adapter can render to them
        // (Apple's GPUs can), give a 16-bit export all sixteen bits; without
        // them it renders to half floats instead (`deep_format`). The format
        // alone is only sampleable: rendering to it is the adapter's own
        // capability, which has to be asked for too. Both only when offered,
        // so no machine is turned away.
        let deep = wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
            | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        let renders_deep = adapter.features().contains(deep)
            && adapter
                .get_texture_format_features(wgpu::TextureFormat::Rgba16Unorm)
                .allowed_usages
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT);
        let optional = if renders_deep {
            deep
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("sourcelight"),
                required_features: optional,
                // The widely-supported floor, except for size. wgpu enforces
                // the limits a device asks for, not what the adapter offers,
                // and the floor's 8,192 px texture edge and 256 MiB buffer are
                // smaller than a 45-100 MP raw or a stitched panorama.
                // Taking the adapter's own values for those two turns no
                // machine away: they are what it already supports.
                required_limits: wgpu::Limits {
                    max_buffer_size: available.max_buffer_size,
                    ..wgpu::Limits::default().using_resolution(available)
                },
                memory_hints: wgpu::MemoryHints::Performance,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                trace: wgpu::Trace::Off,
            })
            .await
            .context("failed to create GPU device")?;

        // wgpu's default for an error nothing captured is to panic, and a
        // panic in a main-thread command aborts the whole app. Every
        // path that can fail on a photo's size or the GPU's memory runs
        // inside `checked`; this is the backstop for anything that does not.
        // Tests keep the panic, which is how a validation error fails one.
        #[cfg(not(test))]
        device.on_uncaptured_error(std::sync::Arc::new(|e| {
            eprintln!("GPU error nothing was waiting for: {e}");
        }));

        Ok(Self {
            device,
            queue,
            info,
            instance,
            adapter,
        })
    }

    /// The longest side a texture can have on this device: 16,384 px on
    /// every DX12, Metal and Vulkan GPU the app targets.
    pub fn max_edge(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    /// Runs `f` with the GPU's errors captured, so a texture too large or an
    /// allocation that fails comes back as an `Err` naming `what`, instead of
    /// wgpu's default of a panic.
    ///
    /// Error scopes belong to the calling thread, so `f` must make its GPU
    /// calls on this one; CPU work on a pool inside it is fine.
    pub fn checked<T>(&self, what: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let memory = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let invalid = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let out = f();
        // Popped innermost first, as scopes have to be.
        let invalid = pollster::block_on(invalid.pop());
        let internal = pollster::block_on(internal.pop());
        let memory = pollster::block_on(memory.pop());
        if let Some(e) = memory {
            return Err(anyhow!("the graphics card ran out of memory {what}: {e}"));
        }
        if let Some(e) = invalid.or(internal) {
            return Err(anyhow!("the graphics card refused {what}: {e}"));
        }
        out
    }

    /// Copies an `Rgba8Unorm` texture back to the CPU, with the row padding the
    /// copy alignment forces on us already stripped.
    ///
    /// `copy_texture_to_buffer` requires each row to start on a
    /// [`wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`] boundary, so for most widths the
    /// buffer is wider than the image. Getting that wrong yields a picture with
    /// a diagonal skew, which is easy to misread as a shader bug — so it is
    /// handled once, here, rather than at each call site.
    pub fn read_rgba8(&self, texture: &wgpu::Texture, width: u32, height: u32) -> Result<Vec<u8>> {
        self.read_texture(texture, width, height, 4)
    }

    /// Level 0 of any texture of `bytes_per_texel`, tightly packed; see
    /// [`Self::read_rgba8`] for the row padding this strips.
    pub fn read_texture(
        &self,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
        bytes_per_texel: u32,
    ) -> Result<Vec<u8>> {
        self.read_texture_region(texture, (0, 0), width, height, bytes_per_texel)
    }

    /// A `width` x `height` region of level 0 from `origin`, tightly packed:
    /// [`Self::read_texture`] for part of a texture.
    pub fn read_texture_region(
        &self,
        texture: &wgpu::Texture,
        origin: (u32, u32),
        width: u32,
        height: u32,
        bytes_per_texel: u32,
    ) -> Result<Vec<u8>> {
        let unpadded = width * bytes_per_texel;
        let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: padded as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: origin.0,
                    y: origin.1,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        // Block until the copy has actually landed, otherwise the map callback
        // never fires and the receive below hangs.
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|e| anyhow!("GPU poll failed: {e}"))?;
        rx.recv()
            .context("readback buffer was never mapped")?
            .context("failed to map readback buffer")?;

        let mapped = slice.get_mapped_range();
        let mut out = Vec::with_capacity((unpadded * height) as usize);
        for row in 0..height {
            let start = (row * padded) as usize;
            out.extend_from_slice(&mapped[start..start + unpadded as usize]);
        }
        drop(mapped);
        buffer.unmap();

        Ok(out)
    }

    /// An `Rgba8Unorm` render target that can also be read back.
    ///
    /// Unorm rather than `Rgba8UnormSrgb` on purpose: the pipeline works in
    /// linear light and applies its own output transform as an explicit stage,
    /// so letting the hardware apply a second, implicit sRGB encode would
    /// double-encode every export.
    pub fn create_render_target(&self, width: u32, height: u32) -> wgpu::Texture {
        self.create_render_target_as(width, height, wgpu::TextureFormat::Rgba8Unorm)
    }

    /// The format a 16-bit export renders to: sixteen-bit unorm where the
    /// device has it, else half floats. A half float holds the encoded 0..1
    /// to one part in 2048 at the brightest and finer below — eight times
    /// finer than eight bits, where banding shows first — and the file is
    /// sixteen-bit either way.
    pub fn deep_format(&self) -> wgpu::TextureFormat {
        if self.device.features().contains(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        ) {
            wgpu::TextureFormat::Rgba16Unorm
        } else {
            wgpu::TextureFormat::Rgba16Float
        }
    }

    /// A render target of `format` that can also be read back.
    pub fn create_render_target_as(
        &self,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// Copies a [`Self::deep_format`] texture back as sixteen-bit RGBA, 0..=65535
    /// for 0..1, row padding stripped as [`Self::read_rgba8`] strips it.
    pub fn read_rgba16(
        &self,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) -> Result<Vec<u16>> {
        let bytes = self.read_texture(texture, width, height, 8)?;
        let halves = bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]));
        Ok(match texture.format() {
            wgpu::TextureFormat::Rgba16Unorm => halves.collect(),
            wgpu::TextureFormat::Rgba16Float => halves
                .map(|h| {
                    let v = half::f16::from_bits(h).to_f32();
                    (v.clamp(0.0, 1.0) * 65535.0).round() as u16
                })
                .collect(),
            other => anyhow::bail!("a {other:?} texture is not a sixteen-bit render"),
        })
    }

    /// Whether this adapter rasterises on the CPU rather than on real hardware.
    ///
    /// True for WARP on Windows and lavapipe/llvmpipe elsewhere. Worth asking
    /// before doing anything expensive: software rendering is two to three
    /// orders of magnitude slower, so work that takes eleven seconds on a GPU
    /// does not finish in a quarter of an hour here.
    ///
    /// Note this is not the same question as "did `request_adapter` take the
    /// fallback path" — a machine whose *only* adapter is llvmpipe reports a
    /// CPU device on the first attempt, having never fallen back at all.
    pub fn is_software(&self) -> bool {
        self.info.device_type == wgpu::DeviceType::Cpu
    }
}

/// Prefers a real GPU, then retries forcing a software adapter so CI runners
/// without one still exercise the pipeline.
///
/// **The fallback says so.** It is a rasteriser on the CPU, and it is between
/// two and three orders of magnitude slower — a render that takes 11 seconds on
/// the GPU does not finish in a quarter of an hour on WARP. Falling back
/// silently turns that into "the tests are slow today", which is a very
/// expensive thing to debug: the usual cause on a developer machine is simply
/// that the app is already open and holding the adapter.
async fn request_adapter(instance: &wgpu::Instance) -> Result<wgpu::Adapter> {
    let mut last_err = None;
    for force_fallback_adapter in [false, true] {
        match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter,
            })
            .await
        {
            Ok(adapter) => {
                if force_fallback_adapter {
                    eprintln!(
                        "gpu: no hardware adapter available — falling back to software \
                         rendering, which is hundreds of times slower. If the app is \
                         running, close it and try again."
                    );
                }
                return Ok(adapter);
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(anyhow!(
        "no GPU adapter available (including software fallback): {}",
        last_err.map(|e| e.to_string()).unwrap_or_default()
    ))
}

/// Shared GPU-test gating.
///
/// **Public, and not `#[cfg(test)]`, on purpose.** The integration tests in
/// `tests/` link this crate from outside, so they cannot see anything gated on
/// `cfg(test)` — and they need exactly the same rule as the unit tests. One
/// copy is better than eight that drift.
pub mod test_support {
    use super::Gpu;

    /// A hardware GPU, or `None` with the skip already recorded.
    ///
    /// Returns `None` when there is no adapter *or* when the only adapter
    /// rasterises on the CPU. Each test that gets `None` is recorded as a
    /// [`Needs::Gpu`](crate::test_support::Needs::Gpu) skip — once per test, not
    /// once per binary, so the count means what it says — and a run that
    /// declares `SOURCELIGHT_REQUIRE=gpu` (or the older
    /// `SOURCELIGHT_REQUIRE_GPU=1`) fails instead, which is how you confirm
    /// the guard is not hiding coverage you meant to have.
    ///
    /// **One device for the whole test binary.** Each caller used to build its
    /// own, which meant forty-four device creations and forty-four compilations
    /// of a thousand-line shader — and the test harness runs as many threads as
    /// the machine has cores, so they all did it at once. Measured: 15.8 seconds
    /// run serially against **over ten minutes** and still climbing in parallel,
    /// at six percent of one core, which reads as a hang rather than as
    /// contention. Sharing removes the contention and the repeated compile
    /// together.
    ///
    /// Safe to share: the tests only ever read through `&Gpu`, and wgpu's device
    /// and queue are both `Sync`.
    pub fn hardware_gpu() -> Option<&'static Gpu> {
        static SHARED: std::sync::OnceLock<Result<Gpu, String>> = std::sync::OnceLock::new();
        match SHARED.get_or_init(init_gpu) {
            Ok(gpu) => Some(gpu),
            Err(why) => {
                crate::test_support::skip(crate::test_support::Needs::Gpu, why);
                None
            }
        }
    }

    /// The shared device, or why there is not one.
    fn init_gpu() -> Result<Gpu, String> {
        match Gpu::new() {
            Ok(gpu) if gpu.is_software() => Err(format!(
                "no hardware GPU — software adapter only ({}, {:?})",
                gpu.info.name, gpu.info.backend
            )),
            Ok(gpu) => {
                eprintln!("GPU: {} ({:?})", gpu.info.name, gpu.info.backend);
                Ok(gpu)
            }
            Err(e) => Err(format!("no GPU adapter: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One device for the app: asked for twice, the same one; once
    /// forgotten, a new one.
    #[test]
    fn the_app_shares_one_device() {
        if test_support::hardware_gpu().is_none() {
            return;
        }
        let made = || MADE.load(std::sync::atomic::Ordering::SeqCst);
        shared().unwrap();
        let before = made();
        shared().unwrap();
        shared().unwrap();
        assert_eq!(made(), before, "asked again, it made another");
        forget_shared();
        shared().unwrap();
        assert_eq!(made(), before + 1, "forgotten, it made no new one");
    }

    /// A hardware GPU, or `None` and a reason.
    ///
    /// Skips on a **software adapter**, not just a missing one. That distinction
    /// is the whole point: CI runners have no GPU, so `Gpu::new` succeeds on
    /// WARP and the tests then run on a CPU rasteriser — where they took the
    /// test binary down mid-suite rather than merely crawling. A missing adapter
    /// was never the failure mode worth guarding against; a present but useless
    /// one was.
    ///
    /// Set `SOURCELIGHT_REQUIRE=gpu` to make either case a hard failure,
    /// which is how you check the guard is not quietly hiding real coverage.
    fn gpu() -> Option<&'static Gpu> {
        super::test_support::hardware_gpu()
    }

    /// Device, queue, texture copy and — the part most likely to be silently
    /// wrong — the row-padding maths in `read_rgba8`. The width is deliberately
    /// not a multiple of 64 pixels so the padded and unpadded strides differ.
    #[test]
    fn clearing_a_target_reads_back_exactly() {
        let Some(gpu) = gpu() else { return };
        let (w, h) = (37, 5);

        let target = gpu.create_render_target(w, h);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Written as k/255 because unorm8 stores exactly that:
                        // only multiples of 1/255 survive the round trip. A
                        // tidy-looking 0.5 lands on 127.5 — an exact tie whose
                        // rounding direction is not guaranteed across adapters,
                        // which is precisely how a golden-image test becomes
                        // intermittently red on someone else's machine.
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 64.0 / 255.0,
                            g: 128.0 / 255.0,
                            b: 191.0 / 255.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        gpu.queue.submit([encoder.finish()]);

        let pixels = gpu.read_rgba8(&target, w, h).unwrap();

        assert_eq!(pixels.len(), (w * h * 4) as usize);
        for (i, px) in pixels.chunks_exact(4).enumerate() {
            assert_eq!(px, [64, 128, 191, 255], "pixel {i} differs");
        }
    }

    /// Proves WGSL actually compiles and rasterises. The shader picks between
    /// two exact colours by pixel coordinate, so there is no float rounding to
    /// make this flaky across adapters.
    #[test]
    fn a_wgsl_shader_rasterises() {
        let Some(gpu) = gpu() else { return };
        let (w, h) = (4, 2);

        let pixels = render_fragment(
            gpu,
            r#"
    if (pos.x < 2.0) {
        return vec4<f32>(1.0, 0.0, 0.0, 1.0);
    }
    return vec4<f32>(0.0, 1.0, 0.0, 1.0);
"#,
            w,
            h,
        );

        let px = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            &pixels[i..i + 4]
        };

        for y in 0..h {
            assert_eq!(px(0, y), [255, 0, 0, 255], "left half at row {y}");
            assert_eq!(px(1, y), [255, 0, 0, 255], "left half at row {y}");
            assert_eq!(px(2, y), [0, 255, 0, 255], "right half at row {y}");
            assert_eq!(px(3, y), [0, 255, 0, 255], "right half at row {y}");
        }
    }

    /// Exercises the golden harness against a gradient — values that land
    /// between representable unorm8 steps, which is where cross-vendor rounding
    /// actually differs and therefore what the tolerance exists for.
    #[test]
    fn a_gradient_matches_its_reference() {
        let Some(gpu) = gpu() else { return };
        let (w, h) = (64, 64);

        let pixels = render_fragment(
            gpu,
            r#"
    let uv = pos.xy / 64.0;
    return vec4<f32>(uv.x, uv.y, 1.0 - uv.x * uv.y, 1.0);
"#,
            w,
            h,
        );

        golden::Golden::new("gradient").assert_matches(&pixels, w, h);
    }

    /// Renders `body` as the fragment shader of a full-screen triangle and
    /// reads the result back. `body` sees `pos` (`@builtin(position)`) and must
    /// return a `vec4<f32>`.
    fn render_fragment(gpu: &Gpu, body: &str, w: u32, h: u32) -> Vec<u8> {
        let source = format!(
            r#"
@vertex
fn vs(@builtin(vertex_index) idx: u32) -> @builtin(position) vec4<f32> {{
    // Single oversized triangle covering the whole target.
    let uv = vec2<f32>(f32((idx << 1u) & 2u), f32(idx & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {{
{body}
}}
"#
        );

        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("test"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });

        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("test"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        let target = gpu.create_render_target(w, h);
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("test"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit([encoder.finish()]);

        gpu.read_rgba8(&target, w, h).unwrap()
    }
}
