//! AI setup: settings, model downloads and the vision model's server.

use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State};

use super::common::{ai_models_dir, bundled_model_sources, count, err};
#[cfg(feature = "ai")]
use crate::develop;
use crate::models::ScanProgress;
use crate::sync::lock_recover;
use crate::{
    ai_models, ai_setup, db, errors, gpu_gate, hardware, llama_pack, search, session_log, sidecar,
    tagger, vlm, AppState,
};

/// Reads provider settings from the DB and the key from the OS credential
/// store. The key is fetched here and nowhere else — it never touches SQLite.
pub(crate) fn ai_config(state: &AppState) -> Result<tagger::ProviderConfig, String> {
    let conn = state.conn()?;
    // Unset means "never set up", which is `None` — not the vision model.
    // Defaulting to it regardless meant a fresh install's very first status
    // line was an error about a choice the user had not made. `ensure_ai_ready`
    // picks and stores a real provider the first time an AI feature is used.
    let provider = tagger::Provider::parse(
        &db::get_setting(&conn, "ai_provider")
            .map_err(err)?
            .unwrap_or_default(),
    );
    // A stored vision model that is no longer in the catalogue — one removed
    // for its licence, or typed by hand — reads as unset, so the setup
    // ladder replaces it with this machine's recommendation instead of
    // fetching it. An Ollama tag stored by an older version reads as unset
    // the same way. Only the vision provider names a model; the built-in one ignores it.
    let model = db::get_setting(&conn, "ai_model")
        .map_err(err)?
        .unwrap_or_default();
    let model = if provider == tagger::Provider::Vision && !vlm::is_listed(model.trim()) {
        String::new()
    } else {
        model
    };
    Ok(tagger::ProviderConfig {
        provider,
        model,
        // The built-in tagger loads models from <app data>/models; the db lives
        // in that same app-data dir, so derive it rather than plumb AppHandle.
        model_dir: state.db_path.parent().map(|p| p.join("models")),
        runtime_dir: Some(vision_dirs(state).0),
        parallel: tagger::resolve_parallel(ai_parallel_setting(&conn)?, hardware::vram_mb()),
        // English unless a command that writes descriptions says otherwise.
        caption_language: None,
    })
}

/// The raw `ai_parallel` setting: how many photos to send at once, 0 = auto.
fn ai_parallel_setting(conn: &Connection) -> Result<usize, String> {
    Ok(db::get_setting(conn, "ai_parallel")
        .map_err(err)?
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0))
}

/// Ensures one on-device model bundle is present.
///
/// Returns the outstanding [`ai_setup::Requirement`] when it still needs
/// fetching and consent has not been given; `None` once it is installed —
/// downloading it on the spot when consent allows.
async fn ensure_onnx_model(
    app: &AppHandle,
    id: &str,
    agreed: bool,
) -> Result<Option<ai_setup::Requirement>, String> {
    let dir = ai_models_dir(app).map_err(err)?;
    // Deleted from Settings, or startup's copy failed: the installer's copy
    // is still there, so no download (and no question) is needed.
    if ai_models::BUNDLED.contains(&id) {
        ai_models::install_bundled(&bundled_model_sources(app), &dir, id);
    }
    let status = ai_models::statuses(&dir)
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("unknown model '{id}'"))?;
    if status.installed {
        return Ok(None);
    }
    if !agreed {
        return Ok(Some(ai_setup::Requirement::new(
            format!("the {} model", status.name),
            status.approx_mb,
        )));
    }
    download_ai_model(app.clone(), id.to_string()).await?;
    Ok(None)
}

/// What every AI purpose says while on-device AI is switched off.
const AI_SWITCHED_OFF: &str = "On-device AI is switched off — turn it back on in Settings → AI";

/// Makes local AI ready for `purpose`, doing whatever setup that takes.
///
/// Idempotent and cheap once everything is in place, so every AI entry point
/// calls it first rather than disabling itself behind a readiness flag. It
/// surveys the whole ladder before acting, which is why consent is one dialog
/// listing everything instead of a prompt per rung: nothing is fetched until
/// the call is repeated with `consented`. [`ai_setup::ask_first`] decides
/// whether the caller asks in between; only a yes to the vision model is kept.
#[tauri::command]
#[specta::specta]
pub(crate) async fn ensure_ai_ready(
    app: AppHandle,
    state: State<'_, AppState>,
    purpose: ai_setup::AiPurpose,
    consented: bool,
) -> Result<ai_setup::Readiness, String> {
    use ai_setup::{AiPurpose, Readiness, Requirement};

    if !cfg!(feature = "ai") {
        return Ok(Readiness::blocked(
            "none",
            "This build was compiled without AI support",
        ));
    }
    // Off is off: only tagging and captions reached the provider check
    // below, so with AI switched off a search still fetched its model, and
    // automatic analysis went on indexing and finding faces while Settings
    // said nothing runs. "off" lets the interface offer to turn it back on.
    if provider_configured(&state)? && matches!(ai_config(&state)?.provider, tagger::Provider::None)
    {
        return Ok(Readiness::blocked("off", AI_SWITCHED_OFF));
    }

    // The on-device purposes: ONNX bundles, no provider, no server. Every
    // missing one is listed at once, so consent is one dialog.
    let models = purpose.onnx_models();
    if !models.is_empty() {
        let mut needs = Vec::new();
        for id in models {
            if let Some(req) = ensure_onnx_model(&app, id, consented).await? {
                needs.push(req);
            }
        }
        if !needs.is_empty() {
            let label = needs[0].label.clone();
            return Ok(Readiness::needs(
                "local",
                &label,
                needs,
                true,
                hardware::metered(),
            ));
        }
        // Automatic analysis is set up: its tagger is the built-in one, so
        // that is the provider, unless one was chosen. Tagging waited
        // for a provider to be stored, and only the non-ONNX path stored one,
        // so a fresh install never tagged anything while Settings said it
        // would. Never the vision model: that stays the user's choice.
        if purpose == AiPurpose::Analysis && !provider_configured(&state)? {
            let conn = state.conn()?;
            db::set_setting(&conn, "ai_provider", ai_setup::DEFAULT_PROVIDER.as_str())
                .map_err(err)?;
        }
        return Ok(Readiness::ready(
            "local",
            models[models.len() - 1],
            format!("Ready · {} runs on this machine", purpose.label()),
        ));
    }

    // Tagging and captions need a provider. Never chosen means never asked, so
    // start from the built-in tagger and keep it.
    let mut cfg = ai_config(&state)?;
    if !provider_configured(&state)? {
        let chosen = ai_setup::DEFAULT_PROVIDER;
        let conn = state.conn()?;
        db::set_setting(&conn, "ai_provider", chosen.as_str()).map_err(err)?;
        cfg.provider = chosen;
    }

    match cfg.provider {
        tagger::Provider::None => Ok(Readiness::blocked("off", AI_SWITCHED_OFF)),

        tagger::Provider::Builtin => {
            if purpose == AiPurpose::Captions {
                // Not a setup problem: this provider has no `caption` at all,
                // so no download would fix it. Say which way out there is.
                return Ok(Readiness::blocked(
                    "builtin",
                    "Descriptions need the vision model — turn on Detailed \
                     descriptions in Settings → AI",
                ));
            }
            if let Some(req) = ensure_onnx_model(&app, "tagger", consented).await? {
                return Ok(Readiness::needs(
                    "builtin",
                    "clip",
                    vec![req],
                    true,
                    hardware::metered(),
                ));
            }
            Ok(Readiness::ready(
                "builtin",
                "clip",
                "Ready · built-in tagging, runs on this machine",
            ))
        }

        tagger::Provider::Vision => {
            let (runtime_dir, models_dir) = vision_dirs(&state);

            // An unset model is this machine's recommendation, not a failure.
            let model = match vlm::find(cfg.model.trim()) {
                Some(m) => m,
                None => {
                    let picked = vlm::find(vlm::recommended(hardware::probe().vram_mb))
                        .ok_or("no vision model is listed")?;
                    let conn = state.conn()?;
                    db::set_setting(&conn, "ai_model", picked.id).map_err(err)?;
                    picked
                }
            };

            // Everything that is missing is listed at once, so consent is one
            // dialog naming it all with its size.
            let have_runtime = llama_pack::installed(&runtime_dir);
            let have_model = ai_models::files_installed(model.files, &models_dir);
            let mut needs = Vec::new();
            if !have_runtime {
                needs.push(Requirement::new(
                    "the local AI runtime",
                    llama_pack::download_mb(),
                ));
            }
            if !have_model {
                needs.push(Requirement::new(
                    format!("the {} vision model", model.name),
                    vlm::size_mb(model.id),
                ));
            }

            // A yes here is kept, so the next model fetched (a change of model
            // in Settings) does not ask again — unless the connection may cost.
            if consented {
                let conn = state.conn()?;
                db::set_setting(&conn, ai_setup::CONSENT_KEY, "1").map_err(err)?;
            }
            let metered = if needs.is_empty() {
                Some(false)
            } else {
                hardware::metered()
            };
            let agreed = consented || {
                let conn = state.conn()?;
                db::get_setting(&conn, ai_setup::CONSENT_KEY)
                    .map_err(err)?
                    .is_some()
                    && metered == Some(false)
            };
            if !needs.is_empty() && !agreed {
                return Ok(Readiness::needs("vision", model.id, needs, false, metered));
            }

            if !have_runtime || !have_model {
                // One setup at a time; one that waited finds what the other
                // fetched and fetches only what is still missing.
                let _one_at_a_time = state.vision_setup.lock().await;
                if !llama_pack::installed(&runtime_dir) {
                    install_runtime_pack(&app, &runtime_dir).await?;
                }
                if !ai_models::files_installed(model.files, &models_dir) {
                    download_vlm_model(&app, model, &models_dir).await?;
                }
            }

            // Load it now. The batch that follows will report progress from its
            // first photo instead of sitting at 0/N while the weights arrive.
            state.vlm_seq.fetch_add(1, Ordering::Relaxed);
            let warmed = tagger::ProviderConfig {
                model: model.id.to_string(),
                ..cfg
            };
            ensure_vlm_server(&app, &warmed).await?;
            // For the batch that follows; if none does (analysis waiting
            // behind Develop, or stopped), it goes after the idle time.
            unload_when_idle(&app, warmed);

            Ok(Readiness::ready(
                "vision",
                model.id,
                format!("Ready · {}", model.name),
            ))
        }
    }
}

/// How long a loaded vision model stays resident after a batch ends.
const VLM_IDLE: std::time::Duration = std::time::Duration::from_secs(120);

/// Loads the model before the batch's first photo, and tells the UI that is
/// what the wait is.
///
/// A cold 7B vision model takes tens of seconds to reach VRAM. That load used
/// to happen inside the first photo's own request, so the progress bar sat at
/// `0/N` with no explanation and the run looked hung — the "lag before tagging"
/// report. Warming first turns the same wait into a labelled step.
pub(crate) async fn warm_engine(
    app: &AppHandle,
    event: &str,
    cfg: &tagger::ProviderConfig,
    engine: &dyn tagger::Tagger,
    total: usize,
) {
    let _ = app.emit(
        event,
        ScanProgress {
            phase: "loading".into(),
            current: 0,
            total: count(total),
            message: "Loading model…".into(),
        },
    );
    // Not fatal: the first real request would load the model anyway, and would
    // report a far better error than this can if it cannot.
    let warmed = if cfg.provider == tagger::Provider::Vision {
        ensure_vlm_server(app, cfg)
            .await
            .map_err(anyhow::Error::msg)
    } else {
        engine.warm().await
    };
    if let Err(e) = warmed {
        eprintln!("warm: {e:#}");
    }
}

/// Starts `llama-server` on `cfg.model`'s files, unless it is already running
/// that model, and waits until it has loaded (see `sidecar`). A server running
/// another model is stopped first: one model at a time.
async fn ensure_vlm_server(app: &AppHandle, cfg: &tagger::ProviderConfig) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (runtime_dir, models_dir) = vision_dirs(&state);
    let binary =
        sidecar::llama_server_binary(&runtime_dir).ok_or("the local AI runtime isn't installed")?;
    let model = vlm::find(&cfg.model)
        .filter(|m| ai_models::files_installed(m.files, &models_dir))
        .ok_or_else(|| format!("the {} vision model isn't downloaded", cfg.model))?;
    // Chosen before the lock is taken: finding it can mean asking the server
    // which devices it sees, and the check-and-start below must not await.
    let device = vision_device(&binary).await;
    gpu_gate::GATE.set_vision_on_gpu(device.as_deref() != Some("none"));
    // Loading puts the model on the GPU: held until it answers, so an AI
    // mask does not run while it loads (`gpu_gate`).
    let _gpu = gpu_gate::GATE.vision().await;
    {
        // Held from the check to the start, so two passes starting together
        // cannot both start a server on the one port.
        let mut guard = lock_recover(&state.vlm_server);
        let running = match guard.as_mut() {
            Some((loaded, child)) => *loaded == cfg.model && matches!(child.try_wait(), Ok(None)),
            None => false,
        };
        if !running {
            if let Some((_, mut old)) = guard.take() {
                sidecar::stop(&mut old, sidecar::STOP_GRACE);
            }
            let slots = sidecar::llama_server_slots(cfg.parallel);
            let args = sidecar::llama_server_args(
                &models_dir.join(model.weights()),
                &models_dir.join(model.projector()),
                slots,
                device.as_deref(),
            );
            eprintln!(
                "vision: starting {} ({slots} at once) on {}{}",
                cfg.model,
                device.as_deref().unwrap_or("its default device"),
                hardware::gpu_note()
            );
            let output = session_log::server_log(&server_log_dir(&state), &cfg.model)
                .inspect_err(|e| eprintln!("vision: no server log: {e}"))
                .ok();
            let child = sidecar::spawn_llama_server(&binary, &args, output).map_err(err)?;
            sidecar::note_server(&server_pid_file(&state), &child);
            *guard = Some((cfg.model.clone(), child));
        }
    }
    // Loading a model takes a few seconds (the first run on a PC compiles its
    // GPU shaders, ~20 s); `/health` answers 503 until it has.
    let started = std::time::Instant::now();
    let link = sidecar::link();
    let client = sidecar::loopback_client(std::time::Duration::from_secs(5));
    for _ in 0..480 {
        // Ours must still be running for any answer to be its.
        let exited = lock_recover(&state.vlm_server)
            .as_mut()
            .is_none_or(|(_, child)| !matches!(child.try_wait(), Ok(None)));
        if exited {
            stop_vlm_server(&state);
            eprintln!(
                "vision: the server exited while loading {} (see llama-server.log)",
                cfg.model
            );
            return Err(format!("the {} vision model failed to load", cfg.model));
        }
        let ok = client
            .get(link.url("/health"))
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        if ok {
            // Before any photo: the server must know this run's key and
            // answer as the name only it was given.
            let models = match client
                .get(link.url("/v1/models"))
                .bearer_auth(&link.key)
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
                _ => String::new(),
            };
            if link.answers_as_ours(&models) {
                eprintln!(
                    "vision: {} ready after {:.1} s{}",
                    cfg.model,
                    started.elapsed().as_secs_f32(),
                    hardware::gpu_note()
                );
                return Ok(());
            }
            stop_vlm_server(&state);
            return Err(
                "another program is answering on the AI server's port, so no photo was sent to it"
                    .into(),
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    stop_vlm_server(&state);
    eprintln!("vision: {} took too long to load", cfg.model);
    Err(format!(
        "the {} vision model took too long to load",
        cfg.model
    ))
}

/// The `--device` the vision model's server runs on (see
/// `sidecar::choose_device`): on Windows the GPU the other models use, or
/// the CPU when there is no discrete GPU; on a Mac, Metal, llama.cpp's own
/// choice. Worked out once a session: finding it can mean running the server
/// to list its devices, and the hardware does not change.
async fn vision_device(binary: &Path) -> Option<String> {
    static DEVICE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    if !cfg!(windows) {
        return None;
    }
    if let Some(device) = DEVICE.get() {
        return device.clone();
    }
    let binary = binary.to_path_buf();
    let device = tokio::task::spawn_blocking(move || {
        // Integrated GPUs report next to no dedicated memory of their own; any
        // discrete GPU worth running a vision model on has gigabytes.
        let discrete = hardware::vram_mb().is_some_and(|mb| mb >= 2048);
        let devices = if discrete {
            sidecar::list_devices(&binary)
        } else {
            Vec::new()
        };
        sidecar::choose_device(&devices, &hardware::ai_gpu().1, discrete)
    })
    .await
    .unwrap_or(None);
    DEVICE.get_or_init(|| device).clone()
}

/// Stops the app's `llama-server`, freeing the model's memory.
pub(crate) fn stop_vlm_server(state: &AppState) {
    if let Some((_, mut child)) = lock_recover(&state.vlm_server).take() {
        sidecar::stop(&mut child, sidecar::STOP_GRACE);
        let _ = std::fs::remove_file(server_pid_file(state));
        eprintln!("vision: server stopped");
    }
}

/// Where the server's own output goes: the logs folder beside the runtime.
fn server_log_dir(state: &AppState) -> PathBuf {
    vision_dirs(state).0.with_file_name("logs")
}

/// Where the running server's process id is noted (`sidecar::note_server`).
fn server_pid_file(state: &AppState) -> PathBuf {
    vision_dirs(state).0.with_file_name("llama-server.pid")
}

/// Releases the model's VRAM once nothing has used it for [`VLM_IDLE`].
///
/// Unloading the instant a batch ended meant a tag pass followed by a caption
/// pass — the order people actually run them in — reloaded several gigabytes
/// for nothing. The sequence token makes deferring it safe: any batch that
/// starts or finishes in the meantime bumps `vlm_seq`, and this task then sees
/// a newer value and leaves the model alone.
fn unload_when_idle(app: &AppHandle, cfg: tagger::ProviderConfig) {
    // Only the vision model's server holds a model between batches.
    if cfg.provider != tagger::Provider::Vision {
        return;
    }
    let token = app
        .state::<AppState>()
        .vlm_seq
        .fetch_add(1, Ordering::Relaxed)
        + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(VLM_IDLE).await;
        let state = app.state::<AppState>();
        let superseded = state.vlm_seq.load(Ordering::Relaxed) != token;
        if !superseded && state.vlm_busy.load(Ordering::Relaxed) == 0 {
            stop_vlm_server(&app.state::<AppState>());
        }
    });
}

/// Arms [`unload_when_idle`] however a vision-model batch ends. A
/// batch claims the model when it starts, which cancels any unload already
/// waiting, and arming it again only at the end left the model loaded until
/// quit when the batch stopped early: a model that wouldn't build, a
/// catalogue that couldn't be read.
pub(crate) struct UnloadWhenDone {
    app: AppHandle,
    cfg: tagger::ProviderConfig,
}

impl UnloadWhenDone {
    /// Claims the model for a batch: an unload already waiting sees a newer
    /// token and leaves it, and none fires until the batch is over.
    pub(crate) fn claim(app: &AppHandle, cfg: &tagger::ProviderConfig) -> UnloadWhenDone {
        let state = app.state::<AppState>();
        state.vlm_seq.fetch_add(1, Ordering::Relaxed);
        state.vlm_busy.fetch_add(1, Ordering::Relaxed);
        UnloadWhenDone {
            app: app.clone(),
            cfg: cfg.clone(),
        }
    }
}

impl Drop for UnloadWhenDone {
    fn drop(&mut self) {
        self.app
            .state::<AppState>()
            .vlm_busy
            .fetch_sub(1, Ordering::Relaxed);
        unload_when_idle(&self.app, self.cfg.clone());
    }
}

#[derive(serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AiSettings {
    provider: String,
    model: String,
    default_model: String,
    /// Photos sent to the model at once as the user set it; 0 means auto.
    parallel: u32,
    /// What auto works out to on this machine, for the field's placeholder.
    parallel_auto: u32,
}

#[tauri::command]
#[specta::specta]
pub(crate) fn get_ai_settings(state: State<'_, AppState>) -> Result<AiSettings, String> {
    let cfg = ai_config(&state)?;
    let parallel = ai_parallel_setting(&state.conn()?)?;
    Ok(AiSettings {
        provider: cfg.provider.as_str().to_string(),
        model: cfg.model,
        default_model: cfg.provider.default_model().to_string(),
        parallel: parallel as u32,
        parallel_auto: tagger::resolve_parallel(0, hardware::vram_mb()) as u32,
    })
}

#[tauri::command]
#[specta::specta]
pub(crate) fn set_ai_settings(
    state: State<'_, AppState>,
    provider: String,
    model: String,
) -> Result<(), String> {
    let provider = tagger::Provider::parse(&provider);
    if provider == tagger::Provider::Vision {
        ensure_listed_model(&model)?;
    }
    let conn = state.conn()?;
    // Switching off remembers what it was, so switching on puts
    // detailed descriptions back rather than dropping to built-in tags.
    if provider == tagger::Provider::None {
        let cfg = ai_config(&state)?;
        if cfg.provider != tagger::Provider::None {
            let before = format!("{}|{}", cfg.provider.as_str(), cfg.model.trim());
            db::set_setting(&conn, "ai_before_off", &before).map_err(err)?;
        }
    }
    db::set_setting(&conn, "ai_provider", provider.as_str()).map_err(err)?;
    db::set_setting(&conn, "ai_model", model.trim()).map_err(err)
}

/// Switches on-device AI back on as it was before it was switched off:
/// with detailed descriptions if they were on. Built-in tags when nothing was
/// remembered, or the remembered vision model is no longer one offered.
#[tauri::command]
#[specta::specta]
pub(crate) fn turn_ai_on(state: State<'_, AppState>) -> Result<(), String> {
    let conn = state.conn()?;
    let before = db::get_setting(&conn, "ai_before_off")
        .map_err(err)?
        .unwrap_or_default();
    let (provider, model) = before.split_once('|').unwrap_or(("builtin", ""));
    let (provider, model) = match tagger::Provider::parse(provider) {
        tagger::Provider::Vision if ensure_listed_model(model).is_ok() => {
            (tagger::Provider::Vision, model)
        }
        tagger::Provider::None | tagger::Provider::Vision => (tagger::Provider::Builtin, ""),
        other => (other, model),
    };
    db::set_setting(&conn, "ai_provider", provider.as_str()).map_err(err)?;
    db::set_setting(&conn, "ai_model", model.trim()).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn provider_status(
    state: State<'_, AppState>,
) -> Result<tagger::ProviderStatus, String> {
    let cfg = ai_config(&state)?;
    let configured = provider_configured(&state)?;
    Ok(tagger::status(&cfg, configured).await)
}

/// Whether a provider has ever been chosen, by the user or by the setup ladder.
/// Distinguishes "off on purpose" from "not set up yet", which read the same in
/// [`ai_config`] but mean opposite things to the UI.
fn provider_configured(state: &AppState) -> Result<bool, String> {
    let conn = state.conn()?;
    Ok(db::get_setting(&conn, "ai_provider")
        .map_err(err)?
        .is_some_and(|v| !v.trim().is_empty()))
}

/// The AI model catalogue with per-model installed status, for the settings
/// list. Works in any build — the models are just files — so a plain build can
/// pre-fetch them for a later `--features ai` run.
#[tauri::command]
#[specta::specta]
pub(crate) fn ai_available_models(
    app: tauri::AppHandle,
) -> Result<Vec<ai_models::AiModelStatus>, String> {
    let dir = ai_models_dir(&app).map_err(err)?;
    Ok(ai_models::statuses(&dir))
}

/// Downloads a model's files, streaming progress on the `ai-download-progress`
/// event. Async so it does not block the UI thread for a multi-hundred-MB fetch.
#[tauri::command]
#[specta::specta]
pub(crate) async fn download_ai_model(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let dir = ai_models_dir(&app).map_err(err)?;

    // One download per model at a time. `app.state()` is fetched fresh (not held
    // across the await, which would not be `Send`); the guard is cleared on every
    // exit path below.
    if !lock_recover(&app.state::<AppState>().active_downloads).insert(id.clone()) {
        // Already on its way — the first-run setup and a first search ask
        // for the same model within seconds of each other on a new install.
        // Waiting for that download is what the second asker wanted; it used
        // to get "'tagger' is already downloading" as an error.
        while lock_recover(&app.state::<AppState>().active_downloads).contains(&id) {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        let installed = ai_models::statuses(&dir)
            .into_iter()
            .any(|m| m.id == id && m.installed);
        return if installed {
            Ok(())
        } else {
            Err(format!("the download of '{id}' didn't finish — try again"))
        };
    }

    let (app_cb, id_cb) = (app.clone(), id.clone());
    let cancel = app.state::<AppState>().jobs.start(&format!("model:{id}"));
    let result = ai_models::download(&id, &dir, &cancel, move |received, total| {
        let _ = app_cb.emit(
            "ai-download-progress",
            ai_models::AiDownloadProgress {
                id: id_cb.clone(),
                received,
                total,
                done: false,
                error: None,
            },
        );
    })
    .await;

    // Clear the in-flight guard now the download has settled (success or not), so
    // a retry after a failure is allowed.
    lock_recover(&app.state::<AppState>().active_downloads).remove(&id);
    if result.is_ok() {
        forget_loaded_model(&app.state::<AppState>(), &id);
    }

    // A terminal event either way, so the UI can settle its progress row without
    // guessing from the byte counts.
    let final_event = match &result {
        Ok(()) => ai_models::AiDownloadProgress {
            id: id.clone(),
            received: 1,
            total: 1,
            done: true,
            error: None,
        },
        Err(e) => ai_models::AiDownloadProgress {
            id: id.clone(),
            received: 0,
            total: 0,
            done: true,
            error: Some(errors::describe(e)),
        },
    };
    let _ = app.emit("ai-download-progress", final_event);
    result.map_err(err)
}

/// Removes a downloaded model's files.
#[tauri::command]
#[specta::specta]
pub(crate) fn delete_ai_model(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let dir = ai_models_dir(&app).map_err(err)?;
    forget_loaded_model(&app.state::<AppState>(), &id);
    ai_models::delete(&id, &dir).map_err(err)
}

/// Drops a loaded engine whose model files just changed, so the next use
/// loads what is on disk now. Without it, a face engine loaded for culling
/// before the People model arrived stayed unable to recognise anyone until
/// the app restarted, and a deleted model stayed loaded.
fn forget_loaded_model(state: &AppState, id: &str) {
    match id {
        "faces" | "people" => *lock_recover(&state.face_engine) = None,
        "tagger" => {
            *lock_recover(&state.search_embedder) = None;
            search::forget_model();
        }
        #[cfg(feature = "ai")]
        "denoise" => *lock_recover(&state.denoiser) = None,
        #[cfg(feature = "ai")]
        "erase" => *lock_recover(&state.eraser) = None,
        #[cfg(feature = "ai")]
        "subject" => develop::ai::forget_subject_model(),
        _ => {}
    }
}

/// A quick GPU/CPU capability probe, so the UI can advise which AI models are
/// realistic on this machine before a large download.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn hardware_report() -> Result<hardware::HardwareReport, String> {
    Ok(hardware::probe())
}

/// (runtime_dir, models_dir) for the vision model, under the app-data dir the
/// DB lives in: `llama-runtime/` for `llama-server`, and the models directory
/// every model shares (the vision models are in its `vision/`).
fn vision_dirs(state: &AppState) -> (PathBuf, PathBuf) {
    let data = state
        .db_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    (data.join("llama-runtime"), data.join("models"))
}

/// What the Ollama runtime this replaced left in the app's data
/// folder: the runtime (`runtime/`, 1.9 GB on Windows) and its model store
/// (`models/ollama/`, several GB). Both are the app's own downloads, and
/// nothing reads them any more. Best effort, like `remove_retired`: what
/// cannot be removed now is tried again next launch.
pub(crate) fn remove_ollama_leftovers(data_dir: &Path) {
    for dir in [
        data_dir.join("runtime"),
        data_dir.join("models").join("ollama"),
    ] {
        if dir.is_dir() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                eprintln!("ai: couldn't remove {}: {e}", dir.display());
            }
        }
    }
}

/// Downloads and installs llama.cpp's `llama-server` into the app-data
/// `llama-runtime/` dir, streaming progress on `vision-runtime-progress`
/// (`{stage, received, total, done, error}`, `stage` being `"downloading"`
/// then `"extracting"`). Run by the setup ladder, never by a button of its own.
async fn install_runtime_pack(app: &AppHandle, runtime_dir: &Path) -> Result<(), String> {
    let emitter = app.clone();
    let cancel = app.state::<AppState>().jobs.start("vision-runtime");
    let result = llama_pack::install(runtime_dir, &cancel, move |stage, received, total| {
        let _ = emitter.emit(
            "vision-runtime-progress",
            serde_json::json!({
                "stage": stage,
                "received": received,
                "total": total,
                "done": false,
                "error": null,
            }),
        );
    })
    .await;

    let payload = match &result {
        Ok(version) => serde_json::json!({
            "stage": "done", "received": 1, "total": 1, "done": true,
            "error": null, "version": version,
        }),
        Err(e) => serde_json::json!({
            "stage": "error", "received": 0, "total": 0, "done": true,
            "error": errors::describe(e),
        }),
    };
    let _ = app.emit("vision-runtime-progress", payload);
    result.map(|_| ()).map_err(err)
}

/// Downloads a vision model's files, pinned like every other model's, through
/// the same downloader. A multi-gigabyte fetch, so it streams
/// `vision-model-progress` (`{received, total, done, error}`) throughout — the
/// Activity panel picks that up wherever it was started from.
async fn download_vlm_model(
    app: &AppHandle,
    model: &vlm::VlmModel,
    models_dir: &Path,
) -> Result<(), String> {
    let emitter = app.clone();
    let cancel = app.state::<AppState>().jobs.start("vlm-model");
    let result = ai_models::download_files(
        model.name,
        vlm::size_mb(model.id),
        model.files,
        models_dir,
        &cancel,
        move |received, total| {
            let _ = emitter.emit(
                "vision-model-progress",
                serde_json::json!({
                    "received": received, "total": total, "done": false, "error": null,
                }),
            );
        },
    )
    .await;

    let payload = match &result {
        Ok(()) => serde_json::json!({ "received": 1, "total": 1, "done": true, "error": null }),
        Err(e) => serde_json::json!({
            "received": 0, "total": 0, "done": true, "error": format!("{e:#}"),
        }),
    };
    let _ = app.emit("vision-model-progress", payload);
    result.map_err(err)
}

#[derive(serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VlmModelInfo {
    id: String,
    name: String,
    params: String,
    size_gb: f32,
    min_vram_gb: u32,
    ocr: String,
    /// Licence of the weights — shown beside the model, as the ONNX list does.
    license: String,
    note: String,
    /// How the model fits this machine's VRAM.
    fit: vlm::Fit,
    /// The single best pick for this hardware.
    recommended: bool,
    /// Already downloaded.
    installed: bool,
}

/// Refuses a vision model outside the catalogue. The catalogue is the list of
/// models whose licences allow any use, commercial included, so nothing
/// else may be stored as the choice or downloaded on the user's behalf. Empty
/// means "pick for this machine" and is always allowed.
fn ensure_listed_model(model: &str) -> Result<(), String> {
    let model = model.trim();
    if model.is_empty() || vlm::is_listed(model) {
        Ok(())
    } else {
        Err(format!(
            "{model} isn't one of the vision models SourceLight can use — choose one from the list"
        ))
    }
}

/// The recommended vision models, each graded against the detected hardware and
/// marked if already downloaded — the data behind the model picker. A model
/// that won't fit is still listed (and selectable): the verdict informs, it
/// doesn't forbid.
#[tauri::command]
#[specta::specta]
pub(crate) async fn vlm_catalogue(state: State<'_, AppState>) -> Result<Vec<VlmModelInfo>, String> {
    let (_, models_dir) = vision_dirs(&state);
    let vram = hardware::probe().vram_mb;
    let recommended = vlm::recommended(vram);
    Ok(vlm::catalogue()
        .iter()
        .map(|m| VlmModelInfo {
            id: m.id.to_string(),
            name: m.name.to_string(),
            params: m.params.to_string(),
            size_gb: m.size_gb,
            min_vram_gb: m.min_vram_gb,
            ocr: m.ocr.to_string(),
            license: m.license.to_string(),
            note: m.note.to_string(),
            fit: vlm::fit(m.min_vram_gb, vram),
            recommended: m.id == recommended,
            installed: ai_models::files_installed(m.files, &models_dir),
        })
        .collect())
}

/// What the optional descriptions engine is using on disk — the runtime and any
/// vision models downloaded — for the Storage list in Settings → AI.
#[derive(serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalVlmStorage {
    /// The managed runtime is installed.
    runtime_installed: bool,
    /// Its approximate size, in MB.
    runtime_mb: u32,
    /// Catalogue models downloaded, read from disk so the answer does not
    /// depend on the server running.
    models: Vec<VlmModelInfo>,
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn local_vlm_storage(
    state: State<'_, AppState>,
) -> Result<LocalVlmStorage, String> {
    let (runtime_dir, models_dir) = vision_dirs(&state);
    let vram = hardware::probe().vram_mb;
    let recommended = vlm::recommended(vram);
    let models = vlm::catalogue()
        .iter()
        .filter(|m| ai_models::files_installed(m.files, &models_dir))
        .map(|m| VlmModelInfo {
            id: m.id.to_string(),
            name: m.name.to_string(),
            params: m.params.to_string(),
            size_gb: m.size_gb,
            min_vram_gb: m.min_vram_gb,
            ocr: m.ocr.to_string(),
            license: m.license.to_string(),
            note: m.note.to_string(),
            fit: vlm::fit(m.min_vram_gb, vram),
            recommended: m.id == recommended,
            installed: true,
        })
        .collect();
    Ok(LocalVlmStorage {
        runtime_installed: llama_pack::installed(&runtime_dir),
        runtime_mb: llama_pack::INSTALLED_MB,
        models,
    })
}

/// Turns detailed descriptions off and deletes what they downloaded: stops the
/// server, removes the runtime and the vision models. Descriptions already
/// written stay in the catalogue and stay searchable; tagging carries on with
/// the built-in model. Asked for by name from Settings → AI, behind a
/// confirmation, so a user can get the gigabytes back.
#[tauri::command]
#[specta::specta]
pub(crate) async fn remove_local_vlm(state: State<'_, AppState>) -> Result<(), String> {
    {
        let conn = state.conn()?;
        if tagger::Provider::parse(
            &db::get_setting(&conn, "ai_provider")
                .map_err(err)?
                .unwrap_or_default(),
        ) == tagger::Provider::Vision
        {
            db::set_setting(&conn, "ai_provider", tagger::Provider::Builtin.as_str())
                .map_err(err)?;
        }
    }
    stop_vlm_server(&state);
    // The process needs a moment to release its files on Windows.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let (runtime_dir, models_dir) = vision_dirs(&state);
    for dir in [&runtime_dir, &models_dir.join("vision")] {
        if dir.exists() {
            std::fs::remove_dir_all(dir).map_err(|e| {
                err(anyhow::Error::new(e).context(format!("couldn't remove {}", dir.display())))
            })?;
        }
    }
    Ok(())
}
