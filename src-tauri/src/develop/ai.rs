//! AI auto-masking: turn a developed frame into a coverage mask with a
//! segmentation model, fed into the mask system as a [`crate::develop::mask::
//! MaskShape::Ai`].
//!
//! Two halves:
//! - **Pure image maths** (`preprocess`/`postprocess`, always compiled and
//!   unit-tested) — resize + ImageNet-normalise the frame into the model's NCHW
//!   input, and turn the model's matte back into an R8 coverage. This is where
//!   the correctness risk lives, so it is testable without a model or a GPU.
//! - **The ONNX session** (`Segmenter`, behind the `ai` cargo feature) — loads
//!   the model via `ort`/ONNX Runtime with DirectML (Windows) / CoreML (macOS) /
//!   CPU, runs it. Off by default so the base build (and CI) never pulls ONNX
//!   Runtime; enable with `--features ai`.
//!
//! First model: **BiRefNet** (MIT) — a single forward pass producing a
//! foreground matte, i.e. "Select Subject". SAM2 (click-to-select) is a later
//! increment; it slots in behind the same `MaskShape::Ai` seam.

use crate::develop::mask::{self, Mask, MaskComponent, MaskShape};

/// What activation, if any, the model's output needs to become 0..1 coverage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Activation {
    /// Output is already 0..1 — clamp only.
    None,
    /// Output is logits — apply a sigmoid.
    Sigmoid,
    /// Decide from the data: sigmoid if the matte strays outside 0..1 (logits),
    /// clamp if it is already probabilities. Robust to either export.
    Auto,
}

/// How a segmentation model wants its input and gives its output.
#[derive(Debug, Clone, Copy)]
pub struct SegConfig {
    /// Square input edge the frame is resized to (BiRefNet: 1024).
    pub size: u32,
    /// Per-channel normalisation, ImageNet by convention.
    pub mean: [f32; 3],
    pub std: [f32; 3],
    /// How to turn the raw output into 0..1 coverage.
    pub activation: Activation,
}

/// BiRefNet-family defaults (MIT weights: use BiRefNet, **not** the
/// non-commercial RMBG-2.0). `Auto` activation copes with exports that emit
/// probabilities and those that emit logits without a config change.
pub const BIREFNET: SegConfig = SegConfig {
    size: 1024,
    mean: [0.485, 0.456, 0.406],
    std: [0.229, 0.224, 0.225],
    activation: Activation::Auto,
};

/// Longest edge of the stored coverage — capped so an AI mask stays small in the
/// stack (it is RLE'd on top of this). The shader resizes it to the render size.
pub const COVERAGE_EDGE: u32 = 1024;

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// ONNX Runtime's shared library, as this platform names it.
const ORT_LIBRARY: &str = if cfg!(windows) {
    "onnxruntime.dll"
} else if cfg!(target_os = "macos") {
    "libonnxruntime.dylib"
} else {
    "libonnxruntime.so"
};

/// The prefix of Microsoft's release package for this platform, unpacked in the
/// repo root for development (`onnxruntime-osx-arm64-1.24.4/`, …).
const ORT_PACKAGE: &str = if cfg!(windows) {
    "onnxruntime-win-x64"
} else if cfg!(target_os = "macos") {
    "onnxruntime-osx"
} else {
    "onnxruntime-linux"
};

/// Set by [`ensure_ort_dylib`], which every session builder calls first: this
/// process has loaded ONNX Runtime, so [`release_ort`] has something to do.
static ORT_USED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Points `ort` at the ONNX Runtime dylib if the caller has not set
/// `ORT_DYLIB_PATH`. Shared by every ONNX consumer (subject masking here, the
/// built-in tagger). Cheap and idempotent.
///
/// A Windows install ships the dll beside the exe, where the loader looks
/// anyway. A macOS app bundle carries it in `Contents/Frameworks`, which
/// `dlopen` of a bare name does **not** search, so that one is found here.
/// Everything after it is a dev convenience so the app "just runs" from source.
///
/// **The runtime the installer ships comes first**: `vendor/onnxruntime`, the
/// DirectML build. A downloaded `onnxruntime-win-x64-*` package in the repo
/// root is only a fallback. That one is the CUDA build, which has no DirectML,
/// so from source every model silently ran on the CPU. SigLIP 2's first load
/// then took minutes, and the dev app stopped behaving like the one people
/// install. macOS has no such split: Microsoft's one `onnxruntime-osx-arm64`
/// package already carries the CoreML provider.
/// Whether the build machine's own paths may be tried: a debug or developer
/// build, never a shipped one.
const DEV_PATHS: bool = cfg!(debug_assertions) || cfg!(feature = "dev-tools");

pub fn ensure_ort_dylib() {
    ORT_USED.store(true, std::sync::atomic::Ordering::Release);
    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        return;
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let bundled = std::env::current_exe()
        .ok()
        .filter(|_| cfg!(target_os = "macos"))
        .and_then(|exe| bundle_frameworks(&exe))
        .map(|dir| dir.join(ORT_LIBRARY));
    // A shipped build loads only the runtime it was installed with:
    // the paths below are the build machine's, and on the maintainer's own
    // PC the installed app quietly loaded the repo's copy instead, so the
    // shipped one was never exercised; on a machine where a release was
    // built at a drive root, any user could have planted one there.
    if !DEV_PATHS {
        let beside = std::env::current_exe()
            .ok()
            .filter(|_| !cfg!(target_os = "macos"))
            .and_then(|exe| exe.parent().map(|d| d.join(ORT_LIBRARY)));
        if let Some(candidate) = bundled.or(beside).filter(|c| c.exists()) {
            // Safe: called during model load, before any concurrent ort use.
            std::env::set_var("ORT_DYLIB_PATH", candidate);
        }
        return;
    }
    let vendored = manifest
        .join("vendor")
        .join("onnxruntime")
        .join(ORT_LIBRARY);
    for candidate in bundled.into_iter().chain([vendored]) {
        if candidate.exists() {
            // Safe: called during model load, before any concurrent ort use.
            std::env::set_var("ORT_DYLIB_PATH", candidate);
            return;
        }
    }
    let root = manifest.join("..");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    for e in entries.flatten() {
        if e.file_name().to_string_lossy().starts_with(ORT_PACKAGE) {
            let library = e.path().join("lib").join(ORT_LIBRARY);
            if library.exists() {
                // Safe: called during model load, before any concurrent ort use.
                std::env::set_var("ORT_DYLIB_PATH", library);
                return;
            }
        }
    }
}

/// `Contents/Frameworks` of the app bundle `exe` runs from, or `None` when it
/// does not run from one.
///
/// Only from inside a bundle. A dev build's executable sits in
/// `target/<profile>/`, where `../Frameworks` is whatever the last bundle build
/// copied into `target/Frameworks` — a stale runtime there would quietly shadow
/// the vendored one, as a leftover 1.22.0 once did.
fn bundle_frameworks(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    (macos.ends_with("MacOS") && contents.ends_with("Contents"))
        .then(|| contents.join("Frameworks"))
}

/// Where a development build has been told to run the app's own models, for
/// measuring them (`probe_ai`, docs/BENCHMARKS.md): the CPU alone, as on a PC
/// with no usable GPU, or DirectML on one adapter, such as the integrated GPU
/// of a machine that has two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceOverride {
    Cpu,
    Adapter(u32),
}

/// Reads `SOURCELIGHT_AI_DEVICE`: `cpu`, or a DirectML adapter index in
/// DXGI's plain order. Anything else is no override.
pub fn parse_device_override(value: &str) -> Option<DeviceOverride> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("cpu") {
        return Some(DeviceOverride::Cpu);
    }
    value.parse().ok().map(DeviceOverride::Adapter)
}

/// The override in force, read once. Only a development build reads it, like
/// `SOURCELIGHT_DATA_DIR`: a shipped app always picks for itself.
pub fn device_override() -> Option<DeviceOverride> {
    static OVERRIDE: std::sync::OnceLock<Option<DeviceOverride>> = std::sync::OnceLock::new();
    *OVERRIDE.get_or_init(|| {
        let found = std::env::var("SOURCELIGHT_AI_DEVICE")
            .ok()
            .filter(|_| DEV_PATHS)
            .and_then(|v| parse_device_override(&v));
        if let Some(device) = found {
            eprintln!("ai: models run on {device:?}, as SOURCELIGHT_AI_DEVICE says");
        }
        found
    })
}

/// Whether the models are to run on the CPU alone, without trying the GPU.
pub fn cpu_only() -> bool {
    device_override() == Some(DeviceOverride::Cpu)
}

/// The hardware execution providers worth asking for on this platform, best
/// first. The CPU is every caller's fallback after them.
///
/// Only providers this platform's ONNX Runtime can contain. `ort` finds each
/// provider's entry point by name in the loaded library, so asking the macOS
/// runtime for DirectML fails — and where that provider was required, the
/// whole GPU session failed with it. Windows gets DirectML (the runtime the
/// installer ships) and CUDA (the dev package); macOS gets CoreML, as an ML
/// Program, the format Apple has not deprecated.
///
/// CoreML is worth having for the face models: measured on an M4 over 73
/// photos, the same 31 faces in the same 16 people, in 24.9 s against 26.8 s
/// and with half the CPU time. It is not worth it for SigLIP 2, which the
/// built-in tagger therefore keeps on the CPU (see its `build_session`).
///
/// `required` makes the first one fail the session if it cannot be
/// registered, for a caller that needs to know the session is not on the GPU
/// (the built-in tagger builds its CPU session differently).
///
/// `coreml_cache` is the folder CoreML keeps this model's compile in (see
/// [`super::coreml_cache`]). Without one it compiles into the temp dir, and
/// only cleans up if the session is dropped.
#[cfg(feature = "ai")]
pub fn gpu_providers(
    required: bool,
    coreml_cache: Option<&std::path::Path>,
) -> Vec<ort::execution_providers::ExecutionProviderDispatch> {
    use ort::execution_providers::{
        coreml::CoreMLModelFormat, CUDAExecutionProvider, CoreMLExecutionProvider,
        DirectMLExecutionProvider, ExecutionProviderDispatch,
    };
    let mut providers: Vec<ExecutionProviderDispatch> = if cfg!(target_os = "macos") {
        let mut coreml =
            CoreMLExecutionProvider::default().with_model_format(CoreMLModelFormat::MLProgram);
        if let Some(dir) = coreml_cache {
            coreml = coreml.with_model_cache_dir(dir.to_string_lossy());
        }
        vec![coreml.build()]
    } else {
        // The high-performance GPU, not whichever drives the display: on a
        // laptop with two, that is the integrated one (`hardware::ai_gpu`).
        let (device, name) = crate::hardware::ai_gpu();
        let (device, name) = match device_override() {
            Some(DeviceOverride::Adapter(n)) => (n, "as SOURCELIGHT_AI_DEVICE says"),
            _ => (*device, name.as_str()),
        };
        static LOGGED: std::sync::Once = std::sync::Once::new();
        LOGGED.call_once(|| eprintln!("ai: DirectML on adapter {device} ({name})"));
        vec![
            DirectMLExecutionProvider::default()
                .with_device_id(device as i32)
                .build(),
            CUDAExecutionProvider::default().build(),
        ]
    };
    if required {
        providers[0] = providers[0].clone().error_on_failure();
    }
    providers
}

/// Who is waiting on a model, which decides how its threads share the CPU
/// ([`cpu_threads`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workload {
    /// Someone is waiting for the answer: Select Subject, Click to Select,
    /// noise reduction, magic erase, the words of a search.
    Interactive,
    /// Automatic analysis working through a library, the search index and
    /// the face pass, which nobody watches photo by photo.
    Background,
}

/// The thread settings every session of the app's own models gets, whichever
/// provider runs it (a GPU session still runs the odd node on its CPU pool).
/// Measured with SigLIP 2's vision tower on the CPU, on a machine with other
/// work running beside it, as any machine in use has (docs/BENCHMARKS.md,
/// "Slower machines"):
///
/// - **No spinning.** By default ONNX Runtime's pool threads spin while they
///   wait for their next piece of work, which is quickest on an idle machine
///   and ruinous on a busy one: they burn the cores the rest of the machine
///   wants, and each other's. On four cores a photo took 175 ms (p90 348)
///   spinning and 125 ms (p90 127) not; with more threads than free cores,
///   which is what ONNX Runtime's one-per-physical-core default amounts to
///   whenever anything else is running, 858 ms against 141. On all sixteen
///   threads, 131 ms against 91. With little else running, spinning is about
///   5% quicker (111 against 117 ms on four cores); a PC in use is the other
///   case far more often.
/// - **Background work below normal priority** ([`low_priority_threads`]),
///   so that on a machine whose cores automatic analysis fills, the window,
///   the webview and every interactive tool still get one the moment they
///   want it. When nothing else wants the cores it costs nothing: 91 ms
///   either way.
///
/// The thread count stays ONNX Runtime's default, one per physical core. A
/// thread fewer, to leave a core free, cost 20% on four cores (149 ms against
/// 125), and without spinning more threads than cores no longer hurts.
#[cfg(feature = "ai")]
pub fn cpu_threads(
    builder: ort::session::builder::SessionBuilder,
    workload: Workload,
) -> ort::Result<ort::session::builder::SessionBuilder> {
    let builder = builder
        .with_intra_op_spinning(false)?
        .with_inter_op_spinning(false)?;
    match workload {
        Workload::Interactive => Ok(builder),
        Workload::Background => low_priority_threads(builder),
    }
}

/// Gives a session a thread pool below normal priority
/// ([`crate::hardware::lower_this_thread`]): for the models automatic
/// analysis runs over a whole library, so that on a CPU they fill only the
/// time nothing else wants. ONNX Runtime's caller is one of its intra-op
/// threads too, so the loop calling the session lowers itself as well
/// ([`crate::hardware::BackgroundPriority`]).
#[cfg(feature = "ai")]
pub fn low_priority_threads(
    builder: ort::session::builder::SessionBuilder,
) -> ort::Result<ort::session::builder::SessionBuilder> {
    builder.with_thread_manager(BackgroundThreads)
}

/// How many pool threads background sessions have started: what the tests
/// check the thread manager by.
#[cfg(feature = "ai")]
pub fn background_threads_made() -> usize {
    BACKGROUND_THREADS.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(feature = "ai")]
static BACKGROUND_THREADS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Makes ONNX Runtime's pool threads for [`low_priority_threads`].
#[cfg(feature = "ai")]
struct BackgroundThreads;

#[cfg(feature = "ai")]
impl ort::environment::ThreadManager for BackgroundThreads {
    type Thread = std::thread::JoinHandle<()>;

    fn create(&mut self, worker: ort::environment::ThreadWorker) -> ort::Result<Self::Thread> {
        BACKGROUND_THREADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::thread::Builder::new()
            .name("ai-background".into())
            .spawn(move || {
                crate::hardware::lower_this_thread();
                worker.work();
            })
            .map_err(ort::Error::wrap)
    }

    fn join(thread: Self::Thread) -> ort::Result<()> {
        thread
            .join()
            .map_err(|_| ort::Error::new("a model thread panicked"))
    }
}

/// Loads `path` on this platform's hardware provider, or on the CPU alone if
/// that cannot load it. `what` names the model in the log line; `workload`
/// sets how its threads share the CPU ([`cpu_threads`]).
///
/// Registering a provider is not the same as it accepting the model. A provider
/// that registers fine can still fail to *compile* the graph, and that fails
/// the whole commit, not just its share of the nodes — CoreML in ONNX Runtime
/// 1.24.4 does exactly that with SigLIP 2's export. Without the second attempt
/// one model a provider dislikes would take its feature down on that platform.
#[cfg(feature = "ai")]
pub fn commit_on_gpu_or_cpu(
    path: &std::path::Path,
    what: &str,
    workload: Workload,
) -> anyhow::Result<ort::session::Session> {
    commit_on_gpu_or_cpu_with(path, what, workload, Ok)
}

/// Loads `path` for a tool someone is waiting on: on a Mac on the CPU alone,
/// elsewhere as [`commit_on_gpu_or_cpu`].
///
/// CoreML compiles a model the first time it meets it, and on an M4 that took
/// 30 s to three minutes for SAM 2's encoder and a minute for BiRefNet; even
/// compiled, loading took 7–31 s again each time Develop opened, and it can't
/// compile LaMa at all. On the CPU they load in a second and run nearly as
/// fast: SAM 2 encodes in 0.73 s against 0.3 s, BiRefNet in 6–8 s against 4.
/// Run on the performance cores ([`crate::hardware::performance_cores`]),
/// without spinning: the efficiency cores only slowed the passes down.
#[cfg(feature = "ai")]
pub fn commit_without_compile(
    path: &std::path::Path,
    what: &str,
) -> anyhow::Result<ort::session::Session> {
    if !cfg!(target_os = "macos") {
        return commit_on_gpu_or_cpu(path, what, Workload::Interactive);
    }
    commit_on_cpu(path)
}

/// Loads `path` on the CPU alone, for a tool someone is waiting on: for a
/// model no GPU provider runs (magic erase's), and on a Mac for the tools
/// [`commit_without_compile`] keeps from CoreML, on the performance cores.
#[cfg(feature = "ai")]
pub fn commit_on_cpu(path: &std::path::Path) -> anyhow::Result<ort::session::Session> {
    use anyhow::Context;
    use ort::execution_providers::CPUExecutionProvider;
    ensure_ort_dylib();
    let builder = ort::session::Session::builder()?;
    let builder = if cfg!(target_os = "macos") {
        match crate::hardware::performance_cores() {
            Some(cores) => builder
                .with_intra_threads(cores)?
                .with_intra_op_spinning(false)?,
            None => builder,
        }
    } else {
        cpu_threads(builder, Workload::Interactive)?
            .with_execution_providers([CPUExecutionProvider::default().build()])?
    };
    builder
        .commit_from_file(path)
        .with_context(|| format!("loading {}", path.display()))
}

/// [`commit_on_gpu_or_cpu`] with session options of the model's own, applied
/// to both attempts (the denoiser pins its free dimensions, which DirectML
/// needs).
#[cfg(feature = "ai")]
pub fn commit_on_gpu_or_cpu_with(
    path: &std::path::Path,
    what: &str,
    workload: Workload,
    configure: impl Fn(
        ort::session::builder::SessionBuilder,
    ) -> ort::Result<ort::session::builder::SessionBuilder>,
) -> anyhow::Result<ort::session::Session> {
    use anyhow::Context;
    use ort::execution_providers::CPUExecutionProvider;
    ensure_ort_dylib();
    let options = |builder| cpu_threads(builder, workload).and_then(&configure);
    let on_cpu = || {
        ort::session::Session::builder()
            .and_then(options)?
            .with_execution_providers([CPUExecutionProvider::default().build()])?
            .commit_from_file(path)
            .with_context(|| format!("loading {}", path.display()))
    };
    if cpu_only() {
        return on_cpu();
    }
    let cache = coreml_cache_entry(path, what);
    let mut providers = gpu_providers(false, cache.as_ref().map(|c| c.dir()));
    providers.push(CPUExecutionProvider::default().build());
    let gpu = ort::session::Session::builder()
        .and_then(options)?
        .with_execution_providers(providers)
        .and_then(|b| b.commit_from_file(path));
    match gpu {
        Ok(session) => {
            if let Some(Err(e)) = cache.map(|c| c.keep()) {
                eprintln!(
                    "{what}: CoreML's compile of {} not kept ({e})",
                    path.display()
                );
            }
            Ok(session)
        }
        Err(e) => {
            if let Some(cache) = cache {
                cache.discard();
            }
            eprintln!(
                "{what}: no GPU session for {} ({e}); using the CPU",
                path.display()
            );
            on_cpu()
        }
    }
}

/// The folder CoreML keeps its compiled models in, when the app has set one.
#[cfg(feature = "ai")]
static COREML_CACHE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Sets the folder CoreML keeps the models it compiles in: the app's cache
/// folder, at startup, before any model loads. A process that sets none (the
/// tests) uses `sourcelight-coreml` in the temp dir, one folder reused from run
/// to run.
#[cfg(feature = "ai")]
pub fn set_coreml_cache(dir: std::path::PathBuf) {
    let _ = COREML_CACHE.set(dir);
}

#[cfg(feature = "ai")]
fn coreml_cache_root() -> std::path::PathBuf {
    COREML_CACHE
        .get()
        .cloned()
        .unwrap_or_else(|| std::env::temp_dir().join("sourcelight-coreml"))
}

/// The folder CoreML compiles `model` into and, once it has, loads it from.
#[cfg(feature = "ai")]
pub fn coreml_cache_dir(model: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    ensure_ort_dylib();
    super::coreml_cache::entry_dir(&coreml_cache_root(), model, ort::info())
}

/// `model`'s folder in the CoreML cache, on a Mac. `None` elsewhere, or when
/// the folder cannot be made; CoreML then compiles into the temp dir as before.
#[cfg(feature = "ai")]
fn coreml_cache_entry(model: &std::path::Path, what: &str) -> Option<super::coreml_cache::Entry> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    super::coreml_cache::Entry::open(&coreml_cache_root(), model, ort::info())
        .map_err(|e| eprintln!("{what}: no CoreML cache for {} ({e})", model.display()))
        .ok()
}

/// Releases ONNX Runtime's environment. Call it once, as the process exits,
/// and build no session after it.
///
/// `ort` keeps the environment in a `static`, and Rust never drops statics, so
/// without this it is still alive when `exit()` runs ONNX Runtime's own C++
/// static destructors. On macOS one of those then locks a mutex another has
/// already destroyed, and the app aborts on its way out (`mutex lock failed:
/// Invalid argument`) — a crash report for every quit after any AI had run.
/// Releasing it first, while the runtime is still whole, leaves those
/// destructors nothing to tear down. A no-op if no model was ever loaded, so
/// quitting never loads the runtime just to release it.
#[cfg(feature = "ai")]
pub fn release_ort() {
    use ort::AsPointer;
    if !ORT_USED.swap(false, std::sync::atomic::Ordering::AcqRel) {
        return;
    }
    if let Ok(environment) = ort::environment::get_environment() {
        // SAFETY: the pointer is the live global environment, released exactly
        // once (the flag above), and nothing builds a session after this.
        unsafe { (ort::api().ReleaseEnv)(environment.ptr().cast_mut()) };
    }
}

/// Resizes and normalises an `w`×`h` RGBA8 frame into the model's planar NCHW
/// `f32` input of shape `[1, 3, size, size]` (R plane, then G, then B).
///
/// The frame is squished to a square, the standard way these matting models are
/// run; `postprocess` unsquishes the result back to the frame's aspect.
///
/// One pass over the output, the three channels together, from bilinear taps
/// worked out once per row and column (`mask::bilinear_taps`). It used to
/// split the frame into three planes, resize each with
/// `mask::resize_coverage` and normalise a third time: 106 ms of a Select
/// Subject that runs its model in 88 ms on a GPU, and the same again before
/// every Click to Select encode. The arithmetic per pixel is unchanged —
/// each channel is resized to a byte, then normalised — so the input is the
/// same, to the bit (`preprocess_matches_the_plane_by_plane_resize`).
pub fn preprocess(rgba: &[u8], w: u32, h: u32, cfg: &SegConfig) -> Vec<f32> {
    let size = cfg.size.max(1);
    let plane = (size * size) as usize;
    // Every byte's normalised value, per channel.
    let normalised: [[f32; 256]; 3] = std::array::from_fn(|c| {
        std::array::from_fn(|v| (v as f32 / 255.0 - cfg.mean[c]) / cfg.std[c])
    });
    let mut out = vec![0f32; 3 * plane];
    let (sw, sh) = (w as usize, h as usize);
    let n = sw * sh;
    if n == 0 {
        // A frame with no pixels resizes to black, as it always did.
        for (c, values) in normalised.iter().enumerate() {
            out[c * plane..(c + 1) * plane].fill(values[0]);
        }
        return out;
    }
    // A pixel past the end of a short buffer reads as black.
    let px: std::borrow::Cow<[u8]> = match rgba.get(..n * 4) {
        Some(all) => all.into(),
        None => {
            let mut padded = rgba.to_vec();
            padded.resize(n * 4, 0);
            padded.into()
        }
    };
    if (sw, sh) == (size as usize, size as usize) {
        for (i, p) in px.chunks_exact(4).enumerate() {
            for c in 0..3 {
                out[c * plane + i] = normalised[c][p[c] as usize];
            }
        }
        return out;
    }
    let xs = mask::bilinear_taps(w, size);
    for (oy, (y0, y1, ty)) in mask::bilinear_taps(h, size).into_iter().enumerate() {
        let (r0, r1) = (&px[y0 * sw * 4..][..sw * 4], &px[y1 * sw * 4..][..sw * 4]);
        for (ox, &(x0, x1, tx)) in xs.iter().enumerate() {
            let o = oy * size as usize + ox;
            for c in 0..3 {
                let top = r0[x0 * 4 + c] as f32 * (1.0 - tx) + r0[x1 * 4 + c] as f32 * tx;
                let bot = r1[x0 * 4 + c] as f32 * (1.0 - tx) + r1[x1 * 4 + c] as f32 * tx;
                let v = mask::round_to_byte(top * (1.0 - ty) + bot * ty);
                out[c * plane + o] = normalised[c][v as usize];
            }
        }
    }
    out
}

/// Turns a model matte (`size`×`size`, 0..1 or logits) into an R8 coverage at
/// `out_w`×`out_h`, unsquishing back to the frame's aspect.
pub fn postprocess(matte: &[f32], cfg: &SegConfig, out_w: u32, out_h: u32) -> Vec<u8> {
    let size = cfg.size.max(1);
    matte_to_coverage(matte, size, size, cfg, out_w, out_h)
}

/// The general form: a matte of an arbitrary `src_w`×`src_h` to R8 coverage at
/// `out_w`×`out_h`, applying `cfg.activation`.
///
/// Shared so every model turns a matte into coverage the same way — BiRefNet's
/// square output and SAM2's smaller mask grid differ only in their dimensions.
pub fn matte_to_coverage(
    matte: &[f32],
    src_w: u32,
    src_h: u32,
    cfg: &SegConfig,
    out_w: u32,
    out_h: u32,
) -> Vec<u8> {
    let (src_w, src_h) = (src_w.max(1), src_h.max(1));
    let plane = (src_w * src_h) as usize;
    let n = plane.min(matte.len());

    let use_sigmoid = match cfg.activation {
        Activation::Sigmoid => true,
        Activation::None => false,
        Activation::Auto => {
            // Probabilities sit in 0..1; anything well outside is logits.
            let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for &v in matte.iter().take(n) {
                lo = lo.min(v);
                hi = hi.max(v);
            }
            hi > 1.001 || lo < -0.001
        }
    };

    let mut cov = vec![0u8; plane];
    for (dst, &v) in cov.iter_mut().zip(matte.iter()).take(n) {
        let p = if use_sigmoid { sigmoid(v) } else { v };
        *dst = mask::round_to_byte(p.clamp(0.0, 1.0) * 255.0);
    }
    mask::resize_coverage(&cov, src_w, src_h, out_w, out_h)
}

/// The coverage size for a frame of `w`×`h`, capped to [`COVERAGE_EDGE`] on the
/// long edge so a stored AI mask stays small.
pub fn coverage_size(w: u32, h: u32) -> (u32, u32) {
    let long = w.max(h).max(1);
    if long <= COVERAGE_EDGE {
        return (w.max(1), h.max(1));
    }
    let s = COVERAGE_EDGE as f32 / long as f32;
    (
        (w as f32 * s).round().max(1.0) as u32,
        (h as f32 * s).round().max(1.0) as u32,
    )
}

/// Builds a single-component AI mask from an R8 coverage buffer.
pub fn ai_mask(label: &str, width: u32, height: u32, coverage: &[u8]) -> Mask {
    Mask {
        components: vec![MaskComponent::new(MaskShape::Ai {
            label: label.to_string(),
            width,
            height,
            rle: mask::encode_ai(coverage),
        })],
        adjustments: mask::LocalAdjustments::default(),
        // Seed the editable name from the subject label, so a fresh AI mask reads
        // as "subject" rather than a generic index until the user renames it.
        name: Some(label.to_string()),
    }
}

/// Subject segmentation of `input` — the open photo rendered at
/// [`BIREFNET`]'s size by `DevelopState::selection_input` — as a ready-to-add
/// AI mask. Runs with no lock held.
///
/// Only real with `--features ai`; otherwise it reports the build is missing
/// AI so the command surfaces a clear message rather than a silent no-op.
/// The subject model, kept between clicks: every click built a new
/// session, 224 MB read from disk and, on DirectML, compiled again, before
/// any work. But only for [`SUBJECT_KEPT_FOR`] after it last ran: DirectML
/// holds every buffer a run allocated until the session is dropped (about
/// 14 GB of a 16 GB card for BiRefNet in fp32 at 1024²), and a model kept until
/// Develop closed left nothing for the vision model, exports or anything else.
#[cfg(feature = "ai")]
static SUBJECT: std::sync::Mutex<Option<(std::path::PathBuf, Segmenter, std::time::Instant)>> =
    std::sync::Mutex::new(None);

/// Long enough for a few masks in a row to skip the reload.
#[cfg(feature = "ai")]
const SUBJECT_KEPT_FOR: std::time::Duration = std::time::Duration::from_secs(30);

/// Drops the kept subject model: when its file changes or goes, Develop
/// closes, or the vision model wants the graphics card.
#[cfg(feature = "ai")]
pub fn forget_subject_model() {
    *crate::sync::lock_recover(&SUBJECT) = None;
}

/// Drops the subject model once it has gone [`SUBJECT_KEPT_FOR`] without a run.
#[cfg(feature = "ai")]
fn forget_subject_model_when_idle() {
    std::thread::spawn(|| {
        std::thread::sleep(SUBJECT_KEPT_FOR);
        let mut kept = crate::sync::lock_recover(&SUBJECT);
        if kept
            .as_ref()
            .is_some_and(|(_, _, ran)| ran.elapsed() >= SUBJECT_KEPT_FOR)
        {
            *kept = None;
            eprintln!("ai mask: subject model released");
        }
    });
}

/// `stage` hears `"loading"` when the model has to be loaded first, which
/// takes longer than the run the first time, and `"subject"` as it runs.
#[cfg(feature = "ai")]
pub fn select_subject(
    model_path: &std::path::Path,
    input: &crate::develop::session::SelectionInput,
    stage: impl Fn(&str),
) -> anyhow::Result<Mask> {
    let mut loaded = crate::sync::lock_recover(&SUBJECT);
    if loaded
        .as_ref()
        .is_none_or(|(path, _, _)| path != model_path)
    {
        stage("loading");
        *loaded = Some((
            model_path.to_path_buf(),
            Segmenter::load(model_path, BIREFNET)?,
            std::time::Instant::now(),
        ));
    }
    let (_, seg, ran) = loaded.as_mut().expect("loaded just above");
    stage("subject");
    let segmented = seg.segment(&input.pixels, input.width, input.height);
    *ran = std::time::Instant::now();
    drop(loaded);
    forget_subject_model_when_idle();
    let (cw, ch, coverage) = segmented?;
    Ok(ai_mask("Subject", cw, ch, &coverage))
}

#[cfg(not(feature = "ai"))]
pub fn select_subject(
    _model_path: &std::path::Path,
    _input: &crate::develop::session::SelectionInput,
    _stage: impl Fn(&str),
) -> anyhow::Result<Mask> {
    anyhow::bail!("this build was compiled without AI — rebuild with `--features ai`")
}

// ------------------------------------------------------------------------
// ONNX session — only when built with `--features ai`.
// ------------------------------------------------------------------------
//
// NOTE: written against `ort` 2.0. `ort`'s API is still stabilising across rc
// releases; if a call below does not resolve on first build, check it against
// the `ort` version in Cargo.toml — the shape (build session with EPs →
// `run` with a named input → extract the f32 output tensor) is stable, only the
// spelling moves. This block is deliberately small for exactly that reason.
#[cfg(feature = "ai")]
pub use imp::Segmenter;

#[cfg(feature = "ai")]
mod imp {
    use super::*;
    use anyhow::{Context, Result};

    /// A loaded segmentation model.
    pub struct Segmenter {
        session: ort::session::Session,
        cfg: SegConfig,
        input_name: String,
        path: std::path::PathBuf,
        /// Whether the session is the CPU's already, with nowhere further to
        /// fall back to.
        on_cpu: bool,
    }

    impl Segmenter {
        /// Loads an ONNX model on the GPU providers on Windows (DirectML,
        /// CUDA), or the CPU if they can't load it; on a Mac on the CPU, which
        /// is quicker there in all but a long session ([`super::commit_without_compile`]).
        pub fn load(model_path: &std::path::Path, cfg: SegConfig) -> Result<Self> {
            let session = super::commit_without_compile(model_path, "segmenter")
                .with_context(|| format!("loading model {}", model_path.display()))?;
            let input_name = input_name(&session)?;
            Ok(Self {
                session,
                cfg,
                input_name,
                path: model_path.to_path_buf(),
                on_cpu: cfg!(target_os = "macos") || super::cpu_only(),
            })
        }

        /// Segments an `w`×`h` RGBA8 frame into an R8 coverage buffer, returned
        /// with its (capped) dimensions.
        ///
        /// A GPU that loaded the model can still fail to run it, and then the
        /// CPU runs it instead, from then on: BiRefNet in fp32 ran out of
        /// memory on integrated Radeon graphics at its first pass, which left
        /// Select Subject failing every time on such a PC.
        pub fn segment(&mut self, rgba: &[u8], w: u32, h: u32) -> Result<(u32, u32, Vec<u8>)> {
            let matte = match self.run(preprocess(rgba, w, h, &self.cfg)) {
                Ok(matte) => matte,
                Err(e) if !self.on_cpu => {
                    eprintln!(
                        "segmenter: the GPU could not run {} ({e:#}); using the CPU",
                        self.path.display()
                    );
                    self.session = super::commit_on_cpu(&self.path)?;
                    self.input_name = input_name(&self.session)?;
                    self.on_cpu = true;
                    // Made again rather than kept: copying 12 MB at every
                    // click for a fallback that is rare would cost more.
                    self.run(preprocess(rgba, w, h, &self.cfg))?
                }
                Err(e) => return Err(e),
            };
            let (ow, oh) = coverage_size(w, h);
            Ok((ow, oh, postprocess(&matte, &self.cfg, ow, oh)))
        }

        fn run(&mut self, input: Vec<f32>) -> Result<Vec<f32>> {
            let size = self.cfg.size as i64;
            let tensor = ort::value::Value::from_array(([1_i64, 3, size, size], input))?;
            let outputs = self
                .session
                .run(ort::inputs![self.input_name.as_str() => tensor])?;
            let (_shape, matte) = outputs[0].try_extract_tensor::<f32>()?;
            Ok(matte.to_vec())
        }
    }

    fn input_name(session: &ort::session::Session) -> Result<String> {
        session
            .inputs
            .first()
            .map(|i| i.name.clone())
            .context("model has no inputs")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_app_bundle_has_frameworks_to_look_in() {
        use std::path::Path;
        assert_eq!(
            bundle_frameworks(Path::new(
                "/Applications/SourceLight.app/Contents/MacOS/SourceLight"
            )),
            Some(Path::new("/Applications/SourceLight.app/Contents/Frameworks").into())
        );
        // A dev build, whose `../Frameworks` is a bundle build's leftovers.
        assert_eq!(
            bundle_frameworks(Path::new(
                "/src/sourcelight/src-tauri/target/debug/sourcelight"
            )),
            None
        );
        assert_eq!(bundle_frameworks(Path::new("sourcelight")), None);
    }

    #[test]
    fn a_development_build_can_be_told_where_the_models_run() {
        assert_eq!(parse_device_override("cpu"), Some(DeviceOverride::Cpu));
        assert_eq!(parse_device_override(" CPU "), Some(DeviceOverride::Cpu));
        assert_eq!(parse_device_override("1"), Some(DeviceOverride::Adapter(1)));
        assert_eq!(parse_device_override("gpu"), None);
        assert_eq!(parse_device_override(""), None);
    }

    #[test]
    fn preprocess_lays_out_nchw_and_normalises() {
        // A 1x1 mid-grey pixel, resized up to the model size.
        let cfg = SegConfig {
            size: 2,
            ..BIREFNET
        };
        let rgba = [128u8, 128, 128, 255];
        let t = preprocess(&rgba, 1, 1, &cfg);
        assert_eq!(t.len(), 3 * 2 * 2, "NCHW length is 3*size*size");
        // Every R-plane entry is the same normalised grey.
        let expect_r = (128.0 / 255.0 - cfg.mean[0]) / cfg.std[0];
        for &v in t.iter().take(4) {
            assert!((v - expect_r).abs() < 1e-4, "R plane normalised");
        }
        // Planes are contiguous and use their own channel's mean/std.
        let expect_b = (128.0 / 255.0 - cfg.mean[2]) / cfg.std[2];
        assert!(
            (t[8] - expect_b).abs() < 1e-4,
            "B plane starts at 2*size*size"
        );
    }

    /// The fused pass gives the model the same input, to the bit, as the
    /// plane-by-plane resize it replaced.
    #[test]
    fn preprocess_matches_the_plane_by_plane_resize() {
        fn plane_by_plane(rgba: &[u8], w: u32, h: u32, cfg: &SegConfig) -> Vec<f32> {
            let size = cfg.size.max(1);
            let n = (w * h) as usize;
            let channel = |c: usize| -> Vec<u8> {
                (0..n)
                    .map(|i| rgba.get(i * 4 + c).copied().unwrap_or(0))
                    .collect()
            };
            let planes: Vec<Vec<u8>> = (0..3)
                .map(|c| mask::resize_coverage(&channel(c), w, h, size, size))
                .collect();
            let plane = (size * size) as usize;
            let mut out = vec![0f32; 3 * plane];
            for c in 0..3 {
                for i in 0..plane {
                    out[c * plane + i] = (planes[c][i] as f32 / 255.0 - cfg.mean[c]) / cfg.std[c];
                }
            }
            out
        }
        let mut seed = 11u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 24) as u8
        };
        for (w, h, size) in [(160, 107, 64), (64, 64, 64), (33, 90, 50), (5, 3, 8)] {
            let cfg = SegConfig { size, ..BIREFNET };
            let rgba: Vec<u8> = (0..w * h * 4).map(|_| next()).collect();
            assert_eq!(
                preprocess(&rgba, w, h, &cfg),
                plane_by_plane(&rgba, w, h, &cfg),
                "{w}x{h} -> {size}"
            );
            // A short buffer reads as black past its end.
            let short = &rgba[..rgba.len() / 2];
            assert_eq!(
                preprocess(short, w, h, &cfg),
                plane_by_plane(short, w, h, &cfg)
            );
        }
        let cfg = SegConfig {
            size: 4,
            ..BIREFNET
        };
        assert_eq!(preprocess(&[], 0, 0, &cfg), plane_by_plane(&[], 0, 0, &cfg));
    }

    #[test]
    fn postprocess_maps_matte_to_coverage_and_resizes() {
        let cfg = SegConfig {
            size: 2,
            activation: Activation::None,
            ..BIREFNET
        };
        // Full matte -> full coverage, at the requested output size.
        let cov = postprocess(&[1.0, 1.0, 1.0, 1.0], &cfg, 4, 3);
        assert_eq!(cov.len(), 12);
        assert!(cov.iter().all(|&c| c > 250), "matte 1.0 -> ~255 coverage");
        // Empty matte -> empty coverage.
        let cov0 = postprocess(&[0.0; 4], &cfg, 2, 2);
        assert!(cov0.iter().all(|&c| c == 0));
    }

    #[test]
    fn sigmoid_export_is_applied_when_configured() {
        let cfg = SegConfig {
            size: 1,
            activation: Activation::Sigmoid,
            ..BIREFNET
        };
        // Logit 0 -> 0.5 -> ~128.
        let cov = postprocess(&[0.0], &cfg, 1, 1);
        assert!(
            (cov[0] as i32 - 128).abs() <= 2,
            "sigmoid(0)=0.5, got {}",
            cov[0]
        );
    }

    #[test]
    fn auto_activation_detects_logits_vs_probabilities() {
        let cfg = SegConfig {
            size: 2,
            activation: Activation::Auto,
            ..BIREFNET
        };
        // Probabilities already in 0..1 are clamped, not squashed.
        let probs = postprocess(&[0.0, 0.25, 0.75, 1.0], &cfg, 2, 2);
        assert_eq!(probs[3], 255, "a 1.0 probability stays full");
        assert!((probs[1] as i32 - 64).abs() <= 2, "0.25 stays 0.25");
        // Logits (outside 0..1) get a sigmoid: -5 -> ~0, 0 -> ~128, 5 -> ~255.
        let logits = postprocess(&[-5.0, 0.0, 5.0, 0.0], &cfg, 2, 2);
        assert!(logits[0] < 10, "sigmoid(-5) ~ 0, got {}", logits[0]);
        assert!(
            (logits[1] as i32 - 128).abs() <= 3,
            "sigmoid(0) ~ 128, got {}",
            logits[1]
        );
        assert!(logits[2] > 245, "sigmoid(5) ~ 255, got {}", logits[2]);
    }

    #[test]
    fn coverage_size_caps_the_long_edge() {
        assert_eq!(
            coverage_size(800, 600),
            (800, 600),
            "small frames untouched"
        );
        let (w, h) = coverage_size(6000, 4000);
        assert_eq!(w, COVERAGE_EDGE, "long edge capped");
        assert_eq!(h, 683, "aspect preserved");
    }

    #[test]
    fn ai_mask_round_trips_its_coverage() {
        let cov = vec![0u8, 255, 255, 0];
        let m = ai_mask("Subject", 2, 2, &cov);
        assert_eq!(m.components.len(), 1);
        match &m.components[0].shape {
            MaskShape::Ai {
                label,
                width,
                height,
                rle,
            } => {
                assert_eq!(label, "Subject");
                assert_eq!((*width, *height), (2, 2));
                assert_eq!(
                    mask::decode_ai(rle),
                    cov,
                    "coverage survives the round trip"
                );
            }
            _ => panic!("expected an AI shape"),
        }
    }
}
