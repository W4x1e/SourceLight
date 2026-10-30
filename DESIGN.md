---
name: SourceLight
description: A flat, truly neutral dark shell for a raw photo editor — a "photographic instrument". Neutral greys with barely-there hairlines, state shown in white rather than a hue, a mid-grey key for the one primary action, muted green/amber/red verdicts, IBM Plex Sans for the interface and IBM Plex Mono for every value, squared corners, and nothing that competes with the frame.
colors:
  base: "#141414"
  panel: "#1b1b1b"
  raised: "#242424"
  hover: "#2c2c2c"
  line: "#262626"
  line-hover: "#363636"
  ink: "#ececec"
  muted: "#a8a8a8"
  faint: "#8c8c8c"
  dim: "#555555"
  accent: "#e6e6e6"
  accent-dim: "#4a4a4a"
  key: "#3c3c3c"
  key-hover: "#474747"
  on-key: "#ffffff"
  on-accent: "#141414"
  focus: "#e6e6e6"
  canvas-void: "#0e0e0e"
  positive: "#6fbf82"
  warning: "#d4a64e"
  danger: "#dc5f55"
  danger-ink: "#f2aaa4"
  scrim: "rgba(0, 0, 0, 0.7)"
  scrim-veil: "rgba(0, 0, 0, 0.4)"
  chip: "rgba(0, 0, 0, 0.6)"
  histogram-red: "rgba(224, 98, 91, 0.5)"
  histogram-green: "rgba(111, 191, 115, 0.5)"
  histogram-blue: "rgba(91, 141, 217, 0.5)"
  cue-warm: "#e8b23a"
  cue-cool: "#4a86d8"
  cue-green: "#5bb85e"
  cue-magenta: "#c74fc7"
  cue-neutral: "#8a8a90"
  label-red: "#e5484d"
  label-yellow: "#f5d90a"
  label-green: "#46a758"
  label-blue: "#3e63dd"
  label-purple: "#8e4ec6"
typography:
  display:
    fontFamily: "'IBM Plex Sans', ui-sans-serif, system-ui, 'Segoe UI', sans-serif"
    fontSize: "16px"
    fontWeight: 600
    lineHeight: "1.25"
    letterSpacing: "-0.01em"
  title:
    fontFamily: "'IBM Plex Sans', ui-sans-serif, system-ui, 'Segoe UI', sans-serif"
    fontSize: "14px"
    fontWeight: 600
    lineHeight: "20px"
    letterSpacing: "normal"
  body:
    fontFamily: "'IBM Plex Sans', ui-sans-serif, system-ui, 'Segoe UI', sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: "1.5"
    letterSpacing: "normal"
  caption:
    fontFamily: "'IBM Plex Sans', ui-sans-serif, system-ui, 'Segoe UI', sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: "1.5"
    letterSpacing: "normal"
  section-label:
    fontFamily: "'IBM Plex Sans', ui-sans-serif, system-ui, 'Segoe UI', sans-serif"
    fontSize: "12px"
    fontWeight: 600
    lineHeight: "1.5"
    letterSpacing: "normal"
  label:
    fontFamily: "'IBM Plex Sans', ui-sans-serif, system-ui, 'Segoe UI', sans-serif"
    fontSize: "11px"
    fontWeight: 500
    lineHeight: "1.45"
    letterSpacing: "normal"
  numeric:
    fontFamily: "'IBM Plex Mono', ui-monospace, 'Cascadia Mono', Consolas, monospace"
    fontSize: "11px–13px (inherits the size of its context)"
    fontWeight: 400
    lineHeight: "1.5"
    letterSpacing: "normal"
    fontFeature: "tnum"
rounded:
  sm: "2px"
  md: "3px"
  lg: "4px"
  full: "9999px"
spacing:
  3xs: "2px"
  2xs: "4px"
  xs: "6px"
  sm: "8px"
  md: "10px"
  lg: "12px"
  xl: "16px"
  2xl: "24px"
components:
  button-key:
    backgroundColor: "{colors.key}"
    textColor: "{colors.on-key}"
    fontWeight: 600
    rounded: "{rounded.md}"
    padding: "6px 12px"
  button-solid:
    backgroundColor: "{colors.raised}"
    textColor: "{colors.ink}"
    rounded: "{rounded.md}"
    padding: "6px 12px"
  button-ghost:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
    rounded: "{rounded.md}"
    padding: "6px 12px"
  segment-active:
    backgroundColor: "#2e2e2e"
    textColor: "{colors.ink}"
    indicator: "2px underline in {colors.accent}"
  mode-rail-button-active:
    backgroundColor: "{colors.raised}"
    textColor: "{colors.ink}"
    indicator: "3px bar on the left edge in {colors.accent}"
  chip:
    backgroundColor: "{colors.raised}"
    textColor: "{colors.muted}"
    rounded: "{rounded.md}"
  input:
    backgroundColor: "{colors.base}"
    textColor: "{colors.ink}"
    rounded: "{rounded.md}"
    focusBorder: "{colors.accent-dim}"
  section-label:
    textColor: "{colors.muted}"
    typography: "{typography.section-label}"
  dialog:
    backgroundColor: "{colors.panel}"
    rounded: "{rounded.lg}"
  toast:
    backgroundColor: "{colors.raised}"
    rounded: "{rounded.lg}"
---

# Design System: SourceLight

## Overview

**Creative North Star: "the photographic instrument."**

SourceLight should feel like a well-made instrument — a camera's menu system, a light meter, a colourist's grading panel — not like a web dashboard. An instrument is neutral so it can be trusted, dense because the operator knows it, and quiet until touched. So the shell is a set of flat, truly neutral greys with no blue cast and no warmth: a tinted shell casts that tint onto the judgement the user came to make. Nothing in the chrome is a colour except the verdicts, and those are muted.

The previous design read as machine-generated — a Fraunces serif for headings, Inter, a steel-blue accent, a bright white "key" slab, tracked uppercase micro-labels, a 1px outline round every box, and a sparkle icon on every AI feature. Each of those is on the list of what every generated UI reaches for. This system removes them on purpose; do not reintroduce them.

**The app is organised around a spine of modes.** The mode rail on the far left always says where you are: **Browse** (the faceted library and grid), **Cull** (a keyboard-first pass to pick and reject), **Places** (naming trips), **People** (faces), and **Develop** (the raw editor, opened on the focused photo). Below them sit **Activity** (background jobs) and **Settings**. Each mode owns its own header and body; there is no global toolbar of unrelated controls.

The atmosphere is **quiet, dense, and exact**. Controls rest at muted grey and resolve to ink when hovered or active. Depth is a stack of neutral greys and a hairline you only see when looking for it. Values read in a monospaced face, like a readout.

**Key characteristics:**

- Flat neutral greys from `#141414` to `#ececec`; no hue in the chrome
- State (selected, rated, focused, active) is **white**, never a colour
- The primary action is a **mid-grey key with white semibold ink** — heavy by weight, not a bright slab
- Verdicts are muted green / amber / red — a pick, a caution, a reject
- **IBM Plex Sans** for the interface; **IBM Plex Mono** for every value; no display or serif face
- Sentence-case section labels; no tracked uppercase
- Squared corners (2–4px); outlines only where a boundary is genuinely needed
- One 24×24 line-icon set at stroke 1.7; **no sparkle icon as an "AI" marker** — name the action instead
- Over-image controls go white-on-black, never tinted

## Colors

### Neutral

Four surfaces, two lines, three text inks and one decorative step. Lightness alone carries elevation.

- **Void** (`#0e0e0e`): The Develop and Cull canvases — a matte for the photo, darker than the shell.
- **Base** (`#141414`): The application ground — behind the grid, inside inputs, under a filmstrip.
- **Panel** (`#1b1b1b`): Every rail, header, footer, dialog and slide-over.
- **Raised** (`#242424`): Secondary buttons, chips, list rows, toasts, the active mode-rail tile.
- **Hover** (`#2c2c2c`): The hover response for every interactive surface. A state, never a resting fill.
- **Line** (`#262626`): The hairline — one step off the panel, so a divider is there without drawing the eye. Rails and headers use it; boxes mostly do not.
- **Line Hover** (`#363636`): The scrollbar thumb and keycap edges.
- **Ink** (`#ececec`) / **Muted** (`#a8a8a8`) / **Faint** (`#8c8c8c`): Primary, resting and secondary text. All three clear WCAG AA (4.5:1) on base, panel and raised — pinned by `src/styles.test.ts`.
- **Dim** (`#555555`): Dividers, disabled states, decoration. Never text.

### State and key

- **Accent** (`#e6e6e6`): State, and only state — a filled star, the active segment's underline, the checked facet, the active mode's bar, the curve line. It is white on purpose: state is legible without adding a colour to the room.
- **Accent Dim** (`#4a4a4a`): Focused input borders and quiet selected fills.
- **Key** (`#3c3c3c` → `#474747` on hover) with **On-Key** (`#ffffff`): The single primary action on a surface. Mid-grey with white semibold ink; it reads as primary by weight and tone. Never two on one surface.
- **Focus** (`#e6e6e6`): The 2px keyboard focus ring, on every focusable control.

### Verdict and status

Three colours, used for state and never as decoration. They double as the cull vocabulary.

- **Positive** (`#6fbf82`): A pick, a ready provider, a finished job.
- **Warning** (`#d4a64e`): Cautions — a configured but unreachable provider, a soft frame.
- **Danger** (`#dc5f55`): A reject, errors, destructive confirmation — as a triad (`/40` border, `/10` fill, **Danger Ink** `#f2aaa4` text on the tint), never a solid red button.

### Data colours (not chrome)

- **Histogram channels**: desaturated RGB at half alpha.
- **Slider cues**: temperature cool→warm, tint green→magenta, painted inside the track because they describe what the control does.
- **Colour labels**: the five Lightroom-order labels, assigned by the user. They are data, so the no-hue rule does not reach them.

### Named rules

**The No-Hue Rule.** The chrome carries no hue. State is white, the key is grey, and green/amber/red are the whole status vocabulary. Adding a brand colour or a tinted accent breaks the premise that anything chromatic on screen belongs to the photograph.

**The Over-Image Rule.** Controls drawn on a photograph switch to white strokes on black scrims — crop handles, the cull legend, before/after labels. Nothing tinted goes over the image, except the mask overlay, where the tint is the data.

## Typography

**Interface:** IBM Plex Sans (variable, 100–700). **Values:** IBM Plex Mono (400/500/600). Both SIL OFL 1.1; "Plex" is a Reserved Font Name, so the app ships IBM's own published split subsets unmodified (`src/fonts/`).

There is no display face. Headings, dialog titles and tallies are Plex Sans at semibold (`.font-display`), slightly tightened.

### Hierarchy

- **Display** (600, 16px): Dialog and slide-over titles, month headings in the grid, large tallies.
- **Title** (600, 14px): Workspace headers ("Cull", "Places"), adjustment group names.
- **Body** (400, 13px/1.5): Facet rows and prose.
- **Caption** (400, 12px/1.5): Metadata rows, hints, chips, status text, button labels — the majority of the UI.
- **Section label** (600, 12px, Muted, **sentence case**): Group headings inside rails and panels (`.section-label`).
- **Label** (11px): The floor — compact metadata and over-image pills.
- **Numeric** (Plex Mono, tabular): Every value that is compared or updates in place.

### Named rules

**The Tabular Rule.** Any number that changes in place or sits in a column carries `tabular-nums`, which also sets it in Plex Mono.

**The Sentence-Case Rule.** Labels are written and shown in sentence case. No `uppercase` with letter-spacing on UI text — a guard in `styles.test.ts` fails on `tracking-wide`. Plain `uppercase` survives only for real acronyms (JPEG, TIFF).

**The Eleven-Pixel Floor.** Nothing is set below 11px.

## Layout

A fixed desktop composition with no responsive breakpoints — the window enforces 940×620 minimum against a 1500×940 default. Adaptation happens through resizable panes and a reflowing grid.

**The shell.** The mode rail on the far left, full height; the active workspace fills the rest: a 52px header, a body, and (in Browse) a 28px status footer. Rails are `shrink-0`, content columns are `min-w-0`.

| Element | Size |
|---|---|
| Mode rail | 84px (68×52 stacked icon+label buttons) |
| Workspace / Develop / Cull header | 52px |
| Status footer | 28px |
| Side panels (filter rail, selection panel, cull verdict, lightbox aside, places rail) | `--panel-w`, 280px |
| Activity slide-over | 360px |
| Develop editing rail | user-resizable, 360px default (300–620px) |
| Icon button | 30×30px |

**The grid.** Column count comes from measured width (`ResizeObserver`), cell size 96–340px, default 168px, 6px gutter.

**The No-Breakpoint Rule.** Panels resize; they do not restack. Every centred modal is bounded by the viewport (`max-h-[…vh…]`, pinned by a test) so nothing falls off a 620px-tall window.

## Elevation and depth

Tonal layering: a surface reads as raised because it is lighter. Shadow (`shadow-2xl`) is reserved for things that float above the app — dialogs, toasts, the Activity slide-over, the lightbox photo — on a black scrim. Scrims do not blur; frosted glass is only used where a control sits on a photograph.

## Shapes

Squared. **2px** — thumbnails, small badges, dismiss boxes. **3px** — buttons, inputs, chips, segmented controls. **4px** — dialogs, toasts, cards, mode-rail tiles. **Full** — only round things: colour-label dots, progress bars, the activity pip.

The **active segment or tab** (`.seg-on`, `.tool-btn.tool-on`) is a flat step up in tone (`#2e2e2e`) with a 2px white underline. The two `--seg-on-*` variables are its single knob.

## Components

### Buttons

Three variants, one geometry, all flat. **Key** (`.btn-accent`) — mid-grey fill, white semibold ink, one per surface. **Solid** (`.btn-solid`) — Raised fill, no outline; the default for real actions. **Ghost** (`.btn-ghost`) — transparent, hovers to Hover. Disabled is 40% opacity. A button is sized to its label; a full-width primary is the exception, not the pattern.

### Mode rail

A 84px spine: the wordmark glyph, then Browse / Cull / Places / People / Develop as stacked icon+label tiles, a spacer, Activity and Settings. The active mode is a Raised tile with a 3px white bar on its left edge.

### AI features

Name the action — "Suggest looks", "Select subject", "Search by content" — with the icon of what it does. No sparkle glyph standing in for "AI"; the feature is a tool, not a novelty.

### Dialogs and toasts

In-app, never the webview's `alert`/`confirm`/`prompt`. A dialog is a Panel card on a `scrim` with a semibold title and a footer whose confirm is the Key (or the red triad when destructive). A toast is a Raised card, bottom-centre, with a status-coloured leading icon.

### Develop

Organised by what each part is for, so a control's place says what it does.

- **Header, three zones.** Left: back to the Library, and the Looks toggle for the panel that opens on that side. Centre: the file being edited. Right: history (undo, redo, the History list), then the ways out — Save as copy, and Export as the one key. Nothing that only changes the *view* goes up here.
- **View bar, under the photo.** Before / Split / After and the gamut warning on the left, zoom on the right. They change what you see, never the edit.
- **Looks panel, left.** Closed whenever Develop opens; the photo comes first.
- **Editing rail, right**, top to bottom: the histogram (a reading of the photo), the tool switch (Adjust / Crop / Mask / Retouch, `.tool-btn`; a tool holding edits shows a dot or a count), the chosen tool's controls — for Adjust, a whole-edit row (Auto, Reset all, one-group-at-a-time) over the groups — and at the foot Copy / Paste / Previous, always in the same place.
- **Groups** are named at Title size with a dot when changed and a Reset beside the toggle. One idea per group: no tabs that hide another tab's edits.

### Sliders

`NumberSlider`, one line each: **label · track · value**, in columns (`--slider-label-w`, `--slider-value-w`) so a panel reads like an instrument's readout. The track is a 3px hairline; a neutral one **fills from where the control rests to the thumb**, and a bipolar one carries a tick at its rest, so which controls moved, which way and how far reads across a panel without reading a number. Colour-cue tracks (temperature, tint, saturation, the HSL bands) keep their gradient. The label is caption Muted and the value Plex Mono Faint at rest; both go to Ink once the control has moved. Double-click the label or track to return it to its **own** rest (`defaultValue`), not blindly to 0. The value is an editable field; arrows nudge a step (Shift: ten). A slider that shares its width with something else uses `layout="stacked"`.

### Everything else

Facet rows are whole-row hit targets with a live count computed against the other filters; chips are Raised, 3px, borderless; inputs recess (Base, darker than their panel) and focus to Accent Dim; the grid cell shows the photo and its rating at rest and the rest on hover; the histogram is three desaturated polygons; icons are one 24×24 line set at stroke 1.7 inheriting `currentColor`.

## Do's and don'ts

### Do

- **Do** keep the chrome neutral: state is white, the key is grey, verdicts are the only colours.
- **Do** set every changing number in `tabular-nums` (Plex Mono).
- **Do** write labels in sentence case.
- **Do** separate with tone and spacing first, a hairline second, an outline last.
- **Do** switch to white-on-black for anything drawn over a photograph.
- **Do** bound every modal by the viewport and keep the 940×620 floor working.
- **Do** name AI actions by what they do.

### Don't

- **Don't** add a serif or display face, or bring back Inter/Fraunces.
- **Don't** add a hue to the chrome — no brand colour, no tinted accent, no fourth status colour.
- **Don't** make the primary action a bright white slab, or put two keys on one surface.
- **Don't** use tracked uppercase micro-labels.
- **Don't** draw a 1px outline round every card, button and chip.
- **Don't** use a sparkle icon to mean "AI".
- **Don't** blur a scrim; blur is only for controls over a photo.
- **Don't** set UI text below 11px.
