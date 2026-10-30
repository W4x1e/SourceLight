//! The vision model's server: llama.cpp's `llama-server`, run by the app.
//!
//! `tagger::llama` talks to it over HTTP on a loopback port of its own;
//! `llama_pack` downloads it and `vlm` the model it runs. This starts and
//! stops it, and decides how: which device, how many photos at once, what an
//! image may cost. The decisions and the argument list are pure and tested;
//! the spawn is the thin, runtime-verified part. `commands/ai.rs` owns the lifecycle:
//! started when a pass begins, stopped two minutes after the last one ends,
//! and on quit.
//!
//! It replaced a managed Ollama: Ollama's API was the slow part of
//! every photo, and the Mac already ran the model this way. See
//! `docs/AI-MODEL-SPIKES.md`.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Starts a server as every one is started: output to `output` (or nowhere),
/// no console window, and on Windows unable to outlive the app
/// ([`die_with_app`]).
fn start(mut cmd: Command, output: Option<std::fs::File>) -> std::io::Result<Child> {
    cmd.stdin(Stdio::null());
    match output {
        Some(file) => {
            cmd.stdout(Stdio::from(file.try_clone()?))
                .stderr(Stdio::from(file));
        }
        None => {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
    // Don't flash a console window when spawning the CLI on Windows.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let child = cmd.spawn()?;
    die_with_app(&child);
    Ok(child)
}

/// The port the app's `llama-server` listens on when no free one can be
/// found; normally each run chooses its own (see [`link`]).
pub const LLAMA_SERVER_PORT: u16 = 41435;

/// This run's link to the server it starts. The app used to trust
/// whatever answered on a fixed port: a program that took the port first,
/// or a server orphaned by a crash, was sent every photo and its answers
/// stored as captions. Now each run chooses a free port, and gives its
/// server a key and a name through the server's environment, which other
/// users cannot read, unlike its command line. Every request carries the
/// key, and before the first photo the server must answer as that name.
pub struct Link {
    pub port: u16,
    /// What every request but `/health` must carry (`LLAMA_API_KEY`).
    pub key: String,
    /// The model name the server is given (`LLAMA_ARG_ALIAS`), which it
    /// reports back from `/v1/models`, behind the key.
    pub name: String,
}

impl Link {
    /// A URL on the server.
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// Whether `/v1/models`' answer is the server this run started.
    pub fn answers_as_ours(&self, models: &str) -> bool {
        models.contains(&self.name)
    }
}

/// The run's [`Link`], made the first time it is asked for.
pub fn link() -> &'static Link {
    static LINK: std::sync::OnceLock<Link> = std::sync::OnceLock::new();
    LINK.get_or_init(|| Link {
        port: free_port().unwrap_or(LLAMA_SERVER_PORT),
        key: secret(),
        name: format!("vision-{}", &secret()[..16]),
    })
}

/// A loopback port nothing is listening on right now.
fn free_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?;
    listener.local_addr().ok().map(|a| a.port())
}

/// 128 bits from the OS random source, as hex.
fn secret() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the OS random source");
    crate::hashing::hex(&bytes)
}

/// An HTTP client for the server, never through a proxy. Loopback
/// requests honoured the system's and `HTTP_PROXY`'s proxies, which could
/// not reach the server (descriptions "took too long to load") or, worse,
/// logged every photo passing through a local one.
pub fn loopback_client(timeout: std::time::Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(timeout)
        .build()
        .unwrap_or_default()
}

fn server_exe() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

/// The installed `llama-server` under `runtime_dir`: at its root, where the
/// Windows archives put it, or one folder down, where the macOS one does
/// (`llama-b11234/`). `None` means the runtime is not installed.
pub fn llama_server_binary(runtime_dir: &Path) -> Option<PathBuf> {
    let at_root = runtime_dir.join(server_exe());
    if at_root.is_file() {
        return Some(at_root);
    }
    std::fs::read_dir(runtime_dir)
        .ok()?
        .flatten()
        .map(|e| e.path().join(server_exe()))
        .find(|p| p.is_file())
}

/// A device llama-server can run a model on, as `--list-devices` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// What `--device` takes: `Vulkan0`, `CUDA0`.
    pub id: String,
    /// The GPU's own name: `NVIDIA GeForce RTX 5070 Ti`.
    pub name: String,
}

/// The devices in `llama-server --list-devices` output, which lists one per
/// line as `  Vulkan0: NVIDIA GeForce RTX 5070 Ti (15995 MiB, 15227 MiB free)`.
pub fn parse_devices(listing: &str) -> Vec<Device> {
    listing
        .lines()
        .filter_map(|line| {
            let (id, rest) = line.trim().split_once(": ")?;
            if id.contains(' ') || !id.ends_with(|c: char| c.is_ascii_digit()) {
                return None;
            }
            let name = rest.rsplit_once(" (").map_or(rest, |(name, _)| name).trim();
            Some(Device {
                id: id.to_string(),
                name: name.to_string(),
            })
        })
        .collect()
}

/// How long the device probe may take before it is given up on.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// The devices `binary` can see. Takes a moment (it starts the backends), so
/// the caller asks once per server start, off the async reactor.
///
/// In the app's job, like the servers, and given [`PROBE_TIMEOUT`]: a
/// GPU driver that hung starting its backend held Describe on "Loading
/// model…" for ever, and outlived the app. Nothing found means letting
/// llama.cpp choose.
pub fn list_devices(binary: &Path) -> Vec<Device> {
    use std::io::Read;
    let mut cmd = Command::new(binary);
    cmd.arg("--list-devices")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(dir) = binary.parent().filter(|d| !d.as_os_str().is_empty()) {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let Ok(mut child) = cmd.spawn() else {
        return Vec::new();
    };
    die_with_app(&child);
    let deadline = std::time::Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            _ => {
                eprintln!("sidecar: the device probe did not finish; llama.cpp will choose");
                let _ = child.kill();
                let _ = child.wait();
                return Vec::new();
            }
        }
    }
    let mut listing = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut listing);
    }
    parse_devices(&listing)
}

/// Letters and digits only, lower-case, without trademark marks: DXGI and
/// Vulkan spell the same GPU's name alike, give or take those.
fn gpu_key(name: &str) -> String {
    name.to_lowercase()
        .replace("(tm)", "")
        .replace('™', "")
        .replace("(r)", "")
        .replace('®', "")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

/// What to pass as `--device`, from the devices llama-server sees, the name
/// of the GPU the app's other AI models run on (`hardware::ai_gpu`),
/// and whether this machine has a discrete GPU at all.
///
/// - **No discrete GPU: the CPU** (`none`). On the 2-CU Radeon that was measured,
///   the CPU was faster than the integrated GPU (tags 4.7 s against
///   10.7 s), and Ollama, which this replaced, used the CPU on such machines
///   too. Stronger laptop iGPUs (Radeon 780M, Intel Arc) are unmeasured.
/// - **Otherwise that GPU**, matched by name. Never the device
///   reporting the most memory: an integrated GPU reports the shared system
///   memory, more than a 16 GB card's.
/// - **No match:** `None`, leaving the choice to llama.cpp, which took the
///   discrete GPU in every run measured.
pub fn choose_device(devices: &[Device], preferred: &str, discrete: bool) -> Option<String> {
    if !discrete {
        return Some("none".into());
    }
    let wanted = gpu_key(preferred);
    if wanted.is_empty() {
        return None;
    }
    devices
        .iter()
        .find(|d| gpu_key(&d.name) == wanted)
        .map(|d| d.id.clone())
}

/// Photos in flight at once on the app's `llama-server`.
///
/// - **On a Mac, two:** measured on a 16 GB M4 with Qwen3-VL 4B, 2.5 s a
///   photo for tags against 2.9 s one at a time, because writing the reply
///   is bound by memory bandwidth and a second reply rides along. Four was
///   slower than two there (4.3 s against 3.9 s for tags and a description).
/// - **Elsewhere, up to four:** on an RTX 5070 Ti, tags went from 0.30 s a
///   photo at two to 0.23 s at four; eight gained little more, because the
///   server reads images one at a time.
///
/// `parallel` is the batch size already resolved from the GPU's memory.
pub fn llama_server_slots(parallel: usize) -> usize {
    parallel.clamp(1, if cfg!(target_os = "macos") { 2 } else { 4 })
}

/// Context per slot: the instructions (~460 tokens), a full-size image
/// (~1,080) and the longest reply (512) with room to spare.
const CONTEXT_PER_SLOT: usize = 4096;

/// The arguments the app starts `llama-server` with:
///
/// - `--image-min-tokens 64`, so an image costs what its size says: the
///   640 px tag image ~260 tokens. Qwen-VL's own default is 1,024 (what
///   bounding boxes want), which made a tag cost as much as a description.
/// - `slots` requests at once, each with its own context.
/// - an 8-bit context cache and flash attention, which keep each slot small.
/// - `--cache-ram 0`: no prompt cache in *system* memory. Tagging never sends
///   the same photo twice, so it never hits, and on a 16 GB Mac it filled to
///   6 GB and pushed the system into swap. Each slot still reuses the
///   instructions it read for the previous photo.
/// - the fixed ChatML template the Qwen-VL models use.
/// - `device`, when there is one to name (see [`choose_device`]).
pub fn llama_server_args(
    model: &Path,
    projector: &Path,
    slots: usize,
    device: Option<&str>,
) -> Vec<String> {
    let slots = slots.max(1);
    let mut args: Vec<String> = [
        "--model",
        &model.display().to_string(),
        "--mmproj",
        &projector.display().to_string(),
        "--host",
        "127.0.0.1",
        "--port",
        &link().port.to_string(),
        "--no-webui",
        // Its slot monitor is nobody's business.
        "--no-slots",
        "--offline",
        "--ctx-size",
        &(CONTEXT_PER_SLOT * slots).to_string(),
        "--parallel",
        &slots.to_string(),
        "--no-jinja",
        "--chat-template",
        "chatml",
        "--cache-type-k",
        "q8_0",
        "--cache-type-v",
        "q8_0",
        "--flash-attn",
        "on",
        "--cache-ram",
        "0",
        "--image-min-tokens",
        "64",
        "--image-max-tokens",
        "1280",
    ]
    .iter()
    .map(|a| a.to_string())
    .collect();
    if let Some(device) = device {
        args.extend(["--device".to_string(), device.to_string()]);
    }
    args
}

/// Starts `llama-server` with `args`, from its own folder so its backends are
/// found beside it. The child is returned for the caller to track and stop.
/// Its output goes to `output` (`session_log::server_log`): which device it
/// chose and how much memory the model took are only ever said there.
pub fn spawn_llama_server(
    binary: &Path,
    args: &[String],
    output: Option<std::fs::File>,
) -> std::io::Result<Child> {
    let mut cmd = Command::new(binary);
    cmd.args(args);
    // Through the environment, not the arguments, which any user can read
    // (see [`Link`]).
    let link = link();
    cmd.env("LLAMA_API_KEY", &link.key)
        .env("LLAMA_ARG_ALIAS", &link.name);
    if let Some(dir) = binary.parent().filter(|d| !d.as_os_str().is_empty()) {
        cmd.current_dir(dir);
    }
    start(cmd, output)
}

/// How long the server gets to shut down cleanly before it is killed.
pub const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// Stops a server we started: asked first (SIGTERM), killed only if it has
/// not gone within `grace`. Windows has no SIGTERM to send, so there it is
/// killed outright; the job it was started in ([`die_with_app`]) is the net
/// for anything it leaves.
pub fn stop(child: &mut Child, grace: std::time::Duration) {
    #[cfg(unix)]
    {
        if let Ok(pid) = i32::try_from(child.id()) {
            // SAFETY: signals only the process we spawned and still hold,
            // so the pid has not been reaped and reused.
            unsafe { libc::kill(pid, libc::SIGTERM) };
            let deadline = std::time::Instant::now() + grace;
            while std::time::Instant::now() < deadline {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = grace;
    let _ = child.kill();
    let _ = child.wait();
}

/// Puts a server in the app's job, so that on Windows nothing the app starts
/// can outlive it.
///
/// An app that crashes, or is ended from Task Manager, stops nothing: the
/// server it started would keep the vision model (gigabytes of VRAM) loaded
/// until the next reboot. With the Ollama runtime this replaced, a runner
/// left behind held qwen3-vl:2b's ~3 GB for as long as anyone watched. So
/// every server goes into one job object that ends all its processes when
/// its last handle closes. Only the app holds that handle, and never closes
/// it: Windows does, when the app's process ends — quit, crash or Task
/// Manager. What a process in the job starts is in the job too.
///
/// A server that cannot be assigned still runs, as it did before there was a
/// job; the app only loses the net under it.
#[cfg(windows)]
fn die_with_app(child: &Child) {
    if let Some(job) = app_job() {
        if let Err(e) = job.assign(child) {
            eprintln!(
                "sidecar: process {} will not end with the app: {e}",
                child.id()
            );
        }
    }
}

#[cfg(not(windows))]
fn die_with_app(_: &Child) {}

/// Notes the server's process id in `pid_file`, so the next run can stop it
/// if this one crashes first ([`stop_orphan`]). Unix only: on Windows the job
/// already ends it with the app.
pub fn note_server(pid_file: &Path, child: &Child) {
    if cfg!(unix) {
        let _ = std::fs::write(pid_file, child.id().to_string());
    }
}

/// Stops a server an earlier run left behind, holding gigabytes of
/// memory: on macOS, a crash or force-quit within two minutes of a Describe
/// pass left it running for good. Only a process that is still a
/// `llama-server` is stopped, so a reused process id is left alone.
pub fn stop_orphan(pid_file: &Path) {
    let Some(pid) = std::fs::read_to_string(pid_file)
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
    else {
        return;
    };
    let _ = std::fs::remove_file(pid_file);
    #[cfg(unix)]
    {
        let name = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "comm="])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        if name.trim_end().ends_with("llama-server") {
            eprintln!("sidecar: stopping a llama-server an earlier run left behind ({pid})");
            // SAFETY: signals a process just confirmed to be a llama-server.
            unsafe { libc::kill(pid, libc::SIGTERM) };
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// The job every server is put in, made with the first. Held in a static, so
/// its handle stays open until the process ends.
#[cfg(windows)]
fn app_job() -> Option<&'static job::Job> {
    static JOB: std::sync::OnceLock<Option<job::Job>> = std::sync::OnceLock::new();
    JOB.get_or_init(|| {
        job::Job::new()
            .inspect_err(|e| eprintln!("sidecar: no job object to end servers with the app: {e}"))
            .ok()
    })
    .as_ref()
}

/// A Windows job object that ends every process in it when its last handle
/// closes.
#[cfg(windows)]
mod job {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    use windows::core::{Owned, Result};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_BASIC_LIMIT_INFORMATION,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub struct Job(Owned<HANDLE>);

    // SAFETY: a job handle is a kernel handle, good on any thread, and the
    // calls made with it are safe to make from several at once.
    unsafe impl Send for Job {}
    // SAFETY: as above.
    unsafe impl Sync for Job {}

    impl Job {
        pub fn new() -> Result<Job> {
            let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
                BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
                    LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    ..Default::default()
                },
                ..Default::default()
            };
            // SAFETY: the limits are the struct, and the size, that this
            // information class takes. No security attributes means a handle
            // that cannot be inherited, which matters: a server holding one
            // would keep the job open, and itself alive, after the app is gone.
            unsafe {
                let job = Owned::new(CreateJobObjectW(None, None)?);
                SetInformationJobObject(
                    *job,
                    JobObjectExtendedLimitInformation,
                    std::ptr::from_ref(&limits).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )?;
                Ok(Job(job))
            }
        }

        pub fn assign(&self, child: &Child) -> Result<()> {
            // SAFETY: the process handle belongs to a child still held, so it
            // is open.
            unsafe { AssignProcessToJobObject(*self.0, HANDLE(child.as_raw_handle())) }
        }

        /// The ids of the processes in the job, for tests: up to 16, which is
        /// plenty for them.
        #[cfg(test)]
        pub fn processes(&self) -> Vec<u32> {
            use windows::Win32::System::JobObjects::{
                JobObjectBasicProcessIdList, QueryInformationJobObject,
            };
            // JOBOBJECT_BASIC_PROCESS_ID_LIST, with room for more than one id.
            #[repr(C)]
            struct List {
                _assigned: u32,
                listed: u32,
                ids: [usize; 16],
            }
            let mut list = List {
                _assigned: 0,
                listed: 0,
                ids: [0; 16],
            };
            // SAFETY: `List` has the layout this information class writes,
            // and the size passed is its own.
            unsafe {
                QueryInformationJobObject(
                    Some(*self.0),
                    JobObjectBasicProcessIdList,
                    std::ptr::from_mut(&mut list).cast(),
                    std::mem::size_of::<List>() as u32,
                    None,
                )
            }
            .expect("listing the job's processes");
            list.ids
                .iter()
                .take(list.listed as usize)
                .map(|&id| id as u32)
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_is_found_at_the_root_or_one_folder_down() {
        let root = crate::test_support::Scratch::new("sidecar-server-binary");
        assert!(llama_server_binary(&root).is_none());
        // The macOS archive's layout: a folder named for the build.
        std::fs::create_dir_all(root.join("llama-b11234")).unwrap();
        std::fs::write(root.join("llama-b11234").join(server_exe()), b"x").unwrap();
        assert_eq!(
            llama_server_binary(&root),
            Some(root.join("llama-b11234").join(server_exe()))
        );
        // The Windows archives' layout, which wins when both are there.
        std::fs::write(root.join(server_exe()), b"x").unwrap();
        assert_eq!(llama_server_binary(&root), Some(root.join(server_exe())));
    }

    /// What b11234 lists on a PC with a discrete and an integrated GPU.
    const LISTING: &str = "Available devices:
  Vulkan0: NVIDIA GeForce RTX 5070 Ti (15995 MiB, 15227 MiB free)
  Vulkan1: AMD Radeon(TM) Graphics (16442 MiB, 15620 MiB free)
";

    #[test]
    fn devices_are_read_from_the_listing() {
        assert_eq!(
            parse_devices(LISTING),
            [
                Device {
                    id: "Vulkan0".into(),
                    name: "NVIDIA GeForce RTX 5070 Ti".into()
                },
                Device {
                    id: "Vulkan1".into(),
                    name: "AMD Radeon(TM) Graphics".into()
                },
            ]
        );
        assert!(parse_devices("Available devices:\n  (none)\n").is_empty());
        assert!(parse_devices("load_backend: loaded Vulkan backend from x.dll").is_empty());
    }

    #[test]
    fn the_device_is_the_gpu_the_other_models_use_or_the_cpu() {
        let devices = parse_devices(LISTING);
        // The adapter the other models use, by name, not the one reporting most memory.
        assert_eq!(
            choose_device(&devices, "NVIDIA GeForce RTX 5070 Ti", true).as_deref(),
            Some("Vulkan0")
        );
        assert_eq!(
            choose_device(&devices, "AMD Radeon™ Graphics", true).as_deref(),
            Some("Vulkan1")
        );
        // Integrated graphics only: the CPU, which beat it.
        assert_eq!(
            choose_device(&devices, "AMD Radeon(TM) Graphics", false).as_deref(),
            Some("none")
        );
        // Nothing to match: llama.cpp chooses.
        assert_eq!(choose_device(&devices, "", true), None);
        assert_eq!(choose_device(&[], "NVIDIA GeForce RTX 5070 Ti", true), None);
    }

    #[test]
    fn llama_server_runs_small_images_small_on_the_chosen_device() {
        let value = |args: &[String], flag: &str| {
            let i = args.iter().position(|a| a == flag)?;
            Some(args[i + 1].clone())
        };
        let args = llama_server_args(Path::new("/m/model"), Path::new("/m/proj"), 2, None);
        assert_eq!(value(&args, "--image-min-tokens").as_deref(), Some("64"));
        assert_eq!(value(&args, "--parallel").as_deref(), Some("2"));
        assert_eq!(
            value(&args, "--ctx-size"),
            Some((CONTEXT_PER_SLOT * 2).to_string())
        );
        assert_eq!(value(&args, "--cache-ram").as_deref(), Some("0"));
        assert_eq!(value(&args, "--port"), Some(link().port.to_string()));
        assert!(args.iter().any(|a| a == "--no-slots"));
        // The key and name go through the environment only.
        assert!(!args
            .iter()
            .any(|a| a.contains(&link().key) || a.contains(&link().name)));
        assert_eq!(value(&args, "--host").as_deref(), Some("127.0.0.1"));
        assert_eq!(value(&args, "--mmproj").as_deref(), Some("/m/proj"));
        assert_eq!(value(&args, "--device"), None);
        let args = llama_server_args(Path::new("/m"), Path::new("/p"), 1, Some("Vulkan0"));
        assert_eq!(value(&args, "--device").as_deref(), Some("Vulkan0"));
    }

    /// Each run's key and name are its own, and only an answer naming the
    /// server's name counts as ours.
    #[test]
    fn only_the_server_this_run_started_is_trusted() {
        let link = link();
        assert_eq!(link.key.len(), 32);
        assert_ne!(link.key, secret());
        assert!(link.answers_as_ours(&format!(r#"{{"data":[{{"id":"{}"}}]}}"#, link.name)));
        assert!(!link.answers_as_ours(r#"{"data":[{"id":"qwen3-vl"}]}"#));
        assert!(link.url("/health").starts_with("http://127.0.0.1:"));
    }

    #[test]
    fn slots_are_two_on_a_mac_and_up_to_four_elsewhere() {
        assert_eq!(llama_server_slots(0), 1);
        let most = if cfg!(target_os = "macos") { 2 } else { 4 };
        assert_eq!(llama_server_slots(8), most);
    }

    #[cfg(unix)]
    #[test]
    fn a_server_is_asked_to_stop_before_it_is_killed() {
        use std::time::{Duration, Instant};
        // Exits on SIGTERM, as llama-server does: gone well inside the grace.
        let mut polite = Command::new("sleep").arg("30").spawn().unwrap();
        let started = Instant::now();
        stop(&mut polite, Duration::from_secs(5));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(polite.try_wait().unwrap().is_some());

        // Ignores it: killed once the grace runs out, never left running.
        let mut stubborn = Command::new("sh")
            .args(["-c", "trap '' TERM; sleep 30"])
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(200)); // let the trap install
        let started = Instant::now();
        stop(&mut stubborn, Duration::from_millis(300));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(stubborn.try_wait().unwrap().is_some());
    }

    /// What the job is for, with `cmd` running `ping` in place of a server
    /// and whatever it runs. Killing the parent, all Windows can do, leaves
    /// the child going; closing the job, as Windows closes the app's when the
    /// app's process ends, takes it down.
    #[cfg(windows)]
    #[test]
    fn a_runner_outlives_its_stopped_server_but_not_the_job() {
        use std::io::Read;
        use std::os::windows::process::CommandExt;
        use windows::core::{Owned, PWSTR};
        use windows::Win32::Foundation::WAIT_OBJECT_0;
        use windows::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        };

        let job = job::Job::new().unwrap();
        // `set /p` waits for a line, so `ping` starts only once the server is
        // in the job.
        let mut server = Command::new("cmd")
            .raw_arg("/c set /p go= & ping -n 60 127.0.0.1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .unwrap();
        job.assign(&server).unwrap();
        drop(server.stdin.take());
        // Its first words mean `ping` is up. Killed while it was still
        // starting, the parent took it down in four runs of six. Held open to
        // the end, so `ping` never writes to a closed pipe.
        let mut output = server.stdout.take().unwrap();
        assert!(output.read(&mut [0; 64]).unwrap() > 0, "ping never started");

        // Found by name: Windows can put a console host in the job as well.
        let runner = job
            .processes()
            .into_iter()
            .find_map(|id| {
                // SAFETY: opens a handle of our own, closed by `Owned`, and
                // reads its name into a buffer whose length goes with it.
                unsafe {
                    let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
                    let process = Owned::new(OpenProcess(access, false, id).ok()?);
                    let mut path = [0u16; 512];
                    let mut len = path.len() as u32;
                    let name = PWSTR(path.as_mut_ptr());
                    QueryFullProcessImageNameW(*process, PROCESS_NAME_WIN32, name, &mut len)
                        .ok()?;
                    String::from_utf16_lossy(&path[..len as usize])
                        .to_lowercase()
                        .ends_with("\\ping.exe")
                        .then_some(process)
                }
            })
            .expect("ping is in the job");
        // SAFETY: waits on a handle we hold.
        let gone_within = |ms| unsafe { WaitForSingleObject(*runner, ms) } == WAIT_OBJECT_0;

        stop(&mut server, STOP_GRACE);
        assert!(!gone_within(500), "the runner went with its server");
        drop(job);
        assert!(gone_within(5_000), "the runner outlived the job");
    }

    /// Every server starts in the job the app holds until it exits.
    #[cfg(windows)]
    #[test]
    fn a_server_starts_in_the_apps_job() {
        let args = ["-n", "30", "127.0.0.1"].map(String::from);
        let mut server = spawn_llama_server(Path::new("ping"), &args, None).unwrap();
        let in_job = app_job().unwrap().processes().contains(&server.id());
        stop(&mut server, STOP_GRACE);
        assert!(in_job);
    }
}
