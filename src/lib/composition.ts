import { t, type Key } from "../i18n";
/**
 * Viewing aids for judging a frame: composition grids, and focus
 * peaking's settings. The drawing lives in `CompositionGrid` and
 * `FocusPeaking`; what they draw is decided here, where it can be tested.
 */

/** The guides a frame can be shown with. */
export type GridKind = "off" | "thirds" | "golden" | "centre";

export const GRID_KINDS: { id: GridKind; label: Key }[] = [
  { id: "off", label: "viewer.aids.gridOff" },
  { id: "thirds", label: "viewer.aids.gridThirds" },
  { id: "golden", label: "viewer.aids.gridGolden" },
  { id: "centre", label: "viewer.aids.gridCentre" },
];

/** φ's reciprocal: the golden section sits at 0.382 and 0.618 of each side. */
const GOLDEN = (Math.sqrt(5) - 1) / 2;

/** Where a grid's lines fall along each side, as fractions of it. */
export function gridFractions(kind: GridKind): number[] {
  switch (kind) {
    case "thirds":
      return [1 / 3, 2 / 3];
    case "golden":
      return [1 - GOLDEN, GOLDEN];
    case "centre":
      return [0.5];
    default:
      return [];
  }
}

/** The grid after this one, round to "off" and back. */
export function nextGrid(kind: GridKind): GridKind {
  const i = GRID_KINDS.findIndex((g) => g.id === kind);
  return GRID_KINDS[(i + 1) % GRID_KINDS.length]!.id;
}

export function gridLabel(kind: GridKind): string {
  return t(GRID_KINDS.find((g) => g.id === kind)?.label ?? "viewer.aids.gridOff");
}

/** The colours peaking can mark sharp detail in. */
export type PeakColor = "red" | "green" | "yellow" | "white";

/** Yellow first, and the default: it reads against skin, sky and foliage alike. */
export const PEAK_COLORS: { id: PeakColor; label: Key; rgb: [number, number, number] }[] = [
  { id: "yellow", label: "store.types.colourYellow", rgb: [1, 0.9, 0.1] },
  { id: "red", label: "store.types.colourRed", rgb: [1, 0.15, 0.15] },
  { id: "green", label: "store.types.colourGreen", rgb: [0.2, 1, 0.3] },
  { id: "white", label: "store.types.colourWhite", rgb: [1, 1, 1] },
];

export function peakRgb(color: PeakColor): [number, number, number] {
  return (PEAK_COLORS.find((c) => c.id === color) ?? PEAK_COLORS[0]!).rgb;
}

/** Sensitivity runs 0–100; the default marks what a camera's peaking would. */
export const DEFAULT_SENSITIVITY = 50;

/**
 * What a pixel's detail must reach to be marked, from the sensitivity (0–100).
 *
 * Detail is how far a pixel stands from its eight neighbours, as a share of
 * the contrast around it (the range over 5 × 5 pixels). That share is what
 * tells sharp from soft whatever the contrast: a crisp step puts about 3/8 of
 * it into one pixel, the same step a pixel or so soft spreads it out and puts
 * well under a fifth. An absolute measure can't: a soft edge of high contrast
 * outscores a sharp one of low. `floor` is the least contrast that counts at
 * all, so noise in flat sky and skin isn't marked.
 */
export function peakThreshold(sensitivity: number): { share: number; floor: number } {
  const s = Math.min(100, Math.max(0, sensitivity)) / 100;
  return { share: 0.26 - 0.16 * s, floor: 0.1 - 0.08 * s };
}

/**
 * The size to measure a source at: its own, unless that is larger than the
 * screen could ever need or a GPU texture may be. A full-resolution raw at
 * 1:1 is where focus is actually judged, so this keeps a 24–45 MP frame whole.
 */
export function peakSize(width: number, height: number, maxEdge = 8192): { width: number; height: number } {
  const long = Math.max(width, height);
  if (long <= maxEdge || long === 0) return { width, height };
  const k = maxEdge / long;
  return { width: Math.round(width * k), height: Math.round(height * k) };
}
