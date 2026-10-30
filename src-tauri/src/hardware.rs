//! A quick capability probe, so the app can tell a user what on-device AI is
//! realistic on their machine before they commit to a multi-hundred-MB download.
//!
//! The single biggest factor is whether there's a **discrete GPU**: the develop
//! pipeline already runs on it via wgpu, and the AI models accelerate on the same
//! hardware (DirectML on Windows, CoreML on macOS). So the probe reads the GPU
//! wgpu picks plus the CPU thread count and turns them into a plain recommendation
//! — no heavyweight system-info dependency.
//!
//! Apple silicon is the exception to "integrated means small": wgpu reports its
//! GPU as integrated, but it shares the whole of the Mac's unified memory, so it
//! is graded by that memory rather than put in the compact tier.
//!
//! It also answers the other thing worth asking before a big download or a big
//! export: how much room is left on the drive ([`free_space`]). Memory and
//! disk are the two questions with no portable API, so each has a small
//! per-OS function behind a stub and the rest of the module stays `cfg`-free.

use serde::Serialize;

#[derive(Serialize, Clone, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HardwareReport {
    /// The GPU wgpu selected (e.g. "NVIDIA GeForce RTX 5070 Ti").
    pub gpu_name: String,
    /// "discrete" | "integrated" | "cpu" | "other" — the headline capability.
    pub gpu_kind: String,
    /// The graphics backend in use (DirectX 12, Metal, …) — also the AI
    /// acceleration path.
    pub backend: String,
    pub cpu_threads: u32,
    /// Dedicated GPU memory in MB, when it could be read (Windows/DXGI). This is
    /// what actually gates which vision models fit, so it drives the per-model
    /// recommendations; `None` means "unknown", not "zero".
    pub vram_mb: Option<u32>,
    /// Total system RAM in MB, when readable — the ceiling for CPU-offloaded
    /// inference when there's no usable GPU.
    pub ram_mb: Option<u32>,
    /// The recommended model tier, and a one-line reason ready to display.
    pub tier: ModelTier,
    pub advice: String,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ModelTier {
    /// A discrete GPU, or Apple silicon — the full models run.
    Full,
    /// Integrated/virtual graphics — models run, but a compact build fits better.
    Compact,
    /// No usable GPU acceleration — AI runs on the CPU and will be slow.
    Cpu,
}

/// The recommendation, kept pure so the mapping is a unit test rather than a
/// guess baked into the probe. `unified_memory` is Apple silicon: an
/// integrated GPU that can use most of the Mac's memory, which runs every
/// model the catalogue lists (see `docs/MACOS.md` for the measurements).
pub fn recommend(gpu_kind: &str, cpu_threads: u32, unified_memory: bool) -> (ModelTier, String) {
    match gpu_kind {
        "discrete" => (
            ModelTier::Full,
            "Discrete GPU detected — the full models will run comfortably.".to_string(),
        ),
        "integrated" if unified_memory => (
            ModelTier::Full,
            "Apple silicon — the GPU shares the Mac's memory, so the full models run.".to_string(),
        ),
        "integrated" => (
            ModelTier::Compact,
            "Integrated graphics — the models run, though a compact build is a better fit."
                .to_string(),
        ),
        _ => {
            let tail = if cpu_threads >= 8 {
                "AI will run on the CPU — usable, but slower than on a GPU."
            } else {
                "AI will run on the CPU and is likely to be slow on this machine."
            };
            (ModelTier::Cpu, format!("No discrete GPU detected — {tail}"))
        }
    }
}

/// The GPU AI models should run on, as ONNX Runtime's DirectML provider
/// counts them, and its name for the log.
///
/// DirectML's `device_id` opens adapter `EnumAdapters1(device_id)`: DXGI's
/// plain order, which puts first the GPU the main display is wired to. On a
/// laptop with integrated and discrete graphics that is the integrated one,
/// so every AI model (faces, search, subject selection, noise reduction) ran
/// there while the discrete GPU idled, and Develop's own rendering, which asks
/// wgpu for the high-performance adapter, used the other. This asks DXGI for
/// the high-performance adapter too, skipping software ones, and finds its
/// place in the plain order by LUID. Index 0, the old behaviour, when there is
/// no such adapter or the lookup fails (DXGI 1.6 needs Windows 10 1803).
#[cfg(windows)]
pub fn ai_gpu() -> &'static (u32, String) {
    static GPU: std::sync::OnceLock<(u32, String)> = std::sync::OnceLock::new();
    GPU.get_or_init(|| high_performance_adapter().unwrap_or((0, String::new())))
}

#[cfg(not(windows))]
pub fn ai_gpu() -> &'static (u32, String) {
    static GPU: (u32, String) = (0, String::new());
    &GPU
}

#[cfg(windows)]
fn high_performance_adapter() -> Option<(u32, String)> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory6, DXGI_ADAPTER_FLAG_SOFTWARE,
        DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
    };
    // SAFETY: plain DXGI queries on interfaces this function owns.
    unsafe {
        let factory: IDXGIFactory6 = CreateDXGIFactory1().ok()?;
        let software = DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32;
        let mut n = 0;
        let preferred = loop {
            let adapter: IDXGIAdapter1 = factory
                .EnumAdapterByGpuPreference(n, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE)
                .ok()?;
            let desc = adapter.GetDesc1().ok()?;
            if desc.Flags & software == 0 {
                break desc;
            }
            n += 1;
        };
        let name = String::from_utf16_lossy(&preferred.Description)
            .trim_end_matches('\0')
            .to_string();
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            if adapter
                .GetDesc1()
                .is_ok_and(|d| d.AdapterLuid == preferred.AdapterLuid)
            {
                return Some((i, name));
            }
            i += 1;
        }
        None
    }
}

/// Whether the app's own models have a DirectML GPU to run on: `Some(true)`
/// on a Windows PC with a hardware graphics adapter, `Some(false)` on one with
/// none (a virtual machine, a server, a PC too old for DirectX 12, whose only
/// adapter is Microsoft's software renderer, which DirectML refuses), and
/// `None` elsewhere, where each model's provider is its own business (a Mac).
///
/// It picks the precision a model is downloaded in (`ai_models::runs_on`):
/// half-precision weights are what a GPU runs best and what a PC's CPU runs
/// worst. A development build told to keep the models on the CPU
/// (`develop::ai::device_override`) answers `Some(false)`, so that it measures
/// what such a PC gets.
pub fn directml_gpu() -> Option<bool> {
    if !cfg!(windows) {
        return None;
    }
    if crate::develop::ai::cpu_only() {
        return Some(false);
    }
    // DXGI that cannot be asked counts as a GPU: DirectML is then tried, as
    // it always was, and the CPU is its fallback.
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    Some(*FOUND.get_or_init(|| hardware_adapter_found().unwrap_or(true)))
}

/// Whether DXGI lists a hardware adapter; `None` when it cannot be asked.
#[cfg(windows)]
fn hardware_adapter_found() -> Option<bool> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
    };
    // SAFETY: plain DXGI queries on interfaces this function owns.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let software = DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32;
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            if adapter.GetDesc1().is_ok_and(|d| d.Flags & software == 0) {
                return Some(true);
            }
            i += 1;
        }
        Some(false)
    }
}

#[cfg(not(windows))]
fn hardware_adapter_found() -> Option<bool> {
    None
}

/// The AI GPU's video memory as the session log notes it when a model
/// loads: ", video memory: X available to the app, Y used by it", or nothing
/// where it can't be read. Another process's share, such as the vision
/// model's server, shows as a smaller budget.
pub fn gpu_note() -> String {
    gpu_memory()
        .map(|(budget, used)| {
            format!(
                ", video memory: {} available to the app, {} used by it",
                human_bytes(budget),
                human_bytes(used)
            )
        })
        .unwrap_or_default()
}

/// (budget, current use) of the AI GPU's own memory for this process, from
/// DXGI. `None` off Windows and when the adapter can't be asked.
#[cfg(windows)]
fn gpu_memory() -> Option<(u64, u64)> {
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter3, IDXGIFactory1, DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
        DXGI_QUERY_VIDEO_MEMORY_INFO,
    };
    let (index, _) = ai_gpu();
    // SAFETY: plain DXGI queries on interfaces this function owns.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let adapter: IDXGIAdapter3 = factory.EnumAdapters1(*index).ok()?.cast().ok()?;
        let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
        adapter
            .QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info)
            .ok()?;
        Some((info.Budget, info.CurrentUsage))
    }
}

#[cfg(not(windows))]
fn gpu_memory() -> Option<(u64, u64)> {
    None
}

fn cpu_threads() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1)
}

/// Lowers the calling thread's CPU priority for good: for a thread that only
/// ever does background work, such as ONNX Runtime's pool for a model run by
/// automatic analysis (`develop::ai::low_priority_threads`).
///
/// One step below the background-thread guard ([`BackgroundPriority`]) would
/// make no difference: what matters is being below *normal*, where the
/// window, the webview and every interactive tool run, so that on a machine
/// whose cores the models fill they still get a core the moment they want one.
/// Windows only for now; elsewhere a no-op (see [`BackgroundPriority`]).
pub fn lower_this_thread() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_LOWEST,
        };
        // SAFETY: the pseudo-handle of the calling thread, always valid.
        let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST) };
    }
}

/// The calling thread at below-normal CPU priority until this drops, then
/// back to what it was: for the loop of a background pass (search indexing,
/// faces), which runs on a pooled thread that must not stay lowered, and
/// which takes part in its own model runs (ONNX Runtime's caller is one of
/// its intra-op threads).
///
/// Windows only. Linux lets a process lower a thread's priority but not, by
/// default, raise it again, so a pooled thread could not be given back as it
/// was; a Mac runs its background models through CoreML or Apple's own
/// scheduling, and was not measured. Both are no-ops.
pub struct BackgroundPriority {
    #[cfg(windows)]
    previous: Option<i32>,
}

impl BackgroundPriority {
    pub fn begin() -> Self {
        #[cfg(windows)]
        {
            use windows::Win32::System::Threading::{
                GetCurrentThread, GetThreadPriority, SetThreadPriority, THREAD_PRIORITY_LOWEST,
            };
            // SAFETY: the pseudo-handle of the calling thread, always valid.
            let previous = unsafe { GetThreadPriority(GetCurrentThread()) };
            // THREAD_PRIORITY_ERROR_RETURN: nothing to restore.
            let previous = (previous != i32::MAX).then_some(previous);
            // SAFETY: as above.
            let lowered = previous.is_some()
                && unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST) }.is_ok();
            Self {
                previous: previous.filter(|_| lowered),
            }
        }
        #[cfg(not(windows))]
        Self {}
    }
}

impl Drop for BackgroundPriority {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(previous) = self.previous {
            use windows::Win32::System::Threading::{
                GetCurrentThread, SetThreadPriority, THREAD_PRIORITY,
            };
            // SAFETY: the pseudo-handle of the calling thread, always valid.
            let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY(previous)) };
        }
    }
}

/// How many performance cores an Apple silicon Mac has (4 on an M4, 10 on
/// an M4 Pro), or `None` where the split isn't known. A model run on the CPU
/// goes no faster for the efficiency cores: on an M4, LaMa took 1.53 s a pass
/// on the four performance cores against 1.73 s on all ten.
#[cfg(target_os = "macos")]
pub fn performance_cores() -> Option<usize> {
    let mut n: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>();
    // SAFETY: a read of one integer into a buffer of its size.
    let ok = unsafe {
        libc::sysctlbyname(
            c"hw.perflevel0.physicalcpu".as_ptr(),
            (&mut n as *mut libc::c_int).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } == 0;
    (ok && n > 0).then_some(n as usize)
}

#[cfg(not(target_os = "macos"))]
pub fn performance_cores() -> Option<usize> {
    None
}

/// (dedicated VRAM MB, total RAM MB), each `None` when it can't be read. This is
/// the one platform-specific corner of the probe — VRAM has no portable API, so
/// it comes from DXGI on Windows, from total memory on a Mac, and is unknown
/// elsewhere. Kept behind a stub so the rest of the module stays cfg-free.
#[cfg(windows)]
fn probe_memory() -> (Option<u32>, Option<u32>) {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    // The discrete GPU is the adapter with the most dedicated memory; take the
    // max across all adapters so an integrated one never shadows it.
    // SAFETY: plain DXGI queries on interfaces this function owns.
    let vram_mb = unsafe {
        CreateDXGIFactory1::<IDXGIFactory1>()
            .ok()
            .and_then(|factory| {
                let mut best: u64 = 0;
                let mut i = 0u32;
                while let Ok(adapter) = factory.EnumAdapters1(i) {
                    if let Ok(desc) = adapter.GetDesc1() {
                        best = best.max(desc.DedicatedVideoMemory as u64);
                    }
                    i += 1;
                }
                (best > 0).then_some((best / (1024 * 1024)) as u32)
            })
    };

    // SAFETY: an out-parameter struct whose length field says its size.
    let ram_mb = unsafe {
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        GlobalMemoryStatusEx(&mut status)
            .is_ok()
            .then_some((status.ullTotalPhys / (1024 * 1024)) as u32)
    };

    (vram_mb, ram_mb)
}

/// macOS is unified memory: CPU and GPU share one pool, so the "VRAM" budget is
/// a large fraction of total RAM. Read `hw.memsize` via `sysctl` (no extra
/// crate), report ~70% as usable GPU memory — the OS and other apps need the
/// rest — and the full amount as RAM.
///
/// Checked on a 16 GB M4: this gives 11.2 GB, where Metal's own
/// `recommendedMaxWorkingSetSize` is 11.8 GB and Ollama reports 11.3 GB
/// available. A little under the real figure is the right side to err on,
/// since the app's own work shares the same memory.
#[cfg(target_os = "macos")]
fn probe_memory() -> (Option<u32>, Option<u32>) {
    let total_mb = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|bytes| (bytes / (1024 * 1024)) as u32);
    match total_mb {
        Some(mb) => (Some((mb as u64 * 7 / 10) as u32), Some(mb)),
        None => (None, None),
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn probe_memory() -> (Option<u32>, Option<u32>) {
    (None, None)
}

/// Whether the internet connection is metered — a phone hotspot, a capped
/// mobile plan, roaming, or over its data limit — so a model download should
/// ask first instead of just starting. `None` when it can't be told, which the
/// caller treats as metered: asking needlessly costs a click, not a bill.
///
/// Windows only for now. It follows Microsoft's guidance for the cost types:
/// only `Unrestricted` (and `Unknown`, which Windows reports for connections
/// it has no cost information about, such as most Ethernet) downloads freely.
#[cfg(windows)]
pub fn metered() -> Option<bool> {
    use windows::Networking::Connectivity::{NetworkCostType, NetworkInformation};
    let cost = NetworkInformation::GetInternetConnectionProfile()
        .ok()?
        .GetConnectionCost()
        .ok()?;
    let kind = cost.NetworkCostType().ok()?;
    let capped = cost.Roaming().unwrap_or(false)
        || cost.OverDataLimit().unwrap_or(false)
        || cost.ApproachingDataLimit().unwrap_or(false);
    Some(
        capped
            || !matches!(
                kind,
                NetworkCostType::Unrestricted | NetworkCostType::Unknown
            ),
    )
}

/// macOS asks Network.framework, as its own apps do: a path is *expensive* over
/// cellular or a phone hotspot, and *constrained* when the user has turned on
/// Low Data Mode. Either means ask first.
///
/// A path monitor reports the current path once as soon as it starts, on a
/// dispatch queue of its own, so this waits for that first report — briefly;
/// a monitor that stays silent is `None`, never a guess — and then cancels it.
/// No network at all is `None` too: the download would fail regardless.
#[cfg(target_os = "macos")]
pub fn metered() -> Option<bool> {
    use std::ffi::{c_char, c_void};
    use std::time::Duration;

    #[link(name = "Network", kind = "framework")]
    extern "C" {
        fn nw_path_monitor_create() -> *mut c_void;
        fn nw_path_monitor_set_queue(monitor: *mut c_void, queue: *mut c_void);
        fn nw_path_monitor_set_update_handler(
            monitor: *mut c_void,
            handler: &block2::Block<dyn Fn(*mut c_void)>,
        );
        fn nw_path_monitor_start(monitor: *mut c_void);
        fn nw_path_monitor_cancel(monitor: *mut c_void);
        fn nw_path_get_status(path: *mut c_void) -> i32;
        fn nw_path_is_expensive(path: *mut c_void) -> bool;
        fn nw_path_is_constrained(path: *mut c_void) -> bool;
        fn nw_release(object: *mut c_void);
    }
    extern "C" {
        fn dispatch_queue_create(label: *const c_char, attr: *mut c_void) -> *mut c_void;
        fn dispatch_release(object: *mut c_void);
    }
    /// `nw_path_status_satisfied`: there is a usable route.
    const SATISFIED: i32 = 1;

    let (tx, rx) = std::sync::mpsc::channel();
    // SAFETY: the path is only read inside the handler, for the duration of the
    // call, as Network.framework documents.
    let handler = block2::RcBlock::new(move |path: *mut c_void| {
        // SAFETY: as above.
        let reading = unsafe {
            (nw_path_get_status(path) == SATISFIED)
                .then(|| nw_path_is_expensive(path) || nw_path_is_constrained(path))
        };
        let _ = tx.send(reading);
    });
    // SAFETY: each object is created here, used only through these calls, and
    // released once. The monitor copies the handler block, so it may outlive
    // `handler`; cancelling stops further calls before the release.
    unsafe {
        let queue = dispatch_queue_create(c"sourcelight.metered".as_ptr(), std::ptr::null_mut());
        let monitor = nw_path_monitor_create();
        if queue.is_null() || monitor.is_null() {
            return None;
        }
        nw_path_monitor_set_queue(monitor, queue);
        nw_path_monitor_set_update_handler(monitor, &handler);
        nw_path_monitor_start(monitor);
        let reading = rx.recv_timeout(Duration::from_secs(2)).ok().flatten();
        nw_path_monitor_cancel(monitor);
        nw_release(monitor);
        dispatch_release(queue);
        reading
    }
}

/// Elsewhere, unknown.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn metered() -> Option<bool> {
    None
}

/// Whether the machine is running on its battery, for automatic analysis to
/// hold the vision model — seconds of GPU a photo — until it is plugged in.
/// A laptop left describing a library on battery is a flat laptop.
///
/// `None` when it cannot be told; callers treat that as mains power, which is
/// what a desktop without a battery is.
#[cfg(target_os = "macos")]
pub fn on_battery() -> Option<bool> {
    use std::ffi::{c_char, c_void};

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPSCopyPowerSourcesInfo() -> *const c_void;
        fn IOPSGetProvidingPowerSourceType(snapshot: *const c_void) -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringGetCString(s: *const c_void, buf: *mut c_char, len: isize, enc: u32) -> bool;
        fn CFRelease(object: *const c_void);
    }
    const UTF8: u32 = 0x0800_0100;

    // SAFETY: the snapshot is owned here and released once; the source type
    // belongs to it and is read before the release.
    unsafe {
        let snapshot = IOPSCopyPowerSourcesInfo();
        if snapshot.is_null() {
            return None;
        }
        let kind = IOPSGetProvidingPowerSourceType(snapshot);
        let mut buf = [0 as c_char; 32];
        let read = !kind.is_null() && CFStringGetCString(kind, buf.as_mut_ptr(), 32, UTF8);
        CFRelease(snapshot);
        if !read {
            return None;
        }
        // kIOPSBatteryPowerValue; the others are "AC Power" and "UPS Power".
        Some(std::ffi::CStr::from_ptr(buf.as_ptr()).to_bytes() == b"Battery Power")
    }
}

/// Windows says whether the AC line is connected; 255 is unknown.
#[cfg(windows)]
pub fn on_battery() -> Option<bool> {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    // SAFETY: a plain out-parameter struct, zeroed, filled by the call.
    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    // SAFETY: writes only the struct it is given.
    if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
        return None;
    }
    match status.ACLineStatus {
        0 => Some(true),
        1 => Some(false),
        _ => None,
    }
}

/// Linux lists its supplies in sysfs: on battery when there is a mains supply
/// and none of them is online.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn on_battery() -> Option<bool> {
    let supplies = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let mains: Vec<bool> = supplies
        .flatten()
        .filter(|s| {
            std::fs::read_to_string(s.path().join("type")).is_ok_and(|t| t.trim() == "Mains")
        })
        .map(|s| std::fs::read_to_string(s.path().join("online")).is_ok_and(|o| o.trim() == "1"))
        .collect();
    (!mains.is_empty()).then(|| !mains.contains(&true))
}

/// A friendlier name for the backend than its `Debug` spelling.
fn backend_label(backend: wgpu::Backend) -> &'static str {
    match backend {
        wgpu::Backend::Dx12 => "DirectX 12",
        wgpu::Backend::Metal => "Metal",
        wgpu::Backend::Vulkan => "Vulkan",
        wgpu::Backend::Gl => "OpenGL",
        wgpu::Backend::BrowserWebGpu => "WebGPU",
        wgpu::Backend::Noop => "none",
    }
}

fn kind_label(device_type: wgpu::DeviceType) -> &'static str {
    match device_type {
        wgpu::DeviceType::DiscreteGpu => "discrete",
        wgpu::DeviceType::IntegratedGpu => "integrated",
        // A virtual GPU (VM passthrough) has some acceleration but rarely a full
        // discrete profile — treat it as the middle tier.
        wgpu::DeviceType::VirtualGpu => "integrated",
        wgpu::DeviceType::Cpu => "cpu",
        wgpu::DeviceType::Other => "other",
    }
}

/// Dedicated GPU memory in MB, or `None` when it can't be read. The memory half
/// of [`probe`] alone — no GPU adapter is created, so it is cheap enough to ask
/// every time a batch starts.
pub fn vram_mb() -> Option<u32> {
    probe_memory().0
}

/// Probes the real preferred adapter — **without** forcing the software fallback,
/// so "no discrete GPU" is reported as such rather than masked by a CPU adapter.
pub fn probe() -> HardwareReport {
    let (gpu_name, gpu_kind, backend) = {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        })) {
            Ok(adapter) => {
                let info = adapter.get_info();
                (
                    info.name,
                    kind_label(info.device_type).to_string(),
                    backend_label(info.backend).to_string(),
                )
            }
            Err(_) => (
                "No GPU detected".to_string(),
                "cpu".to_string(),
                "none".to_string(),
            ),
        }
    };

    let threads = cpu_threads();
    let (vram_mb, ram_mb) = probe_memory();
    let unified = cfg!(all(target_os = "macos", target_arch = "aarch64")) && backend == "Metal";
    let (tier, base_advice) = recommend(&gpu_kind, threads, unified);
    // Fold the measured VRAM into the advice, since it's the number that decides
    // which vision models fit.
    let advice = match vram_mb {
        Some(mb) if mb >= 512 => format!("{base_advice} {} of GPU memory available.", human_gb(mb)),
        _ => base_advice,
    };
    HardwareReport {
        gpu_name,
        gpu_kind,
        backend,
        cpu_threads: threads,
        vram_mb,
        ram_mb,
        tier,
        advice,
    }
}

/// Free space, in bytes, on the drive that holds `path` — or on the nearest
/// ancestor that exists, since an export folder or a model directory may not be
/// created yet. `None` when it cannot be read; callers then carry on as they
/// always did rather than refuse on a guess.
///
/// The space *this user* may write, not the raw free count: on Windows that is
/// what a quota leaves them, on Unix what the root reserve does not.
pub fn free_space(path: &std::path::Path) -> Option<u64> {
    let existing = path.ancestors().find(|p| p.exists())?;
    free_space_at(existing)
}

#[cfg(windows)]
fn free_space_at(path: &std::path::Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available: u64 = 0;
    // SAFETY: `wide` is NUL-terminated and outlives the call; the two totals
    // this does not need may be null per the API contract.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}

#[cfg(unix)]
fn free_space_at(path: &std::path::Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;

    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `statvfs` only writes the struct it is given, and a zeroed
    // `statvfs` is a valid value for it to overwrite.
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: as above; `c_path` is NUL-terminated and outlives the call.
    let ok = unsafe { libc::statvfs(c_path.as_ptr(), &mut stats) } == 0;
    #[allow(clippy::unnecessary_cast)] // the field widths differ by platform
    ok.then(|| stats.f_bavail as u64 * stats.f_frsize as u64)
}

#[cfg(not(any(windows, unix)))]
fn free_space_at(_path: &std::path::Path) -> Option<u64> {
    None
}

/// `Some((wanted, free))` when the drive holding `dir` has less free than a job
/// of `needed` bytes should start with; `None` when it fits, or when free space
/// cannot be read (a job is never refused on a guess).
///
/// `wanted` is `needed` plus a margin: a disk filled to the last byte breaks the
/// catalogue's own writes too, and sizes known in advance are estimates.
pub fn shortfall(dir: &std::path::Path, needed: u64) -> Option<(u64, u64)> {
    let wanted = needed + needed / 10 + 256 * 1024 * 1024;
    let free = free_space(dir)?;
    (free < wanted).then_some((wanted, free))
}

/// Refuses, before anything is written, a job that the drive holding `dir`
/// cannot take — rather than letting it fail partway through, which for a
/// multi-gigabyte download means at 90%.
pub fn ensure_space(dir: &std::path::Path, needed: u64, what: &str) -> anyhow::Result<()> {
    if let Some((wanted, free)) = shortfall(dir, needed) {
        anyhow::bail!(
            "Not enough free space for {what}: it needs about {}, and the drive holding {} has {} free.",
            human_bytes(wanted),
            dir.display(),
            human_bytes(free)
        );
    }
    Ok(())
}

/// "1.4 GB" / "350 MB" for a byte count.
pub fn human_bytes(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{} MB", mb.ceil() as u64)
    }
}

/// A short "12 GB" / "1.5 GB" label for a megabyte count.
fn human_gb(mb: u32) -> String {
    let gb = mb as f32 / 1024.0;
    if gb >= 10.0 {
        format!("{} GB", gb.round() as u32)
    } else {
        format!("{gb:.1} GB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_space_reads_the_drive_and_walks_up_to_a_folder_that_exists() {
        let dir = std::env::temp_dir();
        let free = free_space(&dir).expect("the temp drive has a readable free count");
        assert!(free > 0);
        // A folder not created yet is measured on the drive that will hold it.
        let later = dir.join("sourcelight-not-yet").join("exports");
        assert!(free_space(&later).is_some());
    }

    #[test]
    fn a_job_the_drive_cannot_take_is_refused_with_both_sizes() {
        let dir = std::env::temp_dir();
        assert!(shortfall(&dir, 0).is_none() || free_space(&dir).unwrap() < 256 * 1024 * 1024);
        let err = ensure_space(&dir, u64::MAX / 4, "the test model")
            .unwrap_err()
            .to_string();
        assert!(
            err.starts_with("Not enough free space for the test model"),
            "{err}"
        );
        assert!(err.contains(" free."), "{err}");
    }

    /// Exercises the Network.framework calls themselves: a wrong signature or
    /// a leaked handler shows up here as a crash or a hang, not as a reading.
    /// The power source is read — a Mac always has one, battery or mains —
    /// and a desktop, like the Mac mini the checks run on, is on mains.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_power_source_is_read() {
        let reading = on_battery();
        assert!(reading.is_some());
        eprintln!("on battery: {reading:?}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_mac_metered_check_answers_promptly() {
        let started = std::time::Instant::now();
        let _ = metered();
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
    }

    /// A background pass's loop gives way to the window while it runs, and the
    /// pooled thread it ran on is handed back as it was.
    #[cfg(windows)]
    #[test]
    fn a_background_pass_lowers_its_thread_and_restores_it() {
        use windows::Win32::System::Threading::{
            GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_LOWEST,
        };
        // SAFETY: the pseudo-handle of the calling thread, always valid.
        let priority = || unsafe { GetThreadPriority(GetCurrentThread()) };
        let before = priority();
        {
            let _below = BackgroundPriority::begin();
            assert_eq!(priority(), THREAD_PRIORITY_LOWEST.0);
        }
        assert_eq!(priority(), before);
        // A thread that is background for life stays lowered.
        let lowered = std::thread::spawn(move || {
            lower_this_thread();
            priority()
        })
        .join()
        .unwrap();
        assert_eq!(lowered, THREAD_PRIORITY_LOWEST.0);
    }

    #[test]
    fn byte_counts_read_as_megabytes_or_gigabytes() {
        assert_eq!(human_bytes(350 * 1_048_576), "350 MB");
        assert_eq!(human_bytes(1_500 * 1_048_576), "1.5 GB");
    }

    #[test]
    fn recommend_maps_each_gpu_class_to_a_sensible_tier() {
        assert_eq!(recommend("discrete", 24, false).0, ModelTier::Full);
        assert_eq!(recommend("integrated", 8, false).0, ModelTier::Compact);
        assert_eq!(recommend("cpu", 16, false).0, ModelTier::Cpu);
        assert_eq!(recommend("other", 4, false).0, ModelTier::Cpu);
    }

    #[test]
    fn apple_silicon_is_graded_by_its_memory_not_as_small_integrated_graphics() {
        let (tier, advice) = recommend("integrated", 10, true);
        assert_eq!(tier, ModelTier::Full);
        assert!(advice.contains("Apple silicon"), "{advice}");
        assert!(!advice.contains("compact"), "{advice}");
        // Unified memory only lifts an integrated GPU; no GPU is still no GPU.
        assert_eq!(recommend("cpu", 10, true).0, ModelTier::Cpu);
    }

    #[test]
    fn a_weak_cpu_only_box_is_warned_more_firmly() {
        let strong = recommend("cpu", 16, false).1;
        let weak = recommend("cpu", 2, false).1;
        assert!(weak.contains("slow"));
        assert_ne!(strong, weak);
    }

    #[test]
    fn the_probe_always_returns_something_usable() {
        // On CI's software adapter this still yields a report rather than
        // panicking, which is the contract the UI relies on.
        let r = probe();
        assert!(!r.gpu_kind.is_empty());
        assert!(!r.advice.is_empty());
        assert!(r.cpu_threads >= 1);
    }

    /// The GPU given to DirectML is a real hardware adapter, and the one DXGI
    /// ranks first for performance: on a laptop with two, the discrete one.
    #[cfg(windows)]
    #[test]
    fn ai_models_get_the_high_performance_gpu() {
        use windows::Win32::Graphics::Dxgi::{
            CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory6, DXGI_ADAPTER_FLAG_SOFTWARE,
            DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
        };
        let (index, name) = ai_gpu();
        eprintln!("DirectML adapter {index}: {name}");
        // SAFETY: plain DXGI queries on interfaces this test owns.
        let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory6>() }) else {
            return;
        };
        // SAFETY: as above.
        let Ok(chosen) = (unsafe { factory.EnumAdapters1(*index) }) else {
            assert_eq!(*index, 0, "an index DXGI does not have");
            return;
        };
        // SAFETY: as above.
        let chosen = unsafe { chosen.GetDesc1() }.unwrap();
        if chosen.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            return; // No hardware GPU on this machine: the old behaviour.
        }
        // SAFETY: as above.
        let first: IDXGIAdapter1 =
            unsafe { factory.EnumAdapterByGpuPreference(0, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE) }
                .unwrap();
        // SAFETY: as above.
        let first = unsafe { first.GetDesc1() }.unwrap();
        if first.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0 {
            assert_eq!(
                chosen.AdapterLuid, first.AdapterLuid,
                "not the high-performance GPU"
            );
        }
        assert!(!name.is_empty());
    }
}
