import { api } from "./api";

/**
 * Interface errors, written beside the crash logs.
 *
 * A render failure an error boundary catches, and a script error or rejected
 * promise that nothing catches, all go to the same folder as a Rust panic — so
 * "it went wrong" arrives with a stack and the machine's details, from a folder
 * Settings can open. Nothing leaves the machine: these are files on the user's
 * disk, sent (or not) by the user.
 *
 * Best-effort throughout. Logging an error must never become a second error, so
 * every failure here is swallowed — including the backend being unavailable, as
 * it is in unit tests.
 */
export function reportUiError(surface: string, error: unknown, extraStack = ""): void {
  const err = error instanceof Error ? error : null;
  // A command's failure is logged in the English the core sent, not the
  // translation the user saw (see `CoreError`).
  const english = err && "english" in err && typeof err.english === "string" ? err.english : err?.message;
  const message = err ? `${err.name}: ${english}` : String(error);
  const stack = [err?.stack ?? "", extraStack]
    .filter((s) => s.trim() !== "")
    .join("\n\ncomponent stack:\n");
  try {
    void api.logUiError(surface, message, stack).catch(() => {});
  } catch {
    // The bridge itself was not there.
  }
}
