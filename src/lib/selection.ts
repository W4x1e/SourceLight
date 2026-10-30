/**
 * Reading one value across a multi-photo selection.
 *
 * The selection panel's stars, flags and colour swatches write to every selected
 * photo but used to *display* the focused photo's value, which made them lie and
 * then act on the lie: with twelve photos selected at mixed ratings, the stars
 * showed the focused photo's 3, and clicking that same 3 toggled it off — wiping
 * the rating on all twelve, including the nine that were never 3.
 *
 * The rule these two functions encode: toggle off only when the whole selection
 * demonstrably carries the value already. Otherwise a click assigns.
 */

export type Uniformity<T> =
  /** Nothing selected. */
  | { kind: "empty" }
  /** Every selected photo carries the same value. */
  | { kind: "uniform"; value: T }
  /** The selection disagrees. */
  | { kind: "mixed" };

/** Collapses a selection's values into one verdict. */
export function uniformity<T>(values: readonly T[]): Uniformity<T> {
  const first = values[0];
  if (values.length === 0 || first === undefined) return { kind: "empty" };
  return values.every((v) => v === first) ? { kind: "uniform", value: first } : { kind: "mixed" };
}

/**
 * What clicking `clicked` should write, given what the selection currently
 * holds. Clears only when the selection is uniformly already at that value —
 * a mixed or empty selection assigns instead, so a click can never silently
 * clear photos the user could not see the state of.
 */
export function toggleTarget(
  state: Uniformity<number>,
  clicked: number,
  cleared = 0,
): number {
  return state.kind === "uniform" && state.value === clicked ? cleared : clicked;
}
