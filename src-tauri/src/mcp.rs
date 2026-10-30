//! The editing server (docs/AI-EDIT.md): AI apps edit the photo open in
//! Develop through the Model Context Protocol.
//!
//! Any MCP client can connect: a chat app such as Claude Desktop, or LM Studio
//! running a local model. The model can do what a person can in Develop except
//! frame the photo: settings, looks, masks (the subject or an object picked by
//! AI, or drawn shapes) with their own adjustments, heal and clone, erase,
//! denoise. Every change is a step in history, so Undo takes it back.
//!
//! Three limits keep it safe to leave on:
//!
//! - **Localhost only, no browsers, and a token.** It binds 127.0.0.1, rmcp
//!   refuses any other `Host` (DNS rebinding), and any request carrying an
//!   `Origin` is refused, so a web page cannot reach it. Every request must
//!   carry this install's token ([`TOKEN_FILE`]), because other accounts on
//!   the machine share 127.0.0.1. Off until switched on in Settings → AI.
//! - **Only the open photo, and not its framing.** The crop, straighten, turn
//!   and flip stay the user's, and there is no file or library access.
//! - **A misspelt setting is an error, not a no-op.** `EditStack` reads with
//!   serde defaults, so an unknown name would be dropped without a word and the
//!   model would believe its change was made ([`merged`]).

use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::Mutex;

use axum::response::IntoResponse;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, JsonObject, ListToolsResult, PaginatedRequestParams, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{schemars, tool, tool_handler, tool_router, ErrorData, RoleServer, ServerHandler};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Map, Value};
use specta::Type;
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::common::{err, persist_develop, with_develop};
use crate::commands::develop::{
    auto_all_develop, auto_tone_develop, auto_wb_develop, undo_develop,
};
use crate::commands::masks::{ai_select_points, ai_select_subject, develop_denoise, develop_erase};
use crate::commands::presets::{list_luts, list_preset_groups};
use crate::develop::mask::{
    LocalAdjustments, Mask, MaskComponent, MaskMode, MaskShape, MAX_COMPONENTS, MAX_MASKS,
};
use crate::develop::{presets, spot::MAX_SPOTS, DevelopInfo, EditStack};
use crate::{db, AppState};

const ENABLED: &str = "edit_server_enabled";

/// Fixed rather than picked at random, so a client set up once keeps working
/// across launches.
pub const PORT: u16 = 47_811;

/// The file in the data folder that holds the server's token. Made once, so
/// a client set up with it keeps working; `--mcp` reads it from here.
const TOKEN_FILE: &str = "edit-server-token";

/// Long edge of the preview a model sees. Big enough to judge colour and tone,
/// small enough for a local vision model's context.
const PREVIEW_EDGE: u32 = 1024;

/// The `EditStack` fields `preview_edit` and `apply_edit` change, by their
/// JSON names. Masks have tools of their own, since an AI mask's coverage is a
/// bitmap only the model that made it can write; erases are `remove_object`'s.
const KEYS: &[&str] = &[
    "exposure",
    "warmth",
    "tint",
    "contrast",
    "highlights",
    "shadows",
    "whites",
    "blacks",
    "vibrance",
    "saturation",
    "mixerHue",
    "mixerSaturation",
    "mixerLuminance",
    "curve",
    "curveRed",
    "curveGreen",
    "curveBlue",
    "grading",
    "detail",
    "lens",
    "effects",
    "lut",
    "spots",
];

/// What each of [`KEYS`] does, and the masks, for the model. A test holds the
/// two together.
const GUIDE: &str = "\
Positions (x, y) are fractions of the preview's width and height, from the top left.

Edit settings (preview_edit, apply_edit). Send only the ones you change. null puts one back.
A clear but natural change is usually 0.2 to 0.5 on a -1..1 setting, and 0.3 to 1 stop
of exposure. Below 0.1 is hard to see.
- exposure: stops, -5..5. +1 is one stop brighter.
- warmth: -1..1 from the camera's white balance. + is warmer.
- tint: -1..1. + is more magenta, - more green.
- contrast, highlights, shadows, whites, blacks: -1..1, 0 is neutral.
- vibrance: -1..1, boosts muted colours most. saturation: -1..1, all colours equally.
- mixerHue, mixerSaturation, mixerLuminance: 8 numbers each, -1..1, for the bands
  Red, Orange, Yellow, Green, Aqua, Blue, Purple, Magenta. Send all 8.
- curve: tone curve points [{\"x\":0,\"y\":0},...], both 0..1. [] is a straight line.
  curveRed, curveGreen, curveBlue: the same, per channel.
- grading: colour wheels. {shadows|midtones|highlights: {hue 0..360, saturation 0..1,
  luminance -1..1}, blending -1..1, balance -1..1}.
- detail: {clarity -1..1, texture -1..1, dehaze -1..1, sharpen 0..1, noise 0..1,
  colorNoise 0..1}. For AI noise reduction use the denoise tool.
- lens: {distortion -1..1, vignetting -1..1, chromatic -1..1}. Corrections for the lens.
- effects: {vignette -1..1 (- darkens corners), vignetteMidpoint 0..1, grain 0..1,
  grainSize 0.3..3, halation 0..1}.
- lut: {name, intensity 0..1}. A LUT from list_looks. name \"\" is none.
- spots: heal or clone patches, the whole list. [{targetX, targetY, sourceX, sourceY,
  radius (fraction of width), feather 0..1, heal true|false}]. Up to 40.
Presets: pass preset (an id from list_looks) and presetAmount (1 as designed) too.

Masks (add_mask, edit_mask, remove_mask) confine adjustments to part of the photo.
A mask is built from parts, combined in order. Each part is one of:
- {select: \"subject\"}: the main subject, found by AI.
- {select: \"points\", points: [{x, y, include}]}: the object under the points, found by
  AI. include false carves a region out.
- {select: \"shape\", shape: ...}: {type: \"radial\", cx, cy, radiusX, radiusY, angle,
  feather} | {type: \"linear\", x1, y1, x2, y2} (full at 1, none at 2) |
  {type: \"luminance\", low, high, feather} (0..1) | {type: \"colorRange\", hue, range,
  feather} (fractions of the colour wheel).
Each part can have inverted: true, and mode \"add\" (default), \"subtract\" or \"intersect\".
The rest of the photo is the subject inverted. For any one thing that is not the main
subject, use points on it, not the subject.
Each mask says coversPercent: how much of the photo it covers. Check it, and the tinted
image, against what you meant. Fix a wrong mask with edit_mask or remove_mask.
Put each adjustment on the mask for the part of the photo it is meant for.
Mask adjustments, -1..1: exposure (stops), contrast, highlights, shadows, temperature,
tint, saturation, whites, blacks, clarity, texture.
Up to 8 masks and 16 parts in all.";

/// The current edit, as the model sees it: [`KEYS`], the masks without their
/// bitmaps, and how many erases there are.
fn view(stack: &EditStack) -> Map<String, Value> {
    let Ok(Value::Object(mut all)) = serde_json::to_value(stack) else {
        return Map::new();
    };
    all.retain(|k, _| KEYS.contains(&k.as_str()));
    let masks: Vec<Value> = stack
        .masks
        .iter()
        .enumerate()
        .map(|(index, m)| {
            let parts: Vec<Value> = m
                .components
                .iter()
                .map(|c| {
                    let shape = match &c.shape {
                        MaskShape::Ai { label, .. } => json!({"type": "ai", "label": label}),
                        MaskShape::Brush { strokes } => {
                            json!({"type": "brush", "dabs": strokes.len()})
                        }
                        other => serde_json::to_value(other).unwrap_or(Value::Null),
                    };
                    json!({"shape": shape, "inverted": c.inverted, "mode": c.mode})
                })
                .collect();
            json!({"index": index, "name": m.name, "coversPercent": covers(m), "parts": parts,
                   "adjustments": m.adjustments})
        })
        .collect();
    all.insert("masks".into(), masks.into());
    all.insert("erases".into(), stack.erases.len().into());
    all
}

/// Roughly how much of the frame `mask` covers, in percent to one decimal (a
/// small object rounds to 0.4, not to an empty-looking 0), so a model can
/// tell "the sculpture" from "everything but the couple" without trusting its
/// eyes on the tinted preview. `None` for brightness and colour masks, whose
/// coverage depends on the pixels.
fn covers(mask: &Mask) -> Option<f32> {
    const GRID: u32 = 64;
    if mask.components.iter().any(|c| {
        matches!(
            c.shape,
            MaskShape::Luminance { .. } | MaskShape::ColorRange { .. }
        )
    }) {
        return None;
    }
    // `MaskShape::coverage` calls an AI part full everywhere: its bitmap is
    // the renderer's to decode. Decoded here once per call.
    let bitmaps: Vec<Vec<u8>> = mask
        .components
        .iter()
        .map(|c| match &c.shape {
            MaskShape::Ai {
                width, height, rle, ..
            } => crate::develop::mask::decode_ai_sized(rle, *width, *height),
            _ => Vec::new(),
        })
        .collect();
    let mut total = 0.0;
    for j in 0..GRID {
        for i in 0..GRID {
            let (u, v) = (
                (i as f32 + 0.5) / GRID as f32,
                (j as f32 + 0.5) / GRID as f32,
            );
            total += mask.combined(|c| {
                let index = mask
                    .components
                    .iter()
                    .position(|other| std::ptr::eq(other, c))
                    .unwrap_or(0);
                let raw = match &c.shape {
                    MaskShape::Ai { width, height, .. } => {
                        let px = &bitmaps[index];
                        let x = ((u * *width as f32) as u32).min(width.saturating_sub(1));
                        let y = ((v * *height as f32) as u32).min(height.saturating_sub(1));
                        px.get((y * width + x) as usize)
                            .map_or(0.0, |&b| b as f32 / 255.0)
                    }
                    shape => shape.coverage(u, v),
                };
                if c.inverted {
                    1.0 - raw
                } else {
                    raw
                }
            });
        }
    }
    Some((1000.0 * total / (GRID * GRID) as f32).round() / 10.0)
}

/// `T` with `changes` merged into `current` (RFC 7396: a value replaces, null
/// resets, an object merges field by field), refusing names `T` does not have.
fn merge_into<T: Serialize + DeserializeOwned>(
    current: &T,
    changes: &Map<String, Value>,
) -> Result<T, String> {
    let patch = Value::Object(changes.clone());
    let mut json = serde_json::to_value(current).map_err(|e| e.to_string())?;
    json_patch::merge(&mut json, &patch);
    let out: T =
        serde_json::from_value(json).map_err(|e| format!("Those changes don't fit: {e}."))?;
    let back = serde_json::to_value(&out).map_err(|e| e.to_string())?;
    if let Some(path) = unknown(&patch, &back, "") {
        return Err(format!(
            "There is no setting `{path}`. Call get_photo for the names."
        ));
    }
    Ok(out)
}

/// `current` with a model's `changes` to [`KEYS`] merged in.
fn merged(current: &EditStack, changes: &Map<String, Value>) -> Result<EditStack, String> {
    if let Some(k) = changes.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!(
            "`{k}` can't be changed with this tool. These can: {}.",
            KEYS.join(", ")
        ));
    }
    let mut stack = merge_into(current, changes)?;
    if stack.spots.len() > MAX_SPOTS {
        return Err(format!("At most {MAX_SPOTS} spots."));
    }
    // shortcut: clamps the top-level sliders only; nested groups take what
    // they are given. Clamp those too if a model is seen sending wild values.
    stack.exposure = stack.exposure.clamp(-5.0, 5.0);
    for v in [
        &mut stack.warmth,
        &mut stack.tint,
        &mut stack.contrast,
        &mut stack.highlights,
        &mut stack.shadows,
        &mut stack.whites,
        &mut stack.blacks,
        &mut stack.vibrance,
        &mut stack.saturation,
    ]
    .into_iter()
    .chain(&mut stack.mixer_hue)
    .chain(&mut stack.mixer_saturation)
    .chain(&mut stack.mixer_luminance)
    {
        *v = v.clamp(-1.0, 1.0);
    }
    // The denoise tool runs its model first; a bare Amount would wait for it.
    stack.detail.ai_denoise = current.detail.ai_denoise;
    Ok(stack)
}

/// A mask's adjustments with `changes` merged in, each held to -1..1 (stops
/// for exposure, ±5 as the global one).
fn adjusted(
    current: &LocalAdjustments,
    changes: &Map<String, Value>,
) -> Result<LocalAdjustments, String> {
    let mut a = merge_into(current, changes)?;
    a.exposure = a.exposure.clamp(-5.0, 5.0);
    for v in [
        &mut a.contrast,
        &mut a.highlights,
        &mut a.shadows,
        &mut a.temperature,
        &mut a.tint,
        &mut a.saturation,
        &mut a.whites,
        &mut a.blacks,
        &mut a.clarity,
        &mut a.texture,
    ] {
        *v = v.clamp(-1.0, 1.0);
    }
    Ok(a)
}

/// The first name in `patch` that did not survive the round trip into `back`.
fn unknown(patch: &Value, back: &Value, at: &str) -> Option<String> {
    let (Value::Object(p), Value::Object(b)) = (patch, back) else {
        return None;
    };
    p.iter().find_map(|(k, v)| {
        let path = if at.is_empty() {
            k.clone()
        } else {
            format!("{at}.{k}")
        };
        match b.get(k) {
            None => Some(path),
            Some(bv) => unknown(v, bv, &path),
        }
    })
}

/// Built-in preset `id`, or saved preset `user:N`, on `stack`.
fn with_preset(
    app: &AppHandle,
    stack: &EditStack,
    id: &str,
    amount: f32,
) -> Result<EditStack, String> {
    if let Some(n) = id.strip_prefix("user:") {
        let n: i32 = n.parse().map_err(|_| format!("No preset called {id}."))?;
        let conn = app.state::<AppState>().conn()?;
        let preset = db::list_presets(&conn)
            .map_err(err)?
            .into_iter()
            .find(|p| p.id == n)
            .ok_or_else(|| format!("No preset called {id}."))?;
        return Ok(preset.applied_to(stack));
    }
    presets::apply(id, amount.clamp(0.0, 2.0), stack)
        .ok_or_else(|| format!("No preset called {id}. Call list_looks for the ids."))
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct EditRequest {
    /// The settings to change, e.g. {"exposure": 0.3, "warmth": 0.2}.
    #[serde(default)]
    changes: Map<String, Value>,
    /// A preset id from list_looks, applied before the changes.
    #[serde(default)]
    preset: Option<String>,
    /// How strongly to apply the preset. 1 is as designed.
    #[serde(default)]
    preset_amount: Option<f32>,
    /// apply_edit only: a few words for the history list, e.g. "Warmer sunset".
    #[serde(default)]
    label: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Select {
    /// The photo's main subject, found by AI: usually the people. Only that.
    Subject,
    /// One thing you point at, found by AI: a sign, a boat, a sculpture, one
    /// person of several. Put a point in the middle of it.
    Points,
    /// A drawn shape, given in `shape`.
    Shape,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PointRequest {
    x: f32,
    y: f32,
    /// False carves this region out of the selection.
    #[serde(default = "yes")]
    include: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, schemars::JsonSchema)]
struct Part {
    select: Select,
    /// For select "points".
    #[serde(default)]
    points: Vec<PointRequest>,
    /// For select "shape": radial, linear, luminance or colorRange.
    #[serde(default)]
    shape: Option<Map<String, Value>>,
    /// Swaps covered for uncovered.
    #[serde(default)]
    inverted: bool,
    /// "add" (default), "subtract" or "intersect", against the parts before it.
    #[serde(default)]
    mode: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct AddMaskRequest {
    /// What the mask covers, combined in order. See get_photo's guide.
    parts: Vec<Part>,
    /// What the mask does where it covers, e.g. {"exposure": 0.3, "saturation": -0.2}.
    #[serde(default)]
    adjustments: Map<String, Value>,
    /// A short name, e.g. "Subject" or "Sky".
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct EditMaskRequest {
    /// The mask's index from get_photo.
    index: usize,
    /// Changes to its adjustments. null puts one back to 0.
    #[serde(default)]
    adjustments: Map<String, Value>,
    /// Replaces what the mask covers.
    #[serde(default)]
    parts: Option<Vec<Part>>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct IndexRequest {
    /// The mask's index from get_photo.
    index: usize,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct RemoveObjectRequest {
    /// What to remove, as a mask's parts. Usually {select: "points", points: [...]}.
    parts: Vec<Part>,
    /// A few words for the history list, e.g. "Remove sign".
    #[serde(default)]
    label: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct DenoiseRequest {
    /// 0..1. 0 turns it off.
    amount: f32,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Auto {
    /// Exposure and tone.
    Tone,
    /// The colour of the light.
    WhiteBalance,
    /// Both.
    All,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct AutoRequest {
    what: Auto,
}

/// The masks' parts as components, running the AI selections. Called
/// without the develop lock: the models run outside it.
fn components(app: &AppHandle, parts: Vec<Part>) -> Result<Vec<MaskComponent>, String> {
    if parts.is_empty() {
        return Err("A mask needs at least one part.".into());
    }
    parts
        .into_iter()
        .map(|p| {
            let shape = match p.select {
                Select::Subject => ai_shape(ai_select_subject(app.clone(), app.state())?)?,
                Select::Points => {
                    if p.points.is_empty() {
                        return Err("select \"points\" needs points.".into());
                    }
                    let points = p
                        .points
                        .iter()
                        .map(|q| crate::develop::sam2::Point {
                            x: q.x.clamp(0.0, 1.0),
                            y: q.y.clamp(0.0, 1.0),
                            include: q.include,
                        })
                        .collect();
                    ai_shape(ai_select_points(app.clone(), app.state(), points)?)?
                }
                Select::Shape => drawn(p.shape.map(Value::Object))?,
            };
            Ok(MaskComponent {
                shape,
                inverted: p.inverted,
                mode: mode(p.mode)?,
            })
        })
        .collect()
}

/// A shape the model drew. Not an AI one, whose bitmap it cannot make, and
/// not a brush.
fn drawn(shape: Option<Value>) -> Result<MaskShape, String> {
    let shape: MaskShape = serde_json::from_value(shape.ok_or("select \"shape\" needs a shape.")?)
        .map_err(|e| format!("That shape doesn't fit: {e}."))?;
    if matches!(shape, MaskShape::Ai { .. } | MaskShape::Brush { .. }) {
        return Err("Shapes can be radial, linear, luminance or colorRange.".into());
    }
    Ok(shape)
}

fn mode(mode: Option<String>) -> Result<MaskMode, String> {
    serde_json::from_value(Value::String(mode.unwrap_or_else(|| "add".into())))
        .map_err(|_| "mode is \"add\", \"subtract\" or \"intersect\".".to_string())
}

/// The coverage an AI selection made. It returns a one-part mask.
fn ai_shape(mask: Mask) -> Result<MaskShape, String> {
    mask.components
        .into_iter()
        .next()
        .map(|c| c.shape)
        .ok_or_else(|| "The AI found nothing there.".to_string())
}

/// Puts `masks` within what the renderer holds.
fn check_masks(masks: &[Mask]) -> Result<(), String> {
    let parts: usize = masks.iter().map(|m| m.components.len()).sum();
    if masks.len() > MAX_MASKS || parts > MAX_COMPONENTS {
        return Err(format!(
            "At most {MAX_MASKS} masks and {MAX_COMPONENTS} parts in all. Remove one first."
        ));
    }
    Ok(())
}

/// What a tool sends back: the edit as text, then images.
struct Reply {
    text: String,
    images: Vec<String>,
}

fn answer(result: Result<Reply, String>) -> CallToolResult {
    match result {
        Ok(reply) => {
            let mut content = vec![ContentBlock::text(reply.text)];
            for url in reply.images {
                // `jpeg_data_url` gives a data URL; MCP wants the bare base64.
                let b64 = url.split_once(',').map_or(url.as_str(), |(_, b)| b);
                content.push(ContentBlock::image(b64, "image/jpeg"));
            }
            CallToolResult::success(content)
        }
        Err(e) => CallToolResult::error(vec![ContentBlock::text(e)]),
    }
}

/// Runs `f` off the async runtime: every tool renders, and some run a model.
async fn blocking<T: Send + 'static>(
    app: &AppHandle,
    f: impl FnOnce(&AppHandle) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || f(&app))
        .await
        .map_err(|e| e.to_string())?
}

/// `f` on the develop state, refusing when no photo is open.
fn develop<T>(
    app: &AppHandle,
    f: impl FnOnce(&mut crate::develop::DevelopState) -> Result<T, String>,
) -> Result<T, String> {
    with_develop(&app.state::<AppState>(), |dev| {
        if dev.open_photo_id().is_none() {
            return Err("No photo is open in Develop. Ask the user to open one.".into());
        }
        f(dev)
    })
}

/// A preview of `stack`, and with `mask` the same frame with that mask tinted.
fn previews(
    app: &AppHandle,
    stack: &EditStack,
    mask: Option<usize>,
) -> Result<Vec<String>, String> {
    develop(app, |dev| {
        let mut out = vec![dev.render_preview(stack, PREVIEW_EDGE).map_err(err)?];
        if let Some(i) = mask {
            let mut shown = stack.clone();
            shown.show_mask = Some(i as u32);
            out.push(dev.render_preview(&shown, PREVIEW_EDGE).map_err(err)?);
        }
        Ok(out)
    })
}

/// After a change that is already in the catalogue: tells Develop, and shows
/// the model the result.
fn changed(app: &AppHandle, info: DevelopInfo, mask: Option<usize>) -> Result<Reply, String> {
    let images = previews(app, &info.stack, mask)?;
    let text = serde_json::to_string(&view(&info.stack)).unwrap_or_default();
    let _ = app.emit("develop-updated", info);
    Ok(Reply { text, images })
}

/// Applies `stack` as one step in history, saves it, and shows the result.
fn commit(
    app: &AppHandle,
    label: &str,
    stack: EditStack,
    mask: Option<usize>,
) -> Result<Reply, String> {
    let info = develop(app, |dev| {
        let stack = dev.with_own_erases(stack.sanitised()).map_err(err)?;
        dev.apply(label, stack).map_err(err)
    })?;
    // Saved outside the develop lock, as the commands do.
    let info = persist_develop(&app.state::<AppState>(), info)?;
    changed(app, info, mask)
}

fn label(words: &str, fallback: &str) -> String {
    match words.trim() {
        "" => fallback.to_string(),
        w => format!("AI: {w}"),
    }
}

/// One change to the edit at a time. Each tool reads the edit, changes it and
/// applies the whole of it, so two at once (a client calling tools in
/// parallel, as Claude does) could each apply over the other's change. Taken
/// after any AI selection, which is the slow part and changes nothing.
static EDITING: Mutex<()> = Mutex::new(());

fn one_edit() -> std::sync::MutexGuard<'static, ()> {
    EDITING.lock().unwrap_or_else(|p| p.into_inner())
}

/// The edit `req` asks for, on the current one.
fn requested(app: &AppHandle, req: &EditRequest) -> Result<EditStack, String> {
    let mut stack = develop(app, |dev| dev.current_stack().map_err(err))?;
    if let Some(id) = req.preset.as_deref().filter(|id| !id.is_empty()) {
        stack = with_preset(app, &stack, id, req.preset_amount.unwrap_or(1.0))?;
    }
    merged(&stack, &req.changes)
}

#[derive(Clone)]
struct Editor {
    app: AppHandle,
}

#[tool_router]
impl Editor {
    #[tool(
        description = "The photo open in Develop: its current edit (settings and masks), a guide to every setting, and a preview image. Call this first."
    )]
    async fn get_photo(&self) -> CallToolResult {
        answer(
            blocking(&self.app, |app| {
                let stack = develop(app, |dev| dev.current_stack().map_err(err))?;
                let text = serde_json::to_string(&view(&stack)).unwrap_or_default();
                Ok(Reply {
                    text: format!("{text}\n\n{GUIDE}"),
                    images: previews(app, &stack, None)?,
                })
            })
            .await,
        )
    }

    #[tool(
        description = "The presets (built in and the user's own) and LUTs, with their ids, for preview_edit and apply_edit."
    )]
    async fn list_looks(&self) -> CallToolResult {
        answer(
            blocking(&self.app, |app| {
                let state = app.state::<AppState>();
                let built_in: Vec<Value> = list_preset_groups()
                    .into_iter()
                    .flat_map(|g| {
                        g.presets.into_iter().map(move |p| {
                            json!({"id": p.id, "name": p.name, "group": g.name, "description": p.description})
                        })
                    })
                    .collect();
                let saved: Vec<Value> = db::list_presets(&state.conn()?)
                    .map_err(err)?
                    .into_iter()
                    .map(|p| json!({"id": format!("user:{}", p.id), "name": p.name}))
                    .collect();
                let luts = list_luts(state)?;
                let text = json!({"presets": built_in, "yourPresets": saved, "luts": luts});
                Ok(Reply {
                    text: text.to_string(),
                    images: Vec::new(),
                })
            })
            .await,
        )
    }

    #[tool(
        description = "Try a preset and/or changes to the edit settings and see the result. Nothing is saved. Returns the edit it would make and a preview image."
    )]
    async fn preview_edit(&self, Parameters(req): Parameters<EditRequest>) -> CallToolResult {
        answer(
            blocking(&self.app, move |app| {
                let stack = requested(app, &req)?;
                Ok(Reply {
                    text: serde_json::to_string(&view(&stack)).unwrap_or_default(),
                    images: previews(app, &stack, None)?,
                })
            })
            .await,
        )
    }

    #[tool(
        description = "Apply a preset and/or changes to the edit settings. They show in Develop at once and the user can undo them. Returns the new edit and a preview image."
    )]
    async fn apply_edit(&self, Parameters(req): Parameters<EditRequest>) -> CallToolResult {
        answer(
            blocking(&self.app, move |app| {
                let _one = one_edit();
                let stack = requested(app, &req)?;
                commit(app, &label(&req.label, "AI edit"), stack, None)
            })
            .await,
        )
    }

    #[tool(
        description = "Add a mask: adjustments that apply only to part of the photo. The main subject (found by AI), the rest of the photo (the subject inverted), one object you point at (points), or a gradient. Returns the new edit with how much of the photo the mask covers, a preview, and the preview with the mask tinted. Check both against what you meant before going on."
    )]
    async fn add_mask(&self, Parameters(req): Parameters<AddMaskRequest>) -> CallToolResult {
        answer(
            blocking(&self.app, move |app| {
                let components = components(app, req.parts)?;
                let adjustments = adjusted(&LocalAdjustments::default(), &req.adjustments)?;
                let _one = one_edit();
                let mut stack = develop(app, |dev| dev.current_stack().map_err(err))?;
                let index = stack.masks.len();
                let name = req.name.filter(|n| !n.trim().is_empty());
                stack.masks.push(Mask {
                    components,
                    adjustments,
                    name: name.clone(),
                });
                check_masks(&stack.masks)?;
                let words = name.as_deref().unwrap_or("mask");
                commit(app, &label(words, "AI mask"), stack, Some(index))
            })
            .await,
        )
    }

    #[tool(
        description = "Change a mask's adjustments, name or what it covers. Returns the new edit, a preview, and the preview with the mask tinted."
    )]
    async fn edit_mask(&self, Parameters(req): Parameters<EditMaskRequest>) -> CallToolResult {
        answer(
            blocking(&self.app, move |app| {
                let parts = req.parts.map(|p| components(app, p)).transpose()?;
                let _one = one_edit();
                let mut stack = develop(app, |dev| dev.current_stack().map_err(err))?;
                let mask = stack
                    .masks
                    .get_mut(req.index)
                    .ok_or_else(|| format!("There is no mask {}.", req.index))?;
                mask.adjustments = adjusted(&mask.adjustments, &req.adjustments)?;
                if let Some(parts) = parts {
                    mask.components = parts;
                }
                if let Some(name) = req.name {
                    mask.name = Some(name).filter(|n| !n.trim().is_empty());
                }
                check_masks(&stack.masks)?;
                commit(app, "AI: change mask", stack, Some(req.index))
            })
            .await,
        )
    }

    #[tool(description = "Remove a mask. The masks after it move down one index.")]
    async fn remove_mask(&self, Parameters(req): Parameters<IndexRequest>) -> CallToolResult {
        answer(
            blocking(&self.app, move |app| {
                let _one = one_edit();
                let mut stack = develop(app, |dev| dev.current_stack().map_err(err))?;
                if req.index >= stack.masks.len() {
                    return Err(format!("There is no mask {}.", req.index));
                }
                stack.masks.remove(req.index);
                commit(app, "AI: remove mask", stack, None)
            })
            .await,
        )
    }

    #[tool(
        description = "Remove an object from the photo and fill the gap, using AI. Takes a few seconds. Select the object with points on it."
    )]
    async fn remove_object(
        &self,
        Parameters(req): Parameters<RemoveObjectRequest>,
    ) -> CallToolResult {
        answer(
            blocking(&self.app, move |app| {
                let selection = Mask {
                    components: components(app, req.parts)?,
                    adjustments: LocalAdjustments::default(),
                    name: None,
                };
                let info = develop_erase(
                    app.clone(),
                    app.state(),
                    selection,
                    label(&req.label, "AI: remove object"),
                )?;
                changed(app, info, None)
            })
            .await,
        )
    }

    #[tool(
        description = "AI noise reduction at the given amount. The first run on a photo takes a while."
    )]
    async fn denoise(&self, Parameters(req): Parameters<DenoiseRequest>) -> CallToolResult {
        answer(
            blocking(&self.app, move |app| {
                let amount = req.amount.clamp(0.0, 1.0);
                let info = develop_denoise(app.clone(), app.state(), Some(amount), None)?;
                changed(app, info, None)
            })
            .await,
        )
    }

    #[tool(
        description = "Let the app set exposure and tone, white balance, or both, as a starting point."
    )]
    async fn auto_adjust(&self, Parameters(req): Parameters<AutoRequest>) -> CallToolResult {
        answer(
            blocking(&self.app, move |app| {
                let info = match req.what {
                    Auto::Tone => auto_tone_develop(app.state()),
                    Auto::WhiteBalance => auto_wb_develop(app.state()),
                    Auto::All => auto_all_develop(app.state()),
                }?;
                changed(app, info, None)
            })
            .await,
        )
    }

    #[tool(description = "Undo the last change to the edit, whoever made it.")]
    async fn undo(&self) -> CallToolResult {
        answer(
            blocking(&self.app, |app| {
                develop(app, |_| Ok(()))?;
                let info = undo_develop(app.state())?;
                changed(app, info, None)
            })
            .await,
        )
    }
}

#[tool_handler(
    name = "SourceLight",
    instructions = "Edits the photo open in SourceLight's Develop view. Call get_photo first. Try global changes with preview_edit and apply the one that matches what the user asked for with apply_edit. Use add_mask to edit part of the photo on its own: the subject, the background (the subject inverted), or one object picked with points. Check each mask's coversPercent and tinted image before going on."
)]
impl ServerHandler for Editor {
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools()))
    }
}

/// The tools as clients see them, with [`plain`] schemas.
fn tools() -> Vec<Tool> {
    Editor::tool_router()
        .list_all()
        .into_iter()
        .map(|mut tool| {
            tool.input_schema = plain(&tool.input_schema).into();
            tool
        })
        .collect()
}

/// A tool's input schema without what strict providers refuse. Gemini's
/// function calling takes a subset of OpenAPI's schema: no `$ref`, `oneOf` or
/// type lists, and few `format`s. Claude and local models read either, so
/// every client gets this one.
fn plain(schema: &JsonObject) -> JsonObject {
    let defs = schema
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut out = Value::Object(schema.clone());
    simplify(&mut out, &defs);
    let Value::Object(mut out) = out else {
        unreachable!("a schema object stays an object");
    };
    out.remove("$schema");
    out.remove("$defs");
    out
}

fn simplify(node: &mut Value, defs: &Map<String, Value>) {
    let Value::Object(map) = node else {
        return;
    };
    if let Some(Value::String(path)) = map.remove("$ref") {
        if let Some(Value::Object(def)) = path.strip_prefix("#/$defs/").and_then(|n| defs.get(n)) {
            for (k, v) in def {
                map.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    }
    // A unit enum with doc comments comes out as a `oneOf`: a `const` for each
    // variant with a comment, and one `enum` of those without.
    if let Some(Value::Array(options)) = map.remove("oneOf") {
        let named = |o: &Value| match (o.get("const"), o.get("enum")) {
            (Some(c), _) => Some(vec![c.clone()]),
            (None, Some(Value::Array(e))) => Some(e.clone()),
            _ => None,
        };
        if options.iter().all(|o| named(o).is_some()) {
            let values: Vec<Value> = options.iter().filter_map(named).flatten().collect();
            let said: Vec<String> = options
                .iter()
                .filter_map(|o| {
                    Some(format!(
                        "{}: {}",
                        o.get("const")?.as_str()?,
                        o.get("description")?.as_str()?.replace('\n', " ")
                    ))
                })
                .collect();
            map.insert("type".into(), "string".into());
            map.insert("enum".into(), values.into());
            if !said.is_empty() && !map.contains_key("description") {
                map.insert("description".into(), said.join(" ").into());
            }
        } else {
            map.insert("oneOf".into(), options.into());
        }
    }
    // `Option<T>` comes out as `["T", "null"]`; leaving it out of `required`
    // already says it is optional.
    if let Some(Value::Array(types)) = map.get("type") {
        if let Some(t) = types.iter().find(|t| *t != "null").cloned() {
            map.insert("type".into(), t);
        }
    }
    map.remove("format");
    if map.get("default") == Some(&Value::Null) {
        map.remove("default");
    }
    for key in ["items", "additionalProperties"] {
        if let Some(v) = map.get_mut(key) {
            simplify(v, defs);
        }
    }
    if let Some(Value::Object(props)) = map.get_mut("properties") {
        for v in props.values_mut() {
            simplify(v, defs);
        }
    }
}

/// `SourceLight --mcp`: the editing server over stdio, for AI apps that
/// start a server as a command rather than connecting to an address (Claude
/// Desktop, Codex). Passes each message to the running app's server and
/// prints what comes back, so the app must be open with the server on.
/// `identifier` is the app's, which names the data folder the token is in.
pub fn bridge_stdio(identifier: &str) {
    use std::io::{BufRead, Write};
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    runtime.block_on(async {
        // Never through a proxy: each reply carries the open photo.
        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap_or_default();
        let url = format!("http://127.0.0.1:{PORT}/mcp");
        let mut session: Option<String> = None;
        let mut out = std::io::stdout().lock();
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let id = serde_json::from_str::<Value>(&line)
                .ok()
                .and_then(|m| m.get("id").cloned());
            let mut request = client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream")
                .body(line);
            if let Some(s) = &session {
                request = request.header("Mcp-Session-Id", s);
            }
            // Read for each request: the app makes the token the first time
            // the server is switched on, which may be after this started.
            if let Some(token) = saved_token(identifier) {
                request = request.bearer_auth(token);
            }
            let replies = match request.send().await {
                Ok(response) => {
                    if let Some(s) = response
                        .headers()
                        .get("mcp-session-id")
                        .and_then(|v| v.to_str().ok())
                    {
                        session = Some(s.to_string());
                    }
                    let status = response.status();
                    let sse = response
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|t| t.contains("event-stream"));
                    let body = response.text().await.unwrap_or_default();
                    if status.is_success() {
                        messages(&body, sse)
                    } else {
                        // shortcut: a restarted app has forgotten the session, so the
                        // client is told to reconnect; re-initialising here would hide it.
                        if status == reqwest::StatusCode::NOT_FOUND {
                            session = None;
                        }
                        let said = format!("SourceLight refused that ({status}). Reconnect to it.");
                        id.map(|id| vec![failure(id, &said)]).unwrap_or_default()
                    }
                }
                Err(_) => id
                    .map(|id| vec![failure(id, NOT_RUNNING)])
                    .unwrap_or_default(),
            };
            for reply in replies {
                if writeln!(out, "{reply}").and_then(|()| out.flush()).is_err() {
                    return;
                }
            }
        }
    });
}

const NOT_RUNNING: &str =
    "SourceLight isn't open, or \"Let AI apps edit photos\" is off in its Settings > AI.";

/// The JSON-RPC messages in a server's reply: one JSON body, or the `data:`
/// lines of an event stream (empty ones are its keep-alives).
fn messages(body: &str, sse: bool) -> Vec<String> {
    if !sse {
        return (!body.trim().is_empty())
            .then(|| body.trim().to_string())
            .into_iter()
            .collect();
    }
    body.lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .collect()
}

/// A JSON-RPC error reply to request `id`.
fn failure(id: Value, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": message}}).to_string()
}

/// The token in data folder `dir`, if one has been made.
fn read_token(dir: &Path) -> Option<String> {
    let token = std::fs::read_to_string(dir.join(TOKEN_FILE)).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_string())
}

/// The server's token, made the first time it is asked for: 128 bits from the
/// OS random source, readable only by this user.
fn token(dir: &Path) -> std::io::Result<String> {
    if let Some(token) = read_token(dir) {
        return Ok(token);
    }
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
    let token = crate::hashing::hex(&bytes);
    let path = dir.join(TOKEN_FILE);
    crate::fsutil::write_atomic(&path, token.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(token)
}

/// The token as `--mcp` finds it, outside the app: in the data folder the app
/// uses, including a development build's `SOURCELIGHT_DATA_DIR`.
fn saved_token(identifier: &str) -> Option<String> {
    #[cfg(any(debug_assertions, feature = "dev-tools"))]
    if let Some(dir) =
        std::env::var_os(crate::commands::common::DATA_DIR_ENV).filter(|d| !d.is_empty())
    {
        return read_token(Path::new(&dir));
    }
    crate::identity::data_folders(identifier)
        .iter()
        .find_map(|dir| read_token(dir))
}

/// Whether a request carries `Authorization: Bearer <token>`, compared in
/// constant time.
fn authorised(headers: &axum::http::HeaderMap, token: &str) -> bool {
    let Some((scheme, given)) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
    else {
        return false;
    };
    let (given, token) = (given.trim().as_bytes(), token.as_bytes());
    scheme.eq_ignore_ascii_case("bearer")
        && given.len() == token.len()
        && given.iter().zip(token).fold(0, |d, (a, b)| d | (a ^ b)) == 0
}

/// Whether the server is on, for Settings → AI.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EditServerStatus {
    pub enabled: bool,
    pub running: bool,
    /// The address to give an MCP client.
    pub url: String,
    /// This program, for clients that start the server as a command with
    /// `--mcp` ([`bridge_stdio`]).
    pub command: String,
    /// What clients that connect to [`url`](Self::url) send as
    /// `Authorization: Bearer <token>`. Empty while the server is off.
    pub token: String,
    /// Why it is not running when it should be.
    pub error: Option<String>,
}

struct Server {
    /// Stops the running server.
    stop: Option<Box<dyn FnOnce() + Send>>,
    token: String,
    error: Option<String>,
}

static SERVER: Mutex<Server> = Mutex::new(Server {
    stop: None,
    token: String::new(),
    error: None,
});

fn enabled(conn: &rusqlite::Connection) -> anyhow::Result<bool> {
    Ok(db::get_setting(conn, ENABLED)?.as_deref() == Some("1"))
}

pub fn set_enabled(conn: &rusqlite::Connection, on: bool) -> anyhow::Result<()> {
    db::set_setting(conn, ENABLED, if on { "1" } else { "0" })
}

pub fn status(conn: &rusqlite::Connection) -> anyhow::Result<EditServerStatus> {
    let server = SERVER.lock().unwrap_or_else(|p| p.into_inner());
    Ok(EditServerStatus {
        enabled: enabled(conn)?,
        running: server.stop.is_some(),
        url: format!("http://127.0.0.1:{PORT}/mcp"),
        command: std::env::current_exe()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        token: if server.stop.is_some() {
            server.token.clone()
        } else {
            String::new()
        },
        error: server.error.clone(),
    })
}

/// Starts or stops the server to match the setting. Called at launch and
/// when the switch changes.
pub async fn apply(app: &AppHandle) {
    let on = app
        .state::<AppState>()
        .conn()
        .ok()
        .and_then(|c| enabled(&c).ok())
        .unwrap_or(false);
    {
        let mut server = SERVER.lock().unwrap_or_else(|p| p.into_inner());
        if on == server.stop.is_some() {
            return;
        }
        if let Some(stop) = server.stop.take() {
            stop();
        }
        server.error = None;
        if !on {
            return;
        }
        // Made under the lock, so two calls can't each write their own.
        let made = crate::commands::common::data_dir(app)
            .map_err(|e| e.to_string())
            .and_then(|dir| token(&dir).map_err(|e| e.to_string()));
        match made {
            Ok(token) => server.token = token,
            Err(e) => {
                eprintln!("edit server: could not make its token: {e}");
                server.error = Some(format!("Couldn't make the server's token ({e})."));
                return;
            }
        }
    }
    let result = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, PORT)).await;
    let mut server = SERVER.lock().unwrap_or_else(|p| p.into_inner());
    let listener = match result {
        Ok(l) => l,
        Err(e) => {
            eprintln!("edit server: could not listen on port {PORT}: {e}");
            server.error = Some(format!("Port {PORT} is in use by another app ({e})."));
            return;
        }
    };
    let config = StreamableHttpServerConfig::default().enforce_origin_validation();
    let token = config.cancellation_token.clone();
    let handle = app.clone();
    let service = StreamableHttpService::new(
        move || {
            Ok(Editor {
                app: handle.clone(),
            })
        },
        LocalSessionManager::default().into(),
        config,
    );
    let expected = server.token.clone();
    let router =
        axum::Router::new()
            .nest_service("/mcp", service)
            .layer(axum::middleware::from_fn(
                move |request: axum::extract::Request, next: axum::middleware::Next| {
                    let ok = authorised(request.headers(), &expected);
                    async move {
                        if ok {
                            next.run(request).await
                        } else {
                            axum::http::StatusCode::UNAUTHORIZED.into_response()
                        }
                    }
                },
            ));
    let shutdown = token.clone();
    tauri::async_runtime::spawn(async move {
        let served = axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled_owned().await })
            .await;
        if let Err(e) = served {
            eprintln!("edit server stopped: {e}");
        }
    });
    server.stop = Some(Box::new(move || token.cancel()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn every_key_is_an_edit_setting_and_explained() {
        let all = serde_json::to_value(EditStack::default()).unwrap();
        for k in KEYS {
            assert!(all.get(k).is_some(), "{k} is not an EditStack field");
            assert!(GUIDE.contains(k), "{k} is missing from GUIDE");
        }
        let adjustments = serde_json::to_value(LocalAdjustments::default()).unwrap();
        for k in adjustments.as_object().unwrap().keys() {
            assert!(GUIDE.contains(k.as_str()), "mask {k} is missing from GUIDE");
        }
    }

    #[test]
    fn changes_merge_into_the_current_edit() {
        let current = EditStack {
            contrast: 0.4,
            ..EditStack::default()
        };
        let out = merged(
            &current,
            &changes(json!({"exposure": 0.5, "detail": {"clarity": 0.2}})),
        )
        .unwrap();
        assert_eq!(out.exposure, 0.5);
        assert_eq!(out.contrast, 0.4, "untouched settings stay");
        assert_eq!(out.detail.clarity, 0.2);
        assert_eq!(out.detail.texture, current.detail.texture);
    }

    #[test]
    fn null_puts_a_setting_back() {
        let current = EditStack {
            contrast: 0.4,
            ..EditStack::default()
        };
        let out = merged(&current, &changes(json!({"contrast": null}))).unwrap();
        assert_eq!(out.contrast, 0.0);
    }

    #[test]
    fn misspelt_and_off_limits_names_are_refused() {
        let s = EditStack::default();
        let e = merged(&s, &changes(json!({"temperature": 0.3}))).unwrap_err();
        assert!(e.contains("temperature"), "{e}");
        let e = merged(&s, &changes(json!({"detail": {"sharpness": 0.3}}))).unwrap_err();
        assert!(e.contains("detail.sharpness"), "{e}");
        assert!(merged(&s, &changes(json!({"masks": []}))).is_err());
        assert!(merged(&s, &changes(json!({"geometry": {}}))).is_err());
        assert!(merged(&s, &changes(json!({"erases": []}))).is_err());
        assert!(merged(&s, &changes(json!({"exposure": "lots"}))).is_err());
    }

    #[test]
    fn values_are_held_in_range_and_ai_denoise_is_left_alone() {
        let s = EditStack::default();
        let out = merged(
            &s,
            &changes(json!({
                "exposure": 40, "contrast": -9,
                "mixerHue": [2, 0, 0, 0, 0, 0, 0, 0],
                "detail": {"aiDenoise": 1}
            })),
        )
        .unwrap();
        assert_eq!(out.exposure, 5.0);
        assert_eq!(out.contrast, -1.0);
        assert_eq!(out.mixer_hue[0], 1.0);
        assert_eq!(out.detail.ai_denoise, s.detail.ai_denoise);
    }

    #[test]
    fn spots_and_luts_can_be_set() {
        let out = merged(
            &EditStack::default(),
            &changes(json!({
                "lut": {"name": "film.cube", "intensity": 0.5},
                "spots": [{"targetX": 0.5, "targetY": 0.5, "sourceX": 0.4, "sourceY": 0.5,
                           "radius": 0.02, "feather": 0.5, "heal": true}]
            })),
        )
        .unwrap();
        assert_eq!(out.lut.name, "film.cube");
        assert_eq!(out.spots.len(), 1);
        let many: Vec<Value> = (0..41)
            .map(|_| json!({"targetX": 0.5, "targetY": 0.5, "sourceX": 0.4, "sourceY": 0.5, "radius": 0.02}))
            .collect();
        assert!(merged(&EditStack::default(), &changes(json!({"spots": many}))).is_err());
    }

    #[test]
    fn mask_adjustments_merge_and_hold_in_range() {
        let current = LocalAdjustments {
            saturation: 0.3,
            ..LocalAdjustments::default()
        };
        let out = adjusted(&current, &changes(json!({"exposure": 0.5, "clarity": 7}))).unwrap();
        assert_eq!(out.exposure, 0.5);
        assert_eq!(out.saturation, 0.3);
        assert_eq!(out.clarity, 1.0);
        assert!(adjusted(&current, &changes(json!({"vibrance": 0.2}))).is_err());
    }

    #[test]
    fn drawn_shapes_become_mask_parts_and_ai_ones_are_refused() {
        let radial = json!({"type": "radial", "cx": 0.5, "cy": 0.5, "radiusX": 0.2,
                            "radiusY": 0.2, "angle": 0, "feather": 0.5});
        assert!(matches!(drawn(Some(radial)), Ok(MaskShape::Radial { .. })));
        let linear = json!({"type": "linear", "x1": 0.5, "y1": 0, "x2": 0.5, "y2": 0.6});
        assert!(drawn(Some(linear)).is_ok());
        let ai = json!({"type": "ai", "label": "x", "width": 1, "height": 1, "rle": ""});
        assert!(drawn(Some(ai)).is_err());
        assert!(drawn(Some(json!({"type": "brush", "strokes": []}))).is_err());
        assert!(drawn(Some(json!({"type": "lasso"}))).is_err());
        assert!(drawn(None).is_err());
        assert_eq!(mode(None).unwrap(), MaskMode::Add);
        assert_eq!(mode(Some("subtract".into())).unwrap(), MaskMode::Subtract);
        assert!(mode(Some("xor".into())).is_err());
    }

    #[test]
    fn the_view_hides_mask_bitmaps_and_shows_what_can_change() {
        let mut stack = EditStack::default();
        stack.masks.push(Mask {
            components: vec![MaskComponent {
                shape: MaskShape::Ai {
                    label: "Subject".into(),
                    width: 2,
                    height: 2,
                    rle: "AAAA".into(),
                },
                inverted: true,
                mode: MaskMode::Add,
            }],
            adjustments: LocalAdjustments::default(),
            name: None,
        });
        let v = view(&stack);
        assert_eq!(v.len(), KEYS.len() + 2);
        assert!(!v.contains_key("version") && !v.contains_key("geometry"));
        let text = Value::Object(v).to_string();
        assert!(text.contains("Subject") && !text.contains("AAAA"), "{text}");
    }

    #[test]
    fn coverage_is_reported_for_ai_and_drawn_masks() {
        let part = |shape, inverted| MaskComponent {
            shape,
            inverted,
            mode: MaskMode::Add,
        };
        let mask = |components| Mask {
            components,
            adjustments: LocalAdjustments::default(),
            name: None,
        };
        // The left half of a 4x2 frame.
        let half = MaskShape::Ai {
            label: "Subject".into(),
            width: 4,
            height: 2,
            rle: crate::develop::mask::encode_ai(&[255, 255, 0, 0, 255, 255, 0, 0]),
        };
        assert_eq!(covers(&mask(vec![part(half.clone(), false)])), Some(50.0));
        assert_eq!(covers(&mask(vec![part(half, true)])), Some(50.0));
        let quarter = MaskShape::Ai {
            label: "Subject".into(),
            width: 2,
            height: 2,
            rle: crate::develop::mask::encode_ai(&[255, 0, 0, 0]),
        };
        assert_eq!(
            covers(&mask(vec![part(quarter.clone(), false)])),
            Some(25.0)
        );
        assert_eq!(covers(&mask(vec![part(quarter, true)])), Some(75.0));
        let band = MaskShape::Luminance {
            low: 0.2,
            high: 0.8,
            feather: 0.1,
        };
        assert_eq!(covers(&mask(vec![part(band, false)])), None);
        // One pixel in 256: a small object is not reported as nothing.
        let mut speck = vec![0u8; 16 * 16];
        speck[0] = 255;
        let speck = MaskShape::Ai {
            label: "Object".into(),
            width: 16,
            height: 16,
            rle: crate::develop::mask::encode_ai(&speck),
        };
        assert_eq!(covers(&mask(vec![part(speck, false)])), Some(0.4));
    }

    #[test]
    fn tool_schemas_are_plain_enough_for_every_provider() {
        // From the request types rather than `tools()`: reaching `Editor`,
        // which holds an `AppHandle`, stops the test binary from loading.
        use rmcp::handler::server::common::schema_for_type as schema;
        let tools = [
            ("preview_edit", plain(&schema::<EditRequest>())),
            ("add_mask", plain(&schema::<AddMaskRequest>())),
            ("edit_mask", plain(&schema::<EditMaskRequest>())),
            ("remove_mask", plain(&schema::<IndexRequest>())),
            ("remove_object", plain(&schema::<RemoveObjectRequest>())),
            ("denoise", plain(&schema::<DenoiseRequest>())),
            ("auto_adjust", plain(&schema::<AutoRequest>())),
        ]
        .map(|(name, input_schema)| Tool::new(name, "", input_schema));
        for tool in &tools {
            let text = Value::Object((*tool.input_schema).clone()).to_string();
            for banned in [
                "$ref",
                "$defs",
                "$schema",
                "oneOf",
                "anyOf",
                "\"format\"",
                "\"null\"",
            ] {
                assert!(!text.contains(banned), "{} has {banned}: {text}", tool.name);
            }
        }
        let add = tools.iter().find(|t| t.name == "add_mask").unwrap();
        let part = &add.input_schema["properties"]["parts"]["items"];
        assert_eq!(
            part["properties"]["select"]["enum"],
            json!(["subject", "points", "shape"])
        );
        assert!(part["properties"]["select"]["description"]
            .as_str()
            .unwrap()
            .starts_with("subject: "));
        assert_eq!(
            part["properties"]["points"]["items"]["required"],
            json!(["x", "y"])
        );
        assert_eq!(part["properties"]["shape"]["type"], "object");
        assert_eq!(part["properties"]["mode"]["type"], "string");
        let auto = tools.iter().find(|t| t.name == "auto_adjust").unwrap();
        assert_eq!(
            auto.input_schema["properties"]["what"]["enum"],
            json!(["tone", "whiteBalance", "all"])
        );
        // An enum with an undocumented variant mixes `const` and `enum`.
        let mixed = json!({"type": "object", "properties": {"x": {"oneOf": [
            {"type": "string", "enum": ["a"]},
            {"type": "string", "const": "b", "description": "Bee."}
        ]}}});
        let mixed = plain(mixed.as_object().unwrap());
        assert_eq!(mixed["properties"]["x"]["enum"], json!(["a", "b"]));
    }

    #[test]
    fn the_stdio_bridge_reads_both_kinds_of_reply() {
        assert_eq!(messages("{\"id\":1}\n", false), vec!["{\"id\":1}"]);
        assert!(messages("", false).is_empty());
        let stream =
            "data:\nid: 0\nretry: 3000\n\ndata: {\"method\":\"note\"}\n\ndata: {\"id\":2}\n\n";
        assert_eq!(
            messages(stream, true),
            vec!["{\"method\":\"note\"}", "{\"id\":2}"]
        );
        let f: Value = serde_json::from_str(&failure(json!(7), "no")).unwrap();
        assert_eq!(f["id"], 7);
        assert_eq!(f["error"]["message"], "no");
    }

    /// The token is made once and kept, and only a request carrying it is let
    /// through.
    #[test]
    fn requests_need_the_installs_token() {
        let dir = crate::test_support::Scratch::new("edit-server-token");
        assert_eq!(read_token(dir.path()), None);
        let made = token(dir.path()).unwrap();
        assert_eq!(made.len(), 32);
        assert_eq!(token(dir.path()).unwrap(), made, "made once");
        assert_eq!(read_token(dir.path()).as_deref(), Some(made.as_str()));

        let with = |value: &str| {
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(axum::http::header::AUTHORIZATION, value.parse().unwrap());
            headers
        };
        assert!(authorised(&with(&format!("Bearer {made}")), &made));
        assert!(authorised(&with(&format!("bearer {made}")), &made));
        assert!(!authorised(&axum::http::HeaderMap::new(), &made));
        assert!(!authorised(&with("Bearer "), &made));
        assert!(!authorised(&with(&format!("Bearer {made}x")), &made));
        assert!(!authorised(&with(&format!("Basic {made}")), &made));
        let mut other = made.clone().into_bytes();
        other[0] = if other[0] == b'0' { b'1' } else { b'0' };
        let other = String::from_utf8(other).unwrap();
        assert!(!authorised(&with(&format!("Bearer {other}")), &made));
    }

    #[test]
    fn too_many_masks_are_refused() {
        let one = |n: usize| Mask {
            components: (0..n)
                .map(|_| MaskComponent {
                    shape: MaskShape::Luminance {
                        low: 0.0,
                        high: 0.5,
                        feather: 0.1,
                    },
                    inverted: false,
                    mode: MaskMode::Add,
                })
                .collect(),
            adjustments: LocalAdjustments::default(),
            name: None,
        };
        assert!(check_masks(&vec![one(1); MAX_MASKS]).is_ok());
        assert!(check_masks(&vec![one(1); MAX_MASKS + 1]).is_err());
        assert!(check_masks(&[one(MAX_COMPONENTS + 1)]).is_err());
    }
}
