import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { create } from "zustand";
import { Icon, type IconName } from "./icons";
import { ask } from "@tauri-apps/plugin-dialog";
import { IS_MAC } from "./platform";
import { t } from "../i18n";
import { useHeld, useLeavingList, usePresence } from "./usePresence";

/**
 * In-app dialogs and toasts, replacing the webview's native
 * `window.alert / confirm / prompt`.
 *
 * Native browser dialogs break a desktop tool's crafted feel and render
 * inconsistently in the webview. These are promise-based so a call site reads
 * exactly like the native it replaces —
 *
 *   if (await confirm({ title: "Delete collection?" })) …
 *   const name = await prompt({ title: "New collection" }); if (name) …
 *
 * — while looking like the rest of the app. A single {@link DialogHost} mounted
 * once at the app root renders whatever is pending.
 */

type DialogKind = "confirm" | "prompt";

interface DialogState {
  kind: DialogKind;
  title: string;
  message?: string;
  placeholder?: string;
  defaultValue?: string;
  /** An empty answer is an answer (see `PromptOptions.allowEmpty`). */
  allowEmpty?: boolean;
  confirmLabel: string;
  cancelLabel: string;
  danger: boolean;
  // Resolves the awaiting caller: boolean for confirm, string|null for prompt.
  resolve: (value: boolean | string | null) => void;
}

export type ToastKind = "info" | "success" | "error";
interface ToastItem {
  id: number;
  message: string;
  kind: ToastKind;
  action?: ToastAction;
}

/** A button on a toast, such as Undo. */
export interface ToastAction {
  label: string;
  run: () => void;
}

interface DialogsStore {
  dialog: DialogState | null;
  toasts: ToastItem[];
  open: (d: DialogState) => void;
  settle: (value: boolean | string | null) => void;
  pushToast: (message: string, kind: ToastKind, action?: ToastAction) => void;
  dismissToast: (id: number) => void;
  /** Keeps a toast up while the pointer or focus is on it, so a slow
   *  reader isn't racing the clock; `releaseToast` starts it again. */
  holdToast: (id: number) => void;
  releaseToast: (id: number) => void;
}

let nextToastId = 1;
const toastTimers = new Map<number, number>();
// Errors linger a little longer since they carry detail, and one with an
// action longer still, to reach it.
const toastLife = (t: ToastItem) => (t.action ? 8000 : t.kind === "error" ? 6000 : 3200);

const useDialogs = create<DialogsStore>((set, get) => ({
  dialog: null,
  toasts: [],
  // Only one dialog is on screen at a time, so opening a second has to settle
  // the first — otherwise its `resolve` is dropped on the floor and whoever
  // `await`ed it waits forever. Cancel is the safe answer for a question the
  // user never actually saw.
  open: (d) => {
    const pending = get().dialog;
    if (pending) pending.resolve(pending.kind === "confirm" ? false : null);
    set({ dialog: d });
  },
  settle: (value) => {
    const d = get().dialog;
    if (!d) return;
    set({ dialog: null });
    d.resolve(value);
  },
  pushToast: (message, kind, action) => {
    const id = nextToastId++;
    set((s) => ({ toasts: [...s.toasts, { id, message, kind, action }] }));
    get().releaseToast(id);
  },
  dismissToast: (id) => {
    window.clearTimeout(toastTimers.get(id));
    toastTimers.delete(id);
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
  },
  holdToast: (id) => {
    window.clearTimeout(toastTimers.get(id));
    toastTimers.delete(id);
  },
  releaseToast: (id) => {
    const toast = get().toasts.find((t) => t.id === id);
    if (!toast) return;
    window.clearTimeout(toastTimers.get(id));
    toastTimers.set(id, window.setTimeout(() => get().dismissToast(id), toastLife(toast)));
  },
}));

export interface ConfirmOptions {
  title: string;
  message?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** Styles the confirm button as destructive (a red key). */
  danger?: boolean;
}

/**
 * A Mac asks with the system's own alert — bold question, explanation under
 * it, the buttons named for what they do, a caution badge when it destroys
 * something — which is what a Mac user reads as a real question rather than
 * a web page's pop-up. Windows keeps the drawn dialog.
 */
const NATIVE_ALERTS = IS_MAC && typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Ask the user to confirm. Resolves true on confirm, false on cancel/Escape. */
export function confirm(opts: ConfirmOptions): Promise<boolean> {
  if (NATIVE_ALERTS) {
    return ask(opts.message ?? "", {
      title: opts.title,
      kind: opts.danger ? "warning" : "info",
      okLabel: opts.confirmLabel ?? t("shell.dialog.confirm"),
      cancelLabel: opts.cancelLabel ?? t("common.cancel"),
    }).catch(() => false);
  }
  return new Promise<boolean>((resolve) => {
    useDialogs.getState().open({
      kind: "confirm",
      title: opts.title,
      message: opts.message,
      confirmLabel: opts.confirmLabel ?? t("shell.dialog.confirm"),
      cancelLabel: opts.cancelLabel ?? t("common.cancel"),
      danger: opts.danger ?? false,
      resolve: (v) => resolve(v === true),
    });
  });
}

export interface PromptOptions {
  title: string;
  message?: string;
  placeholder?: string;
  defaultValue?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** OK with the box empty resolves "" rather than being disabled, so that
   *  null means only Cancel or Esc. */
  allowEmpty?: boolean;
}

/** Ask for a line of text. Resolves the trimmed value, or null on cancel —
 *  and on an empty box unless `allowEmpty`, when that is "". */
export function prompt(opts: PromptOptions): Promise<string | null> {
  return new Promise<string | null>((resolve) => {
    useDialogs.getState().open({
      kind: "prompt",
      title: opts.title,
      message: opts.message,
      placeholder: opts.placeholder,
      defaultValue: opts.defaultValue,
      allowEmpty: opts.allowEmpty,
      confirmLabel: opts.confirmLabel ?? t("common.ok"),
      cancelLabel: opts.cancelLabel ?? t("common.cancel"),
      danger: false,
      resolve: (v) =>
        resolve(typeof v === "string" ? v.trim() || (opts.allowEmpty ? "" : null) : null),
    });
  });
}

/** Show a transient confirmation. Replaces the old `window.alert` for results. */
export function toast(message: string, kind: ToastKind = "info", action?: ToastAction): void {
  useDialogs.getState().pushToast(message, kind, action);
}

const TOAST_ICON: Record<ToastKind, IconName> = {
  info: "info",
  success: "check",
  error: "warn",
};
const TOAST_COLOR: Record<ToastKind, string> = {
  info: "text-accent",
  success: "text-positive",
  error: "text-danger",
};

/** Room the toast stack keeps from the window's edge (`bottom-4`/`top-4`). */
const STACK_INSET = 16;

/**
 * Whether the toasts should go to the top of the window: when a dialog card
 * reaches down into the room they take at the bottom. At the minimum window a
 * tall dialog (Settings, Import, Export) ends 32px above the edge with its
 * buttons there, and a toast — often answering a button in it — sat on them.
 * A dialog's top is its title, which a toast can cover for a moment without
 * hiding anything to press. Full-window views (the Lightbox, Develop) are not
 * cards and change nothing.
 */
export function toastsAtTop(cards: readonly Pick<DOMRect, "top" | "bottom">[], windowH: number, stackH: number): boolean {
  const room = windowH - STACK_INSET - stackH - 8;
  return cards.some((r) => r.top > 0 && r.bottom < windowH && r.bottom > room);
}

/** Mount once at the app root: renders the pending dialog and the toast stack. */
export function DialogHost() {
  const dialog = useDialogs((s) => s.dialog);
  // The card fades out after it is answered, showing what it asked.
  const shown = useHeld(dialog);
  const presence = usePresence(dialog !== null);
  const settle = useDialogs((s) => s.settle);
  const toasts = useDialogs((s) => s.toasts);
  const dismissToast = useDialogs((s) => s.dismissToast);
  const holdToast = useDialogs((s) => s.holdToast);
  const releaseToast = useDialogs((s) => s.releaseToast);

  const [value, setValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const cardRef = useRef<HTMLDivElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);

  // Seed and focus the prompt field each time a dialog opens.
  useEffect(() => {
    if (dialog?.kind === "prompt") {
      setValue(dialog.defaultValue ?? "");
      // Focus after paint so the select lands on the seeded text. Focus is
      // asked for explicitly rather than leaned on as a side effect of
      // `select()`, which is not guaranteed to move it.
      requestAnimationFrame(() => {
        inputRef.current?.focus();
        inputRef.current?.select();
      });
    } else if (dialog?.kind === "confirm") {
      // A confirm has no field, so focus used to stay on whatever opened the
      // dialog — behind the scrim, with `aria-modal` claiming otherwise.
      requestAnimationFrame(() => confirmRef.current?.focus());
    }
  }, [dialog]);

  // Put focus back where it came from once the dialog closes, so dismissing one
  // doesn't dump the user at the top of the page.
  useEffect(() => {
    if (!dialog) return;
    const opener = document.activeElement as HTMLElement | null;
    return () => opener?.focus?.();
  }, [dialog]);

  /**
   * Keep Tab inside the card. `aria-modal` tells assistive tech that the rest
   * of the page is inert; without a trap, Tab walked straight out into it.
   */
  const onTrapKey = (e: React.KeyboardEvent) => {
    if (e.key !== "Tab") return;
    const root = cardRef.current;
    if (!root) return;
    const focusable = Array.from(
      root.querySelectorAll<HTMLElement>('button, input, select, textarea, [href], [tabindex]:not([tabindex="-1"])'),
    ).filter((el) => !el.hasAttribute("disabled"));
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (!first || !last) return;
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  // Escape cancels, Enter confirms — the native dialog's keys. Except that
  // Enter on a focused button presses that button: Tab to Cancel and Enter
  // used to confirm, even a delete.
  useEffect(() => {
    if (!dialog) return;
    // Only keys pressed after it opened. A dialog opened by an Enter — a name
    // committed with Enter that turns out to be taken — can mount and add
    // this listener while that same keypress is still bubbling to the window,
    // which answered the question before it was on screen.
    const openedAt = performance.now();
    const onKey = (e: KeyboardEvent) => {
      if (e.timeStamp < openedAt) return;
      if (e.key === "Escape") {
        e.preventDefault();
        settle(dialog.kind === "confirm" ? false : null);
      } else if (e.key === "Enter" && dialog.kind === "confirm") {
        e.preventDefault();
        const focused = document.activeElement;
        if (focused instanceof HTMLButtonElement && cardRef.current?.contains(focused)) {
          // Pressed here rather than left to the browser, which activates a
          // button on Enter at keydown: one path, whichever answers first.
          focused.click();
        } else {
          settle(true);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [dialog, settle]);

  // Out of a tall dialog's footer (see `toastsAtTop`), measured as each toast
  // comes and goes.
  const stackRef = useRef<HTMLDivElement>(null);
  const [atTop, setAtTop] = useState(false);
  useLayoutEffect(() => {
    if (toasts.length === 0) return;
    const measure = () => {
      const cards = [...document.querySelectorAll<HTMLElement>('[role="dialog"]:not([data-closing]):not([data-closing] *)')].map((el) =>
        el.getBoundingClientRect(),
      );
      setAtTop(toastsAtTop(cards, window.innerHeight, stackRef.current?.offsetHeight ?? 0));
    };
    measure();
    // Again whenever a dialog comes or goes while a toast is up (Import opened
    // over a toast already showing sat on Import's buttons), and on resize.
    // Coalesced to one measurement a frame: the grid adds and drops nodes as
    // it scrolls, and each measurement reads layout.
    let frame = 0;
    const soon = () => {
      if (!frame) frame = requestAnimationFrame(() => ((frame = 0), measure()));
    };
    const observer = new MutationObserver(soon);
    observer.observe(document.body, { childList: true, subtree: true });
    window.addEventListener("resize", soon);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", soon);
      cancelAnimationFrame(frame);
    };
  }, [toasts, dialog]);

  // Toasts rise in, and fade where they stood when they go.
  const toastItems = useLeavingList(toasts, (t) => t.id);

  const accept = () => settle(dialog?.kind === "prompt" ? value : true);
  const cancel = () => settle(dialog?.kind === "prompt" ? null : false);

  return (
    <>
      {presence.mounted && shown && (
        <div
          className="dialog-scrim anim-scrim"
          data-closing={presence.closing || undefined}
          onClick={(e) => e.target === e.currentTarget && cancel()}>
          <div
            ref={cardRef}
            onKeyDown={onTrapKey}
            className="dialog-card anim-card"
            role="dialog"
            aria-modal="true"
            aria-labelledby="dialog-title"
            aria-describedby={shown.message ? "dialog-message" : undefined}
          >
            <div className="px-4 py-3">
              <h2
                id="dialog-title"
                className="font-display text-display text-ink"
              >
                {shown.title}
              </h2>
              {shown.message && (
                <p id="dialog-message" className="mt-1 text-caption leading-relaxed text-muted">
                  {shown.message}
                </p>
              )}
              {shown.kind === "prompt" && (
                <input
                  ref={inputRef}
                  className="input mt-3"
                  placeholder={shown.placeholder}
                  value={value}
                  onChange={(e) => setValue(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      accept();
                    }
                  }}
                />
              )}
            </div>
            <div className="flex items-center justify-end gap-2 border-t border-line px-4 py-2.5">
              <button onClick={cancel} className="btn-ghost text-muted">
                {shown.cancelLabel}
              </button>
              {/* Destructive confirm uses DESIGN.md's Fault Red triad (a /40
                  border, /10 fill, red text) rather than a solid-red key, which
                  the system does not have. */}
              <button
                ref={confirmRef}
                onClick={accept}
                disabled={shown.kind === "prompt" && !shown.allowEmpty && !value.trim()}
                className={shown.danger ? "btn-solid btn-danger" : "btn-accent"}
              >
                {shown.confirmLabel}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Both regions always mounted: a live region that arrives
          already holding its message often goes unread, and most toasts are
          the only one on screen. Errors interrupt; the rest wait their turn. */}
      <div
        ref={stackRef}
        className={`pointer-events-none fixed ${atTop ? "top-4" : "bottom-4"} left-1/2 z-[70] flex -translate-x-1/2 flex-col items-center gap-2`}
      >
        {(["alert", "status"] as const).map((region) => (
          <div
            key={region}
            role={region}
            aria-live={region === "alert" ? "assertive" : "polite"}
            className="flex flex-col items-center gap-2"
          >
          {toastItems.filter(({ item: t }) => (t.kind === "error") === (region === "alert")).map(({ item: t, closing }) => (
            <div
              key={t.id}
              data-closing={closing || undefined}
              aria-hidden={closing || undefined}
              onClick={() => dismissToast(t.id)}
              onPointerEnter={() => holdToast(t.id)}
              onPointerLeave={() => releaseToast(t.id)}
              onFocus={() => holdToast(t.id)}
              onBlur={() => releaseToast(t.id)}
              title={t.message}
              className={`toast toast-in pointer-events-auto cursor-default !items-start ${atTop ? "toast-from-top" : ""}`}
            >
              <Icon
                name={TOAST_ICON[t.kind]}
                size={15}
                className={`mt-px shrink-0 ${TOAST_COLOR[t.kind]}`}
              />
              {/* Wrap rather than truncate: raw backend errors come through here
                  (`toast(String(e), "error")`), and a clipped one is unreadable.
                  Clamped so a stack-trace-length message can't fill the screen,
                  with the full text on the title for the rest. */}
              <span className="line-clamp-6 max-w-[420px] whitespace-pre-wrap break-words text-left">
                {t.message}
              </span>
              {t.action && (
                <button
                  className="btn-ghost -my-1 shrink-0 px-2 py-0.5 font-semibold"
                  onClick={(e) => {
                    e.stopPropagation();
                    dismissToast(t.id);
                    t.action!.run();
                  }}
                >
                  {t.action.label}
                </button>
              )}
            </div>
          ))}
          </div>
        ))}
      </div>
    </>
  );
}
