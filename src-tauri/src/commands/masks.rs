//! AI masks, AI noise reduction and magic erase in Develop.

#[cfg(feature = "ai")]
use std::path::Path;
use tauri::{AppHandle, Emitter, State};

#[cfg(feature = "ai")]
use super::common::count;
use super::common::{ai_models_dir, err, persist_develop, with_develop};
#[cfg(feature = "ai")]
use crate::models::ScanProgress;
#[cfg(feature = "ai")]
use crate::sync::lock_recover;
use crate::{ai_models, develop, gpu_gate, hardware, AppState};
#[cfg(feature = "ai")]
use crate::{denoise, jobs};

/// Whether AI auto-masking can run: the feature must be compiled in *and* the
/// model file present. The UI uses this to enable the button or tell the user
/// where to put the model.
#[derive(serde::Serialize, specta::Type)]
pub(crate) struct AiStatus {
    /// Built with `--features ai`.
    enabled: bool,
    /// The model file exists on disk.
    model_present: bool,
    /// Absolute path where the model is expected.
    model_path: String,
}

/// The model file to use: BiRefNet in the precision this machine runs best
/// (fp16 on a GPU, `ai_models::best_file`), else whichever precision is
/// there, else the first `*.onnx` in the models dir — so any exported name
/// works with no renaming.
fn ai_model_file(app: &tauri::AppHandle) -> anyhow::Result<Option<std::path::PathBuf>> {
    let dir = ai_models_dir(app)?;
    if let Some(preferred) = ai_models::best_file(&dir, "birefnet.onnx") {
        return Ok(Some(preferred));
    }
    for entry in std::fs::read_dir(&dir)?.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("onnx"))
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn ai_status(app: tauri::AppHandle) -> Result<AiStatus, String> {
    let dir = ai_models_dir(&app).map_err(err)?;
    let present = ai_model_file(&app).map_err(err)?.is_some();
    Ok(AiStatus {
        enabled: cfg!(feature = "ai"),
        model_present: present,
        model_path: dir.to_string_lossy().into_owned(),
    })
}

/// Where an AI mask run has got to, as `ai-mask-stage` events, so Develop can
/// say why it is taking a while: `waiting` for the vision model to free the
/// graphics card, `loading` the model, then `subject` or `encoding` (reading
/// the photo for click-to-select) while it runs.
fn ai_mask_stage(app: &tauri::AppHandle, stage: &str) {
    let _ = app.emit("ai-mask-stage", stage);
}

/// Runs the app's own model for an AI mask once the graphics card is free,
/// saying so first if it is not.
fn ai_mask_turn(app: &tauri::AppHandle, what: &str) -> gpu_gate::LocalTurn<'static> {
    if gpu_gate::GATE.vision_busy() {
        ai_mask_stage(app, "waiting");
    }
    gpu_gate::GATE.local(what)
}

/// Runs subject segmentation on the current edit and returns a ready-to-add AI
/// mask. The frontend adds it to the stack like any other mask.
///
/// Off the main thread, and the model runs with the develop lock released:
/// the second or two it takes froze the window, and a slider moved meanwhile
/// waited for the lock on the main thread. The photo is rendered for it under
/// the lock, and checked afterwards to be the one still open.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn ai_select_subject(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<develop::mask::Mask, String> {
    // The app downloads this model itself (catalogue id "subject", landing on
    // the `birefnet.onnx` that `ai_model_file` prefers), so point at that rather
    // than at a path to hand-copy a checkpoint into.
    let model = ai_model_file(&app).map_err(err)?.ok_or_else(|| {
        "The “Select Subject” model isn’t downloaded yet — get it in Settings → On-device models."
            .to_string()
    })?;
    let input = with_develop(&state, |dev| {
        dev.selection_input(develop::ai::BIREFNET.size).map_err(err)
    })?;
    eprintln!(
        "ai mask: select subject on {}x{}{}",
        input.width,
        input.height,
        hardware::gpu_note()
    );
    let started = std::time::Instant::now();
    let gpu = ai_mask_turn(&app, "select subject");
    let mask = develop::ai::select_subject(&model, &input, |stage| ai_mask_stage(&app, stage))
        .map_err(err)
        .inspect_err(|e| eprintln!("ai mask: failed: {e}"))?;
    drop(gpu);
    eprintln!("ai mask: done in {:.1} s", started.elapsed().as_secs_f32());
    with_develop(&state, |dev| dev.still_fits(&input).map_err(err))?;
    Ok(mask)
}

/// AI noise reduction on the photo open in Develop. `amount` sets the
/// Amount as a step in history, running the model first if it has not run on
/// this photo yet; `None` only runs it, for an edit that already has an Amount
/// (a photo reopened, or settings pasted). `focus` is the centre of the view
/// in framed UV when zoomed in: the model starts there. Cancellable as
/// `denoise`; a long run shows its result so far as `denoise-preview` events.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn develop_denoise(
    app: AppHandle,
    state: State<'_, AppState>,
    amount: Option<f32>,
    focus: Option<(f32, f32)>,
) -> Result<develop::DevelopInfo, String> {
    #[cfg(feature = "ai")]
    {
        let info = run_denoise(&app, &state, amount, focus, true)?;
        persist_develop(&state, info)
    }
    #[cfg(not(feature = "ai"))]
    {
        let _ = (app, state, amount, focus);
        Err("This build was compiled without AI features.".to_string())
    }
}

/// Magic erase on the photo open in Develop: what `selection` covers (a mask
/// from the masking tools, over the photo as framed now) is taken out and
/// the gap filled, as one step in history. `label` names it in the list.
/// LaMa runs with the develop lock released, a few seconds a window;
/// progress comes as `erase-progress`, and it is cancellable as `erase`.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn develop_erase(
    app: AppHandle,
    state: State<'_, AppState>,
    selection: develop::mask::Mask,
    label: String,
) -> Result<develop::DevelopInfo, String> {
    #[cfg(feature = "ai")]
    {
        let info = run_erase(&app, &state, &selection, label.trim())?;
        persist_develop(&state, info)
    }
    #[cfg(not(feature = "ai"))]
    {
        let _ = (app, state, selection, label);
        Err("This build was compiled without AI features.".to_string())
    }
}

/// Takes erase `index` out of the open photo's edit: the object shows again.
#[tauri::command]
#[specta::specta]
pub(crate) fn develop_remove_erase(
    state: State<'_, AppState>,
    index: u32,
) -> Result<develop::DevelopInfo, String> {
    let info = with_develop(&state, |dev| dev.remove_erase(index as usize).map_err(err))?;
    persist_develop(&state, info)
}

/// LaMa, loaded once from `<models>/erase`.
#[cfg(feature = "ai")]
fn eraser(
    app: &AppHandle,
    state: &AppState,
) -> Result<std::sync::Arc<develop::erase::Inpainter>, String> {
    let mut guard = lock_recover(&state.eraser);
    if let Some(engine) = guard.as_ref() {
        return Ok(std::sync::Arc::clone(engine));
    }
    let dir = ai_models_dir(app).map_err(err)?;
    let engine = std::sync::Arc::new(develop::erase::Inpainter::load(&dir).map_err(err)?);
    *guard = Some(std::sync::Arc::clone(&engine));
    Ok(engine)
}

/// Works out the fills for `selection` outside the develop lock and hands
/// them to Develop.
#[cfg(feature = "ai")]
fn run_erase(
    app: &AppHandle,
    state: &AppState,
    selection: &develop::mask::Mask,
    label: &str,
) -> Result<develop::DevelopInfo, String> {
    let engine = eraser(app, state)?;
    let mut input = with_develop(state, |dev| dev.erase_selection(selection).map_err(err))?;
    let cancel = state.jobs.start("erase");
    let emit = |current: usize, total: usize| {
        let _ = app.emit(
            "erase-progress",
            ScanProgress {
                phase: if current < total { "erase" } else { "done" }.into(),
                current: count(current),
                total: count(total),
                message: "Erasing".into(),
            },
        );
    };
    let started = std::time::Instant::now();
    let gpu = gpu_gate::GATE.local("magic erase");
    let made = develop::erase::run(
        &mut input.region,
        &input.plan,
        |window| {
            if cancel.is_cancelled() {
                anyhow::bail!(jobs::CANCELLED);
            }
            engine.fill(window)
        },
        emit,
    );
    drop(gpu);
    eprintln!(
        "erase: {} window(s){} in {:.1} s",
        input.plan.windows.len(),
        if input.plan.soft() { ", soft" } else { "" },
        started.elapsed().as_secs_f32()
    );
    let made = made.map_err(|e| {
        if cancel.is_cancelled() {
            jobs::CANCELLED.to_string()
        } else {
            err(e)
        }
    })?;
    let label = if label.is_empty() { "Object" } else { label };
    with_develop(state, |dev| {
        dev.attach_erase(&input, label, made).map_err(err)
    })
}

/// A look at AI noise reduction's result so far: Develop re-renders `frame`
/// of `photo_id` without its edit changing.
#[cfg(feature = "ai")]
#[derive(Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
struct DenoisePreview {
    photo_id: i32,
    frame: u32,
}

/// NAFNet, loaded once from `<models>/denoise`.
#[cfg(feature = "ai")]
pub(crate) fn denoise_engine(
    app: &AppHandle,
    state: &AppState,
) -> Result<std::sync::Arc<denoise::DenoiseEngine>, String> {
    let mut guard = lock_recover(&state.denoiser);
    if let Some(engine) = guard.as_ref() {
        return Ok(std::sync::Arc::clone(engine));
    }
    let dir = ai_models_dir(app).map_err(err)?;
    let engine = std::sync::Arc::new(denoise::DenoiseEngine::load(&dir).map_err(err)?);
    *guard = Some(std::sync::Arc::clone(&engine));
    Ok(engine)
}

/// Runs the model over the open photo's linear image and hands the result to
/// Develop. The model runs outside the develop lock: 35 s for 24 MP on a good
/// GPU, during which the sliders keep working on the undenoised image.
#[cfg(feature = "ai")]
fn run_denoise(
    app: &AppHandle,
    state: &AppState,
    amount: Option<f32>,
    focus: Option<(f32, f32)>,
    looks: bool,
) -> Result<develop::DevelopInfo, String> {
    let Some(develop::session::LinearImage {
        photo_id,
        pixels: original,
        width: w,
        height: h,
        focus: stored_focus,
    }) = with_develop(state, |dev| dev.linear_for_denoise(focus).map_err(err))?
    else {
        // It has run on this photo already: only the Amount changes.
        return with_develop(state, |dev| {
            let info = dev.info().map_err(err)?;
            match amount {
                Some(a) => {
                    let mut stack = info.stack;
                    stack.detail.ai_denoise = a;
                    dev.apply("AI noise reduction", stack).map_err(err)
                }
                None => Ok(info),
            }
        });
    };
    let cancel = state.jobs.start("denoise");
    let emit = |phase: &str, current: usize, total: usize| {
        let _ = app.emit(
            "denoise-progress",
            ScanProgress {
                phase: phase.into(),
                current: count(current),
                total: count(total),
                message: "Reducing noise".into(),
            },
        );
    };
    emit("loading", 0, 1);
    // A look at the result so far is shown at the Amount it will have.
    let look_amount = amount
        .or_else(|| {
            with_develop(state, |dev| {
                Ok(dev.current_stack().map(|s| s.detail.ai_denoise).ok())
            })
            .ok()
            .flatten()
        })
        .filter(|a| *a > 0.0)
        .unwrap_or(denoise::DEFAULT_AMOUNT);
    let mut shown = false;
    let mut look = |part: Vec<f32>| {
        let pixels = denoise::blend(&original, &denoise::to_f16(&part), look_amount);
        if let Ok(Some(frame)) = with_develop(state, |dev| {
            dev.show_denoise_preview(photo_id, &pixels).map_err(err)
        }) {
            shown = true;
            let _ = app.emit("denoise-preview", DenoisePreview { photo_id, frame });
        }
    };
    // The whole run, ~35 s for 24 MP: a tagging run pauses meanwhile.
    let gpu = gpu_gate::GATE.local("noise reduction");
    let result = denoise_engine(app, state).and_then(|engine| {
        // Hundreds of tiles; a percent at a time is all a bar can show.
        let mut last = usize::MAX;
        engine
            .denoise_steered(
                &original,
                w,
                h,
                denoise::Steer {
                    focus: stored_focus,
                    preview: if looks { Some(&mut look) } else { None },
                },
                |done, total| {
                    let step = done * 100 / total.max(1);
                    if step != last {
                        last = step;
                        emit("denoise", done, total);
                    }
                },
                || cancel.is_cancelled(),
            )
            .map_err(err)
    });
    drop(gpu);
    emit("done", 0, 0);
    let denoised = match result {
        Ok(d) => d,
        Err(e) => {
            // A look left part of the photo denoised: put the original back.
            if shown {
                if let Ok(Some(frame)) = with_develop(state, |dev| {
                    dev.restore_linear(photo_id, &original).map_err(err)
                }) {
                    let _ = app.emit("denoise-preview", DenoisePreview { photo_id, frame });
                }
            }
            return Err(if cancel.is_cancelled() {
                jobs::CANCELLED.into()
            } else {
                e
            });
        }
    };
    with_develop(state, |dev| {
        dev.attach_denoise(photo_id, original, denoised, amount)
            .map_err(err)
    })
}

/// Runs AI noise reduction before an export from Develop if the edit asks for
/// it and it has not run this session, so the file matches what the edit says
/// rather than the undenoised image a reopened photo shows until it is run.
pub(crate) fn ensure_denoised(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let info = with_develop(state, |dev| dev.info().map_err(err))?;
    if info.stack.detail.ai_denoise <= 0.0 || info.denoise_ready {
        return Ok(());
    }
    #[cfg(feature = "ai")]
    {
        // Develop hears of it, or its panel would still offer "Run now" over
        // a result the export just made.
        let info = run_denoise(app, state, None, None, false)?;
        let _ = app.emit("develop-updated", info);
        Ok(())
    }
    #[cfg(not(feature = "ai"))]
    {
        let _ = app;
        Err("This edit uses AI noise reduction, which this build does not include.".to_string())
    }
}

/// Click-to-select. `points` are in framed-image UV, each flagged include or
/// exclude. Returns a mask the frontend swaps in as the user keeps clicking —
/// the photo's embedding is cached, so only the first click pays for encoding.
///
/// Off the main thread, and that first encoding — about two seconds — runs
/// with the develop lock released, as [`ai_select_subject`]'s model does.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn ai_select_points(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    points: Vec<develop::sam2::Point>,
) -> Result<develop::mask::Mask, String> {
    let dir = ai_models_dir(&app).map_err(err)?.join("sam2");
    if !dir.exists() {
        // Catalogue id "click" fetches both halves into exactly this directory.
        return Err(
            "The “Click to Select” model isn’t downloaded yet — get it in Settings → \
             On-device models."
                .to_string(),
        );
    }
    #[cfg(feature = "ai")]
    {
        if let Some(mask) = with_develop(&state, |dev| dev.sam2_decode(&points).map_err(err))? {
            return Ok(mask);
        }
        // Not encoded yet, or encoded for another photo or framing: encode it
        // again, after an encode already under way, which may be this one.
        let _turn = lock_recover(&state.sam2_encoding);
        if let Some(mask) = with_develop(&state, |dev| dev.sam2_decode(&points).map_err(err))? {
            return Ok(mask);
        }
        encode_for_click(&app, &state, &dir)?;
        with_develop(&state, |dev| {
            dev.sam2_decode(&points)
                .map_err(err)?
                .ok_or_else(|| "the selection model lost its place — click again".to_string())
        })
    }
    #[cfg(not(feature = "ai"))]
    {
        let _ = (state, points);
        Err("this build was compiled without AI — rebuild with `--features ai`".to_string())
    }
}

/// Encodes the open photo for click to select, with the models already
/// loaded where there are some, and keeps the encoding for the clicks. The
/// caller holds `sam2_encoding`, so two never encode at once.
#[cfg(feature = "ai")]
fn encode_for_click(app: &AppHandle, state: &AppState, dir: &Path) -> Result<(), String> {
    let (input, loaded) = with_develop(state, |dev| {
        let input = dev.selection_input(develop::sam2::SAM2.size).map_err(err)?;
        Ok((input, dev.take_sam2_models()))
    })?;
    eprintln!(
        "ai mask: click to select, encoding {}x{}{}",
        input.width,
        input.height,
        hardware::gpu_note()
    );
    let started = std::time::Instant::now();
    let gpu = ai_mask_turn(app, "click to select");
    let mut models = match loaded {
        Some(models) => models,
        None => {
            ai_mask_stage(app, "loading");
            develop::sam2::Sam2::load(dir).map_err(err)?
        }
    };
    ai_mask_stage(app, "encoding");
    let embedding = models
        .encode(&input.pixels, input.width, input.height)
        .map_err(err)
        .inspect_err(|e| eprintln!("ai mask: failed: {e}"))?;
    drop(gpu);
    eprintln!(
        "ai mask: encoded in {:.1} s",
        started.elapsed().as_secs_f32()
    );
    with_develop(state, |dev| {
        dev.keep_sam2(&input, models, embedding).map_err(err)
    })
}

/// Readies magic erase while a selection is being made: click to select's
/// encoding of the photo, then LaMa, loaded and run once, so neither the
/// first click nor the first erase waits for them. Only for models already
/// downloaded, and quietly: a failure here is met again, and said, when the
/// tool is used.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn develop_prepare_erase(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    #[cfg(feature = "ai")]
    {
        let models = ai_models_dir(&app).map_err(err)?;
        let sam2 = models.join("sam2");
        if sam2.exists() {
            let _turn = lock_recover(&state.sam2_encoding);
            let ready = with_develop(&state, |dev| Ok(dev.sam2_ready()))?;
            if !ready {
                if let Err(e) = encode_for_click(&app, &state, &sam2) {
                    eprintln!("erase: click to select not readied: {e}");
                }
            }
        }
        if models.join(develop::erase::MODEL_FILE).exists() {
            let fresh = lock_recover(&state.eraser).is_none();
            match eraser(&app, &state) {
                Ok(engine) if fresh => {
                    let started = std::time::Instant::now();
                    if let Err(e) = engine.warm() {
                        eprintln!("erase: LaMa not warmed: {e:#}");
                    }
                    eprintln!(
                        "erase: LaMa loaded and warmed in {:.1} s",
                        started.elapsed().as_secs_f32()
                    );
                }
                Ok(_) => {}
                Err(e) => eprintln!("erase: LaMa not loaded: {e}"),
            }
        }
    }
    #[cfg(not(feature = "ai"))]
    let _ = (app, state);
    Ok(())
}
