/**
 * App-level constants and helpers, plus the single re-export point for the
 * generated types.
 *
 * The shapes themselves now come from `bindings.ts`, generated from the Rust
 * command signatures (`cargo run --bin export_bindings`). Components keep
 * importing from here, so the generated file stays an implementation detail —
 * and nothing in the app hand-mirrors a Rust struct any more.
 */
import type { Key } from "./i18n";
import type { EditStack, Mask, MaskComponent, MaskMode, MaskShape, PhotoFilter, PhotoSort, Spot, ToneShare } from "./bindings";

export type {
  AiSettings,
  Collection,
  SmartCollection,
  DevelopInfo,
  EditStack,
  Erase,
  ColorSpace,
  ExportFormat,
  ExportOptions,
  Preset,
  FacetCount,
  Facets,
  LocalAdjustments,
  Mask,
  MaskComponent,
  MaskMode,
  MaskShape,
  Stroke,
  Photo,
  PhotoFilter,
  Spot,
  Place,
  PlaceFacet,
  ProviderStatus,
  ScanResult,
  TripCluster,
} from "./bindings";

/** Payload of the `scan-progress` and `tag-progress` events. */
export type { ScanProgress as Progress } from "./bindings";

/**
 * An edit stack with every field present.
 *
 * The generated `EditStack` has optional fields because the Rust struct carries
 * `#[serde(default)]` — that is deliberate and load-bearing, since a stack
 * saved today must still load once the develop module has twenty more
 * parameters. It does mean the UI has to fill the blanks, exactly as the filter
 * does on load.
 */
type RequiredStack = { [K in keyof EditStack]-?: NonNullable<EditStack[K]> };

/**
 * `NonNullable` does not reach inside a tuple, so the mixer arrays would stay
 * `(number | null)[]` and force a null check at every slider. They are
 * overridden to plain numbers here and filled in by `withStackDefaults`.
 */
export type Stack = Omit<
  RequiredStack,
  | "mixerHue"
  | "mixerSaturation"
  | "mixerLuminance"
  | "curve"
  | "curveRed"
  | "curveGreen"
  | "curveBlue"
  | "grading"
  | "detail"
  | "lens"
  | "effects"
  | "lut"
  | "preset"
> & {
  mixerHue: Band8;
  mixerSaturation: Band8;
  mixerLuminance: Band8;
  curve: CurvePt[];
  curveRed: CurvePt[];
  curveGreen: CurvePt[];
  curveBlue: CurvePt[];
  grading: Grading;
  geometry: Geom;
  detail: Detail;
  lens: Lens;
  effects: Effects;
  lut: Lut;
  /** The built-in preset on the edit ("" for none), its amount (1 as
   *  designed), and its share of highlights, shadows, whites and blacks. */
  preset: { id: string; amount: number; tone: [ToneShare, ToneShare, ToneShare, ToneShare] };
};

/** A selected creative LUT and its blend. */
export type Lut = { name: string; intensity: number };

/** Manual lens corrections, all -1..1. */
export type Lens = { distortion: number; vignetting: number; chromatic: number };

/** Finishing effects: post-crop vignette, film grain and halation. */
export type Effects = {
  vignette: number;
  vignetteMidpoint: number;
  grain: number;
  grainSize: number;
  /** The warm bloom out of a highlight — the one thing on this list a sensor
   *  never produces on its own. */
  halation: number;
  halationThreshold: number;
  /** How far grain fades out of the highlights. Uniform noise is the giveaway
   *  of fake grain; on film the highlights read smoothest. */
  grainHighlightRolloff: number;
};

/** Local contrast, capture sharpening and noise reduction. */
export type Detail = {
  clarity: number;
  texture: number;
  sharpen: number;
  sharpenRadius: number;
  sharpenMasking: number;
  sharpenDetail: number;
  noise: number;
  colorNoise: number;
  dehaze: number;
  /** AI noise reduction's Amount, 0 when off. */
  aiDenoise: number;
};

/** Crop rectangle, normalised against the uncropped frame. */
export type Crop = { x: number; y: number; width: number; height: number };

/** Everything that changes the frame rather than the colour. */
export type Geom = {
  crop: Crop;
  straighten: number;
  quarterTurns: number;
  flip: boolean;
};

export const FULL_FRAME: Crop = { x: 0, y: 0, width: 1, height: 1 };

/**
 * Aspect presets, as width/height. `null` keeps whatever the drag produced.
 *
 * "Original" is resolved against the photo rather than stored, so the same
 * preset gives a portrait crop on a portrait frame.
 */
export const ASPECT_PRESETS: { label: string; labelKey?: Key; ratio: number | null }[] = [
  // Only "Free" is a word; the ratios read the same in every language. Show
  // `labelKey` through `t` when it is set, else `label`.
  { label: "Free", labelKey: "store.types.aspectFree", ratio: null },
  { label: "1:1", ratio: 1 },
  { label: "4:3", ratio: 4 / 3 },
  { label: "3:2", ratio: 3 / 2 },
  { label: "16:9", ratio: 16 / 9 },
  // Portrait counterparts — a ratio below 1 is taller than it is wide, and the
  // crop maths (`ratio / frameAspect`) reshapes the rectangle either way.
  { label: "3:4", ratio: 3 / 4 },
  { label: "2:3", ratio: 2 / 3 },
  { label: "9:16", ratio: 9 / 16 },
];

/** One colour-grading wheel, with every field resolved. */
export type Grade = { hue: number; saturation: number; luminance: number };

/** The three wheels plus the two controls that shape how they overlap. */
export type Grading = {
  shadows: Grade;
  midtones: Grade;
  highlights: Grade;
  blending: number;
  balance: number;
};

/** The tonal ranges, in the order the panel lays them out. */
export const GRADE_RANGES = ["shadows", "midtones", "highlights"] as const;
export type GradeRangeName = (typeof GRADE_RANGES)[number];

/**
 * A curve control point with both axes known.
 *
 * `CurvePoint` from the bindings has `number | null` on each axis, which is
 * how specta renders an `f32`. Dragging a point is not a place to be
 * null-checking coordinates, so they are resolved once on load.
 */
export type CurvePt = { x: number; y: number };

/** Hue bands of the colour mixer, in the order the Rust arrays use. */
export const BAND_NAMES = [
  "Red",
  "Orange",
  "Yellow",
  "Green",
  "Aqua",
  "Blue",
  "Purple",
  "Magenta",
] as const;

/**
 * One value per hue band.
 *
 * A fixed-length tuple because specta generates `[f32; 8]` as one — which is
 * stricter and better than `number[]`, since it makes a miscounted band list a
 * compile error.
 */
export type Band8 = [number, number, number, number, number, number, number, number];

const NO_BANDS = (): Band8 => [0, 0, 0, 0, 0, 0, 0, 0];

/** Fills any missing or null band with zero. */
const toBand8 = (v?: readonly (number | null)[] | null): Band8 =>
  BAND_NAMES.map((_, i) => v?.[i] ?? 0) as Band8;

/** A preset's share of a tone slider, when it holds none. */
const NO_SHARE: ToneShare = { added: 0, left: null };

export const DEFAULT_STACK: Stack = {
  // `CURRENT_VERSION` in develop/stack.rs: 2 opens at the camera's brightness,
  // 3 balances by temperature, 4 reads the mixer's hues in OkLCh.
  version: 4,
  exposure: 0,
  warmth: 0,
  tint: 0,
  contrast: 0,
  highlights: 0,
  shadows: 0,
  whites: 0,
  blacks: 0,
  vibrance: 0,
  saturation: 0,
  mixerHue: NO_BANDS(),
  mixerSaturation: NO_BANDS(),
  mixerLuminance: NO_BANDS(),
  curve: [],
  curveRed: [],
  curveGreen: [],
  curveBlue: [],
  grading: {
    shadows: { hue: 0, saturation: 0, luminance: 0 },
    midtones: { hue: 0, saturation: 0, luminance: 0 },
    highlights: { hue: 0, saturation: 0, luminance: 0 },
    blending: 0,
    balance: 0,
  },
  geometry: {
    crop: FULL_FRAME,
    straighten: 0,
    quarterTurns: 0,
    flip: false,
  },
  detail: {
    clarity: 0,
    texture: 0,
    sharpen: 0,
    // Not zero: a radius of zero describes no neighbourhood, so the slider
    // would start somewhere that cannot act even once the amount is raised.
    sharpenRadius: 1,
    sharpenMasking: 0,
    sharpenDetail: 0,
    noise: 0,
    colorNoise: 0,
    dehaze: 0,
    aiDenoise: 0,
  },
  lens: { distortion: 0, vignetting: 0, chromatic: 0 },
  masks: [],
  spots: [],
  erases: [],
  effects: {
    vignette: 0,
    vignetteMidpoint: 0.5,
    grain: 0,
    grainSize: 1,
    halation: 0,
    halationThreshold: 0.72,
    grainHighlightRolloff: 0.7,
  },
  lut: { name: "", intensity: 1 },
  preset: { id: "", amount: 1, tone: [NO_SHARE, NO_SHARE, NO_SHARE, NO_SHARE] },
};

/**
 * Written out field by field rather than spread over the defaults: a spread
 * would copy `undefined` straight over them. Being explicit also means adding a
 * parameter fails to compile here until it is handled.
 */
export function withStackDefaults(s?: EditStack | null): Stack {
  return {
    version: s?.version ?? DEFAULT_STACK.version,
    exposure: s?.exposure ?? DEFAULT_STACK.exposure,
    warmth: s?.warmth ?? DEFAULT_STACK.warmth,
    tint: s?.tint ?? DEFAULT_STACK.tint,
    contrast: s?.contrast ?? DEFAULT_STACK.contrast,
    highlights: s?.highlights ?? DEFAULT_STACK.highlights,
    shadows: s?.shadows ?? DEFAULT_STACK.shadows,
    whites: s?.whites ?? DEFAULT_STACK.whites,
    blacks: s?.blacks ?? DEFAULT_STACK.blacks,
    vibrance: s?.vibrance ?? DEFAULT_STACK.vibrance,
    saturation: s?.saturation ?? DEFAULT_STACK.saturation,
    // Fresh arrays each time — sharing the default array would let one photo's
    // slider edit leak into every other stack built from defaults.
    mixerHue: toBand8(s?.mixerHue),
    mixerSaturation: toBand8(s?.mixerSaturation),
    mixerLuminance: toBand8(s?.mixerLuminance),
    // An absent curve is the identity, which is why it can be empty rather
    // than needing a default shape.
    curve: (s?.curve ?? []).map((p) => ({ x: p.x ?? 0, y: p.y ?? 0 })),
    curveRed: (s?.curveRed ?? []).map((p) => ({ x: p.x ?? 0, y: p.y ?? 0 })),
    curveGreen: (s?.curveGreen ?? []).map((p) => ({ x: p.x ?? 0, y: p.y ?? 0 })),
    curveBlue: (s?.curveBlue ?? []).map((p) => ({ x: p.x ?? 0, y: p.y ?? 0 })),
    grading: toGrading(s?.grading),
    geometry: {
      crop: {
        x: s?.geometry?.crop?.x ?? 0,
        y: s?.geometry?.crop?.y ?? 0,
        // A missing crop means the whole frame, not a zero-sized one — the
        // difference between an unedited photo and an invisible one.
        width: s?.geometry?.crop?.width ?? 1,
        height: s?.geometry?.crop?.height ?? 1,
      },
      straighten: s?.geometry?.straighten ?? 0,
      quarterTurns: s?.geometry?.quarterTurns ?? 0,
      flip: s?.geometry?.flip ?? false,
    },
    detail: {
      clarity: s?.detail?.clarity ?? 0,
      texture: s?.detail?.texture ?? 0,
      sharpen: s?.detail?.sharpen ?? 0,
      sharpenRadius: s?.detail?.sharpenRadius ?? 1,
      sharpenMasking: s?.detail?.sharpenMasking ?? 0,
      sharpenDetail: s?.detail?.sharpenDetail ?? 0,
      noise: s?.detail?.noise ?? 0,
      colorNoise: s?.detail?.colorNoise ?? 0,
      dehaze: s?.detail?.dehaze ?? 0,
      aiDenoise: s?.detail?.aiDenoise ?? 0,
    },
    lens: {
      distortion: s?.lens?.distortion ?? 0,
      vignetting: s?.lens?.vignetting ?? 0,
      chromatic: s?.lens?.chromatic ?? 0,
    },
    // Masks arrive fully resolved from the backend, or absent. New ones are
    // built by `newMask` with every field set.
    masks: (s?.masks ?? []) as Mask[],
    // Spots likewise; `newSpot` sets every field.
    spots: (s?.spots ?? []) as Spot[],
    // Erases are made only by the backend, which sends them whole.
    erases: s?.erases ?? [],
    effects: {
      vignette: s?.effects?.vignette ?? 0,
      vignetteMidpoint: s?.effects?.vignetteMidpoint ?? 0.5,
      grain: s?.effects?.grain ?? 0,
      grainSize: s?.effects?.grainSize ?? 1,
      // A stack saved before these existed carries neither, so both fall back
      // to the same defaults the Rust side uses for a missing field.
      halation: s?.effects?.halation ?? 0,
      halationThreshold: s?.effects?.halationThreshold ?? 0.72,
      grainHighlightRolloff: s?.effects?.grainHighlightRolloff ?? 0.7,
    },
    lut: {
      name: s?.lut?.name ?? "",
      intensity: s?.lut?.intensity ?? 1,
    },
    // The built-in preset on the edit; a stack saved before presets had none.
    preset: {
      id: s?.preset?.id ?? "",
      amount: s?.preset?.amount ?? 1,
      tone: [0, 1, 2, 3].map((i) => ({
        added: s?.preset?.tone?.[i]?.added ?? 0,
        left: s?.preset?.tone?.[i]?.left ?? null,
      })) as Stack["preset"]["tone"],
    },
  };
}

/** A mask's local adjustments, all present. */
export type LocalAdj = {
  exposure: number;
  contrast: number;
  highlights: number;
  shadows: number;
  temperature: number;
  tint: number;
  saturation: number;
  whites: number;
  blacks: number;
  clarity: number;
  texture: number;
};

const NEUTRAL_LOCAL: LocalAdj = {
  exposure: 0,
  contrast: 0,
  highlights: 0,
  shadows: 0,
  temperature: 0,
  tint: 0,
  saturation: 0,
  whites: 0,
  blacks: 0,
  clarity: 0,
  texture: 0,
};

/** Every shape kind (incl. AI, which is model-generated, not user-drawn). */
export type ShapeKind = MaskShape["type"];
/** The kinds a user can create by hand — AI masks come from inference instead. */
export type CreatableKind = Exclude<ShapeKind, "ai">;

/** A fresh default shape of the given (user-creatable) kind, in framed-image UV. */
export function newShape(kind: CreatableKind): MaskShape {
  switch (kind) {
    case "radial":
      return { type: "radial", cx: 0.5, cy: 0.5, radiusX: 0.25, radiusY: 0.25, angle: 0, feather: 0.5 };
    case "linear":
      return { type: "linear", x1: 0.2, y1: 0.5, x2: 0.8, y2: 0.5 };
    case "brush":
      return { type: "brush", strokes: [] };
    case "luminance":
      return { type: "luminance", low: 0.5, high: 1, feather: 0.15 };
    case "colorRange":
      return { type: "colorRange", hue: 0.08, range: 0.1, feather: 0.08 };
  }
}

/** A component wrapping a fresh shape, combining by `mode` (default add). */
export function newComponent(kind: CreatableKind, mode: MaskMode = "add"): MaskComponent {
  return { shape: newShape(kind), inverted: false, mode };
}

/** A new single-component mask of the given shape kind, ready to edit. */
export function newMask(kind: CreatableKind): Mask {
  return { components: [newComponent(kind, "add")], adjustments: { ...NEUTRAL_LOCAL } };
}

/** The components of a mask, tolerating the optional array from the bindings. */
export function maskComponents(m: Mask): MaskComponent[] {
  return m.components ?? [];
}

/** A spot's fields with every value resolved (the bindings type them nullable). */
export type SpotVals = {
  targetX: number;
  targetY: number;
  sourceX: number;
  sourceY: number;
  radius: number;
  feather: number;
  heal: boolean;
};

/** Reads a spot with every field present. */
export function spotVals(s: Spot): SpotVals {
  return {
    targetX: s.targetX ?? 0.5,
    targetY: s.targetY ?? 0.5,
    sourceX: s.sourceX ?? 0.5,
    sourceY: s.sourceY ?? 0.5,
    radius: s.radius ?? 0.04,
    feather: s.feather ?? 0.3,
    heal: !!s.heal,
  };
}

/** A new heal spot: a target circle and a source offset a little to its left. */
export function newSpot(targetX: number, targetY: number): Spot {
  const sourceX = targetX > 0.2 ? targetX - 0.12 : targetX + 0.12;
  return { targetX, targetY, sourceX, sourceY: targetY, radius: 0.04, feather: 0.4, heal: true };
}

/** Reads a mask's adjustments with every field resolved. */
export function maskAdj(m: Mask): LocalAdj {
  const a = m.adjustments;
  return {
    exposure: a?.exposure ?? 0,
    contrast: a?.contrast ?? 0,
    highlights: a?.highlights ?? 0,
    shadows: a?.shadows ?? 0,
    temperature: a?.temperature ?? 0,
    tint: a?.tint ?? 0,
    saturation: a?.saturation ?? 0,
    whites: a?.whites ?? 0,
    blacks: a?.blacks ?? 0,
    clarity: a?.clarity ?? 0,
    texture: a?.texture ?? 0,
  };
}

const toGrade = (g?: { hue?: number | null; saturation?: number | null; luminance?: number | null } | null): Grade => ({
  hue: g?.hue ?? 0,
  saturation: g?.saturation ?? 0,
  luminance: g?.luminance ?? 0,
});

/** Fresh objects each time, for the reason the mixer arrays are rebuilt. */
const toGrading = (g?: EditStack["grading"]): Grading => ({
  shadows: toGrade(g?.shadows),
  midtones: toGrade(g?.midtones),
  highlights: toGrade(g?.highlights),
  blending: g?.blending ?? 0,
  balance: g?.balance ?? 0,
});

export const EMPTY_FILTER: PhotoFilter = {
  dateFrom: null,
  dateTo: null,
  genres: [],
  subjects: [],
  conditions: [],
  subjectsAll: false,
  excludeSubjects: [],
  excludeGenres: [],
  excludeConditions: [],
  placeIds: [],
  minRating: null,
  ratings: [],
  flags: [],
  colorLabels: [],
  collectionId: null,
  personId: null,
  personIds: [],
  cameras: [],
  lenses: [],
  focalRanges: [],
  isoRanges: [],
  apertureRanges: [],
  fileKinds: [],
  shapes: [],
  faceCounts: [],
  palette: [],
  editedOnly: false,
  copiesOnly: false,
  facesOnly: false,
  locatedOnly: false,
  unplacedOnly: false,
  untaggedOnly: false,
  folderIds: [],
  missingOnly: false,
  shootId: null,
  search: null,
  semantic: null,
  sort: "newest",
};

/**
 * The five colour labels, matching the `color_label` integers the backend
 * clamps to (0 = none). The conventional order and roughly the usual hues, so a
 * migrating user's mental model carries over. Shared by the grid overlay, the filter rail
 * and the selection bar so a label's colour is defined in exactly one place.
 */
export const COLOR_LABELS: { value: number; nameKey: Key; hex: string }[] = [
  { value: 1, nameKey: "store.types.colourRed", hex: "#e5484d" },
  { value: 2, nameKey: "store.types.colourYellow", hex: "#f5d90a" },
  { value: 3, nameKey: "store.types.colourGreen", hex: "#46a758" },
  { value: 4, nameKey: "store.types.colourBlue", hex: "#3e63dd" },
  { value: 5, nameKey: "store.types.colourPurple", hex: "#8e4ec6" },
];

/**
 * The range facets' options, by the key the backend filters on
 * (`db::FOCAL_RANGES`, `ISO_RANGES`, `APERTURE_RANGES` — keep them in step),
 * and how each reads in the rail.
 */
export const FOCAL_RANGES: { key: string; labelKey: Key }[] = [
  { key: "0-24", labelKey: "store.types.focalUpTo24" },
  { key: "25-50", labelKey: "store.types.focal25to50" },
  { key: "51-105", labelKey: "store.types.focal51to105" },
  { key: "106-200", labelKey: "store.types.focal106to200" },
  { key: "201-", labelKey: "store.types.focalOver200" },
];
export const ISO_RANGES: { key: string; labelKey: Key }[] = [
  { key: "0-400", labelKey: "store.types.isoUpTo400" },
  { key: "401-1600", labelKey: "store.types.iso401to1600" },
  { key: "1601-6400", labelKey: "store.types.iso1601to6400" },
  { key: "6401-", labelKey: "store.types.isoOver6400" },
];
export const APERTURE_RANGES: { key: string; labelKey: Key }[] = [
  { key: "0-2.8", labelKey: "store.types.apertureWider28" },
  { key: "2.9-5.6", labelKey: "store.types.aperture29to56" },
  { key: "5.7-", labelKey: "store.types.apertureNarrower56" },
];

/** The kind facets' options, by the key the backend filters on
 *  (`db::FILE_KINDS`, `SHAPES`, `FACE_COUNTS` — keep them in step). */
export const FILE_KINDS: { key: string; label: string }[] = [
  { key: "raw", label: "Raw" },
  { key: "jpeg", label: "JPEG" },
  { key: "heic", label: "HEIC / HEIF" },
  { key: "tiff", label: "TIFF" },
  { key: "png", label: "PNG" },
  { key: "webp", label: "WebP" },
];
export const SHAPES: { key: string; labelKey: Key }[] = [
  { key: "portrait", labelKey: "store.types.shapePortrait" },
  { key: "landscape", labelKey: "store.types.shapeLandscape" },
  { key: "square", labelKey: "store.types.shapeSquare" },
  { key: "panorama", labelKey: "store.types.shapePanorama" },
];
/** The named colours a photo can be covered by, in `palette::SWATCHES`
 *  order, each with how the rail draws it. */
export const PALETTE: { key: string; labelKey: Key; css: string }[] = [
  { key: "red", labelKey: "store.types.colourRed", css: "rgb(214 48 42)" },
  { key: "orange", labelKey: "store.types.colourOrange", css: "rgb(240 128 32)" },
  { key: "yellow", labelKey: "store.types.colourYellow", css: "rgb(238 206 48)" },
  { key: "green", labelKey: "store.types.colourGreen", css: "rgb(64 150 64)" },
  { key: "teal", labelKey: "store.types.colourTeal", css: "rgb(20 140 140)" },
  { key: "blue", labelKey: "store.types.colourBlue", css: "rgb(44 84 214)" },
  { key: "purple", labelKey: "store.types.colourPurple", css: "rgb(128 58 160)" },
  { key: "pink", labelKey: "store.types.colourPink", css: "rgb(240 128 176)" },
  { key: "brown", labelKey: "store.types.colourBrown", css: "rgb(128 82 44)" },
  { key: "white", labelKey: "store.types.colourWhite", css: "rgb(244 244 244)" },
  { key: "grey", labelKey: "store.types.colourGrey", css: "rgb(128 128 128)" },
  { key: "black", labelKey: "store.types.colourBlack", css: "rgb(16 16 18)" },
];

export const FACE_COUNTS: { key: string; labelKey: Key }[] = [
  { key: "0", labelKey: "store.types.faces0" },
  { key: "1", labelKey: "store.types.faces1" },
  { key: "2", labelKey: "store.types.faces2" },
  { key: "3-5", labelKey: "store.types.faces3to5" },
  { key: "6-", labelKey: "store.types.faces6" },
];

/** How the grid can be ordered, in the order the menu offers them. */
export const SORTS: { value: PhotoSort; labelKey: Key }[] = [
  { value: "newest", labelKey: "store.types.sortNewest" },
  { value: "oldest", labelKey: "store.types.sortOldest" },
  { value: "added", labelKey: "store.types.sortAdded" },
  { value: "rating", labelKey: "store.types.sortRating" },
  { value: "name", labelKey: "store.types.sortName" },
  { value: "sharpest", labelKey: "store.types.sortSharpest" },
  // Offered only while a collection is shown.
  { value: "custom", labelKey: "store.types.sortCustom" },
];

/** Whether an order follows capture time, so date headings mean something over it. */
export function sortIsByDate(sort: PhotoSort | undefined): boolean {
  return sort === undefined || sort === "newest" || sort === "oldest";
}

/**
 * A stored filter in today's shape. Filters saved before exact ratings carry
 * `minRating: n`, meaning n stars and up; that becomes the same set of exact
 * ratings, so the rail can show and change it. One saved with a single
 * `personId` becomes the same person in `personIds`, which the rail shows.
 * Missing fields are filled from `EMPTY_FILTER`.
 */
export function normaliseFilter(saved: Partial<PhotoFilter> | null | undefined): PhotoFilter {
  const loaded = { ...EMPTY_FILTER, ...(saved ?? {}) };
  const legacy = loaded.personId ?? null;
  const f =
    legacy === null
      ? loaded
      : { ...loaded, personId: null, personIds: [...new Set([...(loaded.personIds ?? []), legacy])] };
  if (f.minRating !== null && f.minRating !== undefined) {
    const floor = Math.max(1, Math.min(5, f.minRating));
    const upward = [1, 2, 3, 4, 5].filter((n) => n >= floor);
    return { ...f, minRating: null, ratings: [...new Set([...(f.ratings ?? []), ...upward])].sort((a, b) => a - b) };
  }
  return { ...f, ratings: f.ratings ?? [] };
}

/**
 * True when no filter is narrowing the library.
 *
 * Every field that narrows the set must be listed here, because this predicate
 * is what the UI uses to decide whether to offer "Clear all" and "Save as view".
 * A field omitted here becomes a filter the user can neither see nor clear —
 * which is exactly what `personId` was: People jumps into Browse with it set,
 * and the whole clear/save affordance stayed hidden.
 *
 * `semantic` counts too: a content search narrows the library like any facet,
 * and the whole point of routing it through the filter is that it can be saved
 * as a smart collection.
 */
export function isFilterEmpty(f: PhotoFilter): boolean {
  return (
    !f.dateFrom &&
    !f.dateTo &&
    f.genres.length === 0 &&
    f.subjects.length === 0 &&
    f.conditions.length === 0 &&
    (f.excludeSubjects?.length ?? 0) === 0 &&
    (f.excludeGenres?.length ?? 0) === 0 &&
    (f.excludeConditions?.length ?? 0) === 0 &&
    f.placeIds.length === 0 &&
    f.minRating === null &&
    (f.ratings?.length ?? 0) === 0 &&
    f.flags.length === 0 &&
    f.colorLabels.length === 0 &&
    f.collectionId === null &&
    (f.personId ?? null) === null &&
    (f.personIds?.length ?? 0) === 0 &&
    (f.cameras?.length ?? 0) === 0 &&
    (f.lenses?.length ?? 0) === 0 &&
    (f.focalRanges?.length ?? 0) === 0 &&
    (f.isoRanges?.length ?? 0) === 0 &&
    (f.apertureRanges?.length ?? 0) === 0 &&
    (f.fileKinds?.length ?? 0) === 0 &&
    (f.shapes?.length ?? 0) === 0 &&
    (f.faceCounts?.length ?? 0) === 0 &&
    (f.palette?.length ?? 0) === 0 &&
    !f.editedOnly &&
    !f.copiesOnly &&
    !f.facesOnly &&
    !f.locatedOnly &&
    !f.unplacedOnly &&
    !f.untaggedOnly &&
    (f.folderIds?.length ?? 0) === 0 &&
    (f.shootId ?? null) === null &&
    !f.missingOnly &&
    !f.search &&
    !f.semantic
  );
}
