import { act, render } from "@testing-library/react";
import { useRef } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { isTextEntry, overlayOpen, useEscape, useModal } from "./useEscape";

const press = (key: string, init: KeyboardEventInit = {}) =>
  act(() => {
    (document.activeElement ?? document.body).dispatchEvent(
      new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init }),
    );
  });

function Panel({ onClose, label, scrim = false }: { onClose: () => void; label: string; scrim?: boolean }) {
  const card = useRef<HTMLDivElement>(null);
  useModal(true, card, onClose);
  const body = (
    <div ref={card} role="dialog" aria-modal="true" aria-label={label}>
      <button>{label} one</button>
      <button>{label} two</button>
    </div>
  );
  return scrim ? <div className="dialog-scrim">{body}</div> : body;
}

function Menu({ onClose }: { onClose: () => void }) {
  useEscape(true, onClose);
  return null;
}

/**
 * Overlays own their keys: Escape closes the innermost only and goes
 * no further, Tab stays inside a modal, and focus goes in and comes back.
 */
describe("overlay keys", () => {
  afterEach(() => document.body.replaceChildren());

  it("closes only the innermost overlay, and the workspace never hears it", () => {
    const workspace = vi.fn();
    window.addEventListener("keydown", workspace);
    const [outer, inner] = [vi.fn(), vi.fn()];
    const view = render(
      <>
        <Panel label="Export" onClose={outer} />
        <Menu onClose={inner} />
      </>,
    );
    press("Escape");
    expect(inner).toHaveBeenCalledOnce();
    expect(outer).not.toHaveBeenCalled();
    expect(workspace).not.toHaveBeenCalled();
    view.unmount();
    window.removeEventListener("keydown", workspace);
  });

  it("answers Escape inside its own scrim, as the copy-settings dialog draws one", () => {
    const close = vi.fn();
    const view = render(<Panel label="Copy" onClose={close} scrim />);
    press("Escape");
    expect(close).toHaveBeenCalledOnce();
    view.unmount();
  });

  it("takes focus, keeps Tab inside, and gives focus back on close", async () => {
    const opener = document.createElement("button");
    document.body.append(opener);
    opener.focus();
    const view = render(<Panel label="Settings" onClose={() => {}} />);
    await act(() => new Promise((r) => requestAnimationFrame(() => r(null))));
    const [one, two] = Array.from(document.querySelectorAll("button")).filter((b) => b !== opener);
    expect(document.activeElement).toBe(one);
    expect(overlayOpen()).toBe(true);

    two!.focus();
    press("Tab");
    expect(document.activeElement).toBe(one);
    press("Tab", { shiftKey: true });
    expect(document.activeElement).toBe(two);

    view.unmount();
    expect(document.activeElement).toBe(opener);
  });

  it("treats a slider as a control, not a place to type", () => {
    const input = (type: string) => Object.assign(document.createElement("input"), { type });
    expect(isTextEntry(input("range"))).toBe(false);
    expect(isTextEntry(input("checkbox"))).toBe(false);
    expect(isTextEntry(input("text"))).toBe(true);
    expect(isTextEntry(input("number"))).toBe(true);
    expect(isTextEntry(document.createElement("textarea"))).toBe(true);
    expect(isTextEntry(document.createElement("button"))).toBe(false);
  });
});
