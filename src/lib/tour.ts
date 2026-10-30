import type { AppMode } from "../store";
import type { Key } from "../i18n";
import type { ActionId } from "./keymap";

/**
 * The guided tour: a spotlight on one real control at a time, with a card
 * beside it saying what it is for. It runs once after the first folder is
 * added, and again from Settings › About.
 *
 * Each step names its target by a `data-tour` attribute on the control, so the
 * tour points at the live UI rather than at a picture of it. A step with a
 * `mode` switches workspace first. A step whose target is missing (or has no
 * size) shows its card in the middle of the window instead.
 */

/** What a placeholder in a body shows as a key. An action id is read from the live
 *  keymap when shown, so a rebound key is never described wrongly; `keys` is
 *  literal keycap text; `label` is a named key that has a translation. */
export type TourKey = { key: ActionId } | { keys: string } | { label: Key };

export interface TourStep {
  id: string;
  /** The `data-tour` value to spotlight, or null for a centred card. */
  target: string | null;
  mode?: AppMode;
  /** Both are catalogue keys, shown through `t` where the card renders. */
  title: Key;
  body: Key;
  /** The keys a body's `{placeholders}` stand for, each drawn as a keycap. */
  keys?: Record<string, TourKey>;
}

export const TOUR: TourStep[] = [
  { id: "welcome", target: null, title: "shell.tour.welcome.title", body: "shell.tour.welcome.body" },
  { id: "browse", target: "rail-browse", mode: "browse", title: "shell.tour.browse.title", body: "shell.tour.browse.body" },
  {
    id: "search",
    target: "search",
    mode: "browse",
    title: "shell.tour.search.title",
    body: "shell.tour.search.body",
    keys: { enter: { label: "shell.keymap.key.enter" } },
  },
  {
    id: "filters",
    target: "filter-rail",
    mode: "browse",
    title: "shell.tour.filters.title",
    body: "shell.tour.filters.body",
    keys: { toggle: { key: "toggle-filters" } },
  },
  { id: "library", target: "library-menu", mode: "browse", title: "shell.tour.library.title", body: "shell.tour.library.body" },
  {
    id: "cull",
    target: "cull-verdict",
    mode: "cull",
    title: "shell.tour.cull.title",
    body: "shell.tour.cull.body",
    keys: {
      pick: { key: "flag-pick" },
      reject: { key: "flag-reject" },
      rate: { keys: "1–5" },
      next: { key: "next-undecided" },
    },
  },
  // The AI culling the app is built around, and where finished photos go:
  // the tour taught Cull as keys alone, and never said where export lives.
  { id: "suggest", target: "cull-suggest", mode: "cull", title: "shell.tour.suggest.title", body: "shell.tour.suggest.body" },
  { id: "export", target: "export", mode: "browse", title: "shell.tour.export.title", body: "shell.tour.export.body" },
  { id: "places", target: "rail-places", mode: "places", title: "shell.tour.places.title", body: "shell.tour.places.body" },
  { id: "people", target: "rail-people", mode: "people", title: "shell.tour.people.title", body: "shell.tour.people.body" },
  {
    id: "develop",
    target: "rail-develop",
    mode: "browse",
    title: "shell.tour.develop.title",
    body: "shell.tour.develop.body",
    keys: { develop: { key: "open-develop" } },
  },
  { id: "activity", target: "rail-activity", title: "shell.tour.activity.title", body: "shell.tour.activity.body" },
  { id: "settings", target: "rail-settings", title: "shell.tour.settings.title", body: "shell.tour.settings.body" },
];

export interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** Space between the spotlight and the card, and between the card and the window edge. */
export const CARD_GAP = 12;
export const EDGE = 16;

/**
 * Where the card goes beside `target`: right, then below, then above, then
 * left — the first side it fits on — and clamped inside the window. With no
 * side that fits (a target filling the window), it sits centred over it.
 */
export function placeCard(
  target: Box | null,
  card: { width: number; height: number },
  viewport: { width: number; height: number },
): { left: number; top: number } {
  const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(v, Math.max(lo, hi)));
  const clampX = (x: number) => clamp(x, EDGE, viewport.width - card.width - EDGE);
  const clampY = (y: number) => clamp(y, EDGE, viewport.height - card.height - EDGE);

  if (!target) {
    return { left: clampX((viewport.width - card.width) / 2), top: clampY((viewport.height - card.height) / 2) };
  }

  const right = target.left + target.width + CARD_GAP;
  const below = target.top + target.height + CARD_GAP;
  const above = target.top - CARD_GAP - card.height;
  const left = target.left - CARD_GAP - card.width;
  const midX = target.left + target.width / 2 - card.width / 2;
  const midY = target.top + target.height / 2 - card.height / 2;

  if (right + card.width <= viewport.width - EDGE) return { left: right, top: clampY(midY) };
  if (below + card.height <= viewport.height - EDGE) return { left: clampX(midX), top: below };
  if (above >= EDGE) return { left: clampX(midX), top: above };
  if (left >= EDGE) return { left, top: clampY(midY) };
  return { left: clampX(midX), top: clampY(midY) };
}
