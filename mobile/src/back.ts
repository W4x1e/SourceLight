/**
 * Back steps. Whatever Back should close (a photo, a menu, a sheet, the
 * editor) adds a history entry as it opens, and closes when Back pops it.
 * Each entry carries its depth, so a screen can tell whether Back has gone
 * past its own entry, whoever added the ones above it: closing the sync
 * sheet beside a tablet's photo leaves the photo open, and Back on one tab
 * leaves the other tab's photo alone.
 */
type Step = { depth?: number } & Record<string, unknown>;

/** The depth of the entry showing: 0 before any step. */
export const depthNow = () => (history.state as Step | null)?.depth ?? 0;

/** Adds a step called `name`, and returns its depth. */
export function pushStep(name: string): number {
  const depth = depthNow() + 1;
  history.pushState({ [name]: true, depth }, "");
  return depth;
}

/** Puts `name` in place of the step showing, at the same depth. */
export function replaceStep(name: string) {
  history.replaceState({ [name]: true, depth: depthNow() }, "");
}

/** Whether Back has gone past the step at `depth` (null: no step). */
export const gonePast = (depth: number | null) => depth !== null && depthNow() < depth;

/** Goes back past the step at `depth` and any above it, in one go. */
export function backPast(depth: number) {
  const steps = depthNow() - depth + 1;
  if (steps > 0) history.go(-steps);
}
