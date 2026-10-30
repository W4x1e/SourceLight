# AI editing

Let an AI model edit the photo open in Develop: the user says what they want
("warmer, like late afternoon", "match this reference"), and the model moves the
same settings a person would.

There are two ways in, built in this order:

1. **The editing server (built).** The app runs a Model Context Protocol (MCP)
   server. Any MCP client can connect, such as Claude Desktop or LM Studio with
   a local model. The user chats in that app, attaching a reference image there
   if they have one, and the model edits the photo in SourceLight.
2. **A box in Develop (next).** The user types the edit in Develop and picks a
   provider in Settings → AI: the built-in local model, another server on the
   machine (OpenAI-compatible: llama-server, Ollama, LM Studio), or a cloud
   service with the user's own key (OpenAI-compatible covers OpenAI and
   Gemini; Anthropic needs its own adapter). It drives the same three tools.

## The editing server

`src-tauri/src/mcp.rs`. Off by default. Settings → AI → AI apps turns it on and
shows the address, `http://127.0.0.1:47811/mcp` (Streamable HTTP).

| Tool | What it does |
|---|---|
| `get_photo` | The current edit (settings, masks without their bitmaps, how many erases), a guide to every setting, and a 1024px preview. |
| `list_looks` | Built-in presets, the user's saved presets (`user:<id>`) and LUTs. |
| `preview_edit(changes, preset, presetAmount)` | Merges a preset and/or changes into the current edit and returns the result and a preview. Nothing is saved. |
| `apply_edit(..., label)` | The same, applied as one history step ("AI: <label>"). |
| `add_mask(parts, adjustments, name)` | A mask with its own adjustments. Returns a preview and the preview with the mask tinted, so the model can check what it covers. |
| `edit_mask(index, adjustments, parts, name)` | Changes a mask's adjustments (merged), what it covers, or its name. |
| `remove_mask(index)` | Takes a mask off. |
| `remove_object(parts, label)` | AI erase (LaMa) of what the parts cover. |
| `denoise(amount)` | AI noise reduction, running the model first if it hasn't run on this photo. |
| `auto_adjust(what)` | The app's auto tone, white balance, or both. |
| `undo()` | Undoes the last step, whoever made it. |

`changes` is a partial edit (RFC 7396 merge patch over `EditStack`'s JSON):
send only what changes, `null` resets a setting. It covers the global
settings, the LUT and the heal/clone spots.

A mask is a list of **parts**, combined in order, each `add` (default),
`subtract` or `intersect`, and optionally inverted:

- `{select: "subject"}`: the main subject, by AI (BiRefNet, `ai_select_subject`).
- `{select: "points", points: [{x, y, include}]}`: the object under the points,
  by AI (SAM2 click-to-select, `ai_select_points`). Positions are fractions of
  the preview, from the top left.
- `{select: "shape", shape: {...}}`: a radial, linear, luminance or colour-range
  shape, as `MaskShape`'s JSON.

So "brighten the subject" is one part, "darken everything but the subject" is
the subject inverted, and "the person on the left, not their bag" is points
with an `include: false` on the bag. AI parts run the app's own models, with
the GPU gate, outside the develop lock.

### Limits

- **Localhost only, no browsers.** Binds 127.0.0.1. rmcp refuses other `Host`
  headers, and any request with an `Origin` header is refused, so a web page
  can't reach it.
- **A token.** Every request must carry `Authorization: Bearer <token>`, or it
  gets 401. Programs running as the user could read the catalogue anyway, but
  127.0.0.1 is shared by every account on the machine, and by sandboxed apps
  that can't read the user's files. The token is 128 random bits, made the
  first time the server is switched on and kept in `edit-server-token` in the
  data folder, so a client set up once keeps working. Settings shows it in the
  snippets for apps that connect to the address.
- **Everything but the framing.** The crop, straighten, turn and flip stay the
  user's. Brush masks aren't offered: a model can't paint usefully by dabs.
- **Unknown names are errors.** `EditStack` reads with serde defaults, so a
  misspelt name would otherwise do nothing and the model would think it worked.
- **Within the renderer's caps.** 8 masks, 16 parts, 40 spots. Top-level and
  mask adjustments are clamped to their slider ranges.
- **Only the open photo.** No library access and no files.
- **What leaves the computer is the AI app's choice.** SourceLight sends
  nothing out itself. The app gets the previews and settings it asks for and
  sends them wherever its model runs: an app that uses a cloud model, such as
  Claude Desktop, to that model's provider; LM Studio with a local model,
  nowhere. Settings says so beside the switch.

### Connecting

Settings → AI → AI apps shows each of these with the address, the token and
this program's path filled in, and a Copy button. SourceLight must be open,
with the photo in Develop.

| App | How | |
|---|---|---|
| Claude Desktop | Settings → Developer → Edit Config, then restart | `{"mcpServers": {"sourcelight": {"command": "<SourceLight exe>", "args": ["--mcp"]}}}` |
| Claude Code | a terminal | `claude mcp add --transport http sourcelight http://127.0.0.1:47811/mcp --header "Authorization: Bearer <token>"` |
| Gemini CLI | `~/.gemini/settings.json` | `{"mcpServers": {"sourcelight": {"httpUrl": "http://127.0.0.1:47811/mcp", "headers": {"Authorization": "Bearer <token>"}}}}` |
| Codex | a terminal | `codex mcp add sourcelight -- "<SourceLight exe>" --mcp` |
| LM Studio | its `mcp.json` | `{"mcpServers": {"sourcelight": {"url": "http://127.0.0.1:47811/mcp", "headers": {"Authorization": "Bearer <token>"}}}}` |

The ChatGPT app can't use it: its connectors are called from OpenAI's servers,
which can't reach a server on this computer. Codex runs locally and can.

**`--mcp` (stdio).** Claude Desktop and Codex start a server as a command
rather than connecting to an address. `SourceLight.exe --mcp` reads
JSON-RPC lines on stdin, posts each to the running app's server and prints
the replies (`mcp::bridge_stdio`). It reads the token from the data folder
itself (`SOURCELIGHT_DATA_DIR` in a development build), so these configs need
none, and never goes through a proxy. It runs before the app proper, at the
top of `run` in `lib.rs`, so single-instance doesn't turn it away. If the app
isn't open, or the switch is off, each request gets a JSON-RPC error saying
so.

**Plain schemas.** `list_tools` publishes each tool's schema flattened
(`mcp::plain`): no `$ref`/`$defs`, `oneOf` of constants as an `enum`,
`["T", "null"]` as `T`, no `format`. Gemini's function calling takes an
OpenAPI subset and rejects the rest; Claude and local models read either.

## Not done yet

- A Store (MSIX) install keeps its exe under `WindowsApps`, which other apps
  may not be allowed to start by path. Claude Desktop and Codex may need an app
  execution alias in the package manifest. Not tried. `--mcp` also has to
  read the token from the data folder, which MSIX redirects into the package,
  so a bridge started outside the package may not find it.
- AI parts need the `ai` build and their models downloaded (Settings → AI). A
  build without them returns the app's own "not downloaded" message.
- The in-app box and its provider settings (way in 2).
- A measured comparison of local models (Qwen3-VL 4B, 8B) against a cloud model
  on the same photos and prompts, in `scripts/model-eval`.
