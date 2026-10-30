import React from "react";
import { reportUiError } from "../lib/errorLog";
import { toast } from "../lib/dialogs";
import { t } from "../i18n";

/**
 * Catches a render-time throw instead of letting it blank the window.
 *
 * The app already reports *asynchronous* failures well — every store action
 * sets `error`, and the banner in `App` shows it. None of that helps here. A
 * component that throws while rendering unmounts the whole React tree, and
 * because this is a Tauri webview with no address bar and no reload button,
 * what the user gets is a black rectangle and no way out.
 *
 * So this deliberately does two things beyond catching:
 *
 * **It offers a way back that is not "restart the app".** Most render throws
 * come from one bad photo, one malformed edit stack, one unexpected shape from
 * a command — the surface is broken, the library is not. Remounting the subtree
 * is usually enough, and it costs nothing to try before reaching for a reload.
 *
 * **It puts the details somewhere a person can send them.** There is no crash
 * reporting and no telemetry (by design — nothing leaves the disk), so the only
 * route from a user's crash to a fix is them telling us. That is only realistic
 * if copying the stack takes one click.
 */

interface Props {
  children: React.ReactNode;
  /** Names the surface in the message, e.g. "Develop". */
  label?: string;
  /**
   * Class for the fallback container.
   *
   * So the fallback takes the place of the surface it replaces in the
   * stylesheet. In the healthy case this boundary renders a Fragment and adds
   * no DOM node at all, so Develop stays a direct child of `.app-shell`; the
   * fallback *is* a div, and as `develop-shell` it is treated as Develop by the
   * direct-canvas rules rather than as the Library they hide.
   */
  fallbackClassName?: string;
  /** Closes the overlay this guards and goes back to the library:
   *  Retry throws again on a fault that repeats, and Reload restarts the
   *  whole interface. */
  onLeave?: () => void;
}

interface State {
  error: Error | null;
  info: React.ErrorInfo | null;
  /** Bumped on retry, to force a fresh subtree rather than a reused one. */
  attempt: number;
}

export default class ErrorBoundary extends React.Component<Props, State> {
  state: State = { error: null, info: null, attempt: 0 };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    // Kept for the copy button, and written to the log folder beside the crash
    // logs — the webview devtools are otherwise the only place it exists.
    this.setState({ info });
    console.error("render failed", error, info.componentStack);
    reportUiError(this.props.label ?? "the app", error, info.componentStack ?? "");
  }

  private retry = () => {
    this.setState((s) => ({ error: null, info: null, attempt: s.attempt + 1 }));
  };

  private report = () => {
    const { error, info } = this.state;
    const text = [
      `SourceLight — render error${this.props.label ? ` in ${this.props.label}` : ""}`,
      `When: ${new Date().toISOString()}`,
      `Where: ${window.location.href}`,
      "",
      String(error?.stack ?? error),
      "",
      "Component stack:",
      (info?.componentStack ?? "").trim(),
    ].join("\n");
    // Said either way: the click used to do nothing visible.
    void (navigator.clipboard?.writeText(text) ?? Promise.reject(new Error("no clipboard")))
      .then(() => toast(t("shell.error.copied"), "success"))
      .catch(() => toast(t("shell.error.copyFailed"), "error"));
  };

  render() {
    const { error } = this.state;
    if (!error) {
      // The key is what makes retry a genuine remount: without it React reuses
      // the existing instances, including whatever state caused the throw.
      return <React.Fragment key={this.state.attempt}>{this.props.children}</React.Fragment>;
    }

    return (
      <div
        className={`flex min-h-0 flex-1 items-center justify-center bg-base p-8 ${this.props.fallbackClassName ?? ""}`}
      >
        <div className="max-w-lg rounded-lg border border-line bg-panel p-6">
          <h2 className="text-title text-ink">
            {this.props.label ? t("shell.error.stopped", { label: this.props.label }) : t("shell.error.generic")}
          </h2>
          <p className="mt-2 text-caption text-muted">
            {t("shell.error.detail")}
          </p>
          <p className="mt-2 font-mono text-label text-faint">{String(error.message || error)}</p>

          <div className="mt-4 flex gap-2">
            <button
              onClick={this.retry}
              className="rounded-md border border-line bg-raised px-3 py-1.5 text-caption text-ink hover:bg-hover"
            >
              {t("common.retry")}
            </button>
            {this.props.onLeave && (
              <button
                onClick={this.props.onLeave}
                className="rounded-md border border-line bg-raised px-3 py-1.5 text-caption text-ink hover:bg-hover"
              >
                {t("shell.error.backToLibrary")}
              </button>
            )}
            <button
              onClick={this.report}
              className="rounded-md border border-line px-3 py-1.5 text-caption text-muted hover:text-ink"
            >
              {t("shell.error.copyDetails")}
            </button>
            <button
              onClick={() => window.location.reload()}
              className="rounded-md border border-line px-3 py-1.5 text-caption text-muted hover:text-ink"
            >
              {t("shell.error.reload")}
            </button>
          </div>
        </div>
      </div>
    );
  }
}
