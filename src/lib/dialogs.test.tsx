import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { confirm, DialogHost, prompt, toast, toastsAtTop } from "./dialogs";
import { EXIT_MS } from "./usePresence";

/**
 * The dialog host claims `aria-modal`, so it has to behave like one: focus goes
 * into the card on open and back to the opener on close, and Tab stays inside.
 * These were all missing — focus simply stayed on whatever was behind the scrim.
 *
 * The store behind `DialogHost` is a module singleton, so every test settles its
 * own dialog before finishing.
 */

/** A page behind the scrim, with something focusable to return to. */
function Rig() {
  return (
    <>
      <button data-testid="opener">Open</button>
      <DialogHost />
    </>
  );
}

afterEach(async () => {
  // Escape any dialog a failing assertion left standing.
  const esc = screen.queryByRole("dialog");
  if (esc) {
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  }
  // Let a closing card or toast finish its exit inside the test that closed it.
  await act(() => new Promise((r) => setTimeout(r, EXIT_MS + 20)));
});

describe("DialogHost focus management", () => {
  it("focuses the confirm button, which a confirm dialog had no way to reach", async () => {
    render(<Rig />);
    const pending = confirm({ title: "Delete it?", confirmLabel: "Delete" });

    const button = await screen.findByRole("button", { name: "Delete" });
    await waitFor(() => expect(document.activeElement).toBe(button));

    button.click();
    await expect(pending).resolves.toBe(true);
  });

  it("cancels on Enter when Cancel has focus, even a destructive confirm", async () => {
    render(<Rig />);
    const pending = confirm({ title: "Delete the keyword?", confirmLabel: "Delete", danger: true });
    const remove = await screen.findByRole("button", { name: "Delete" });
    await waitFor(() => expect(document.activeElement).toBe(remove));

    // Shift+Tab from Delete reaches Cancel, as a keyboard user would.
    const cancel = screen.getByRole("button", { name: "Cancel" });
    cancel.focus();
    cancel.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await expect(pending).resolves.toBe(false);
  });

  it("confirms on Enter when the confirm button has focus", async () => {
    render(<Rig />);
    const pending = confirm({ title: "Merge them?", confirmLabel: "Merge" });
    const merge = await screen.findByRole("button", { name: "Merge" });
    await waitFor(() => expect(document.activeElement).toBe(merge));
    merge.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await expect(pending).resolves.toBe(true);
  });

  it("returns focus to the opener when the dialog closes", async () => {
    render(<Rig />);
    const opener = screen.getByTestId("opener");
    opener.focus();
    expect(document.activeElement).toBe(opener);

    const pending = confirm({ title: "Sure?" });
    const button = await screen.findByRole("button", { name: "Confirm" });
    await waitFor(() => expect(document.activeElement).toBe(button));

    button.click();
    await pending;
    await waitFor(() => expect(document.activeElement).toBe(opener));
  });

  it("labels the dialog by its title for assistive tech", async () => {
    render(<Rig />);
    const pending = confirm({ title: "Delete collection?", message: "The photos are kept." });

    const dialog = await screen.findByRole("dialog");
    expect(dialog.getAttribute("aria-modal")).toBe("true");

    // Resolve the references the way assistive tech would. (jest-dom's
    // accessible-name matchers would say this more directly, but it is not a
    // dependency here and this is not worth adding one for.)
    const labelledBy = dialog.getAttribute("aria-labelledby");
    expect(labelledBy).toBeTruthy();
    expect(document.getElementById(labelledBy!)?.textContent).toBe("Delete collection?");

    const describedBy = dialog.getAttribute("aria-describedby");
    expect(describedBy).toBeTruthy();
    expect(document.getElementById(describedBy!)?.textContent).toBe("The photos are kept.");

    screen.getByRole("button", { name: "Cancel" }).click();
    await pending;
  });

  it("focuses the field on a prompt rather than the confirm button", async () => {
    render(<Rig />);
    const pending = prompt({ title: "New collection", defaultValue: "Trip" });

    const field = await screen.findByRole("textbox");
    await waitFor(() => expect(document.activeElement).toBe(field));

    screen.getByRole("button", { name: "Cancel" }).click();
    await expect(pending).resolves.toBeNull();
  });
});

describe("DialogHost queueing", () => {
  // Opening a second dialog used to drop the first one's `resolve` on the floor,
  // leaving its caller awaiting forever.
  it("settles a pending dialog when a second one replaces it", async () => {
    render(<Rig />);
    const first = confirm({ title: "First" });
    await screen.findByText("First");

    const second = confirm({ title: "Second" });
    await screen.findByText("Second");

    await expect(first).resolves.toBe(false);

    screen.getByRole("button", { name: "Cancel" }).click();
    await expect(second).resolves.toBe(false);
  });

  it("resolves a replaced prompt as null rather than hanging", async () => {
    render(<Rig />);
    const first = prompt({ title: "Name it" });
    await screen.findByText("Name it");

    const second = confirm({ title: "Actually, sure?" });
    await screen.findByText("Actually, sure?");

    await expect(first).resolves.toBeNull();

    screen.getByRole("button", { name: "Cancel" }).click();
    await second;
  });
});

describe("toasts and assistive tech", () => {
  it("speaks through regions that are there before the message is", () => {
    render(<DialogHost />);
    // Mounted empty: a region that arrives already holding its message is
    // often not read at all.
    const polite = screen.getByRole("status");
    const urgent = screen.getByRole("alert");
    expect(polite.textContent).toBe("");
    expect(urgent.textContent).toBe("");

    act(() => {
      toast("Copied the path");
      toast("The drive is not connected", "error");
    });
    expect(polite.textContent).toContain("Copied the path");
    expect(urgent.textContent).toContain("The drive is not connected");
    expect(urgent.textContent).not.toContain("Copied the path");
    act(() => {
      for (const t of screen.getAllByText(/Copied the path|The drive is not connected/)) fireEvent.click(t);
    });
  });

  it("stays while the pointer is on it, then goes in its own time", () => {
    vi.useFakeTimers();
    try {
      render(<DialogHost />);
      act(() => toast("A long error to read slowly", "error"));
      const shown = screen.getByText("A long error to read slowly").closest(".toast")!;

      fireEvent.pointerEnter(shown);
      act(() => vi.advanceTimersByTime(60_000));
      expect(screen.queryByText("A long error to read slowly")).not.toBeNull();

      fireEvent.pointerLeave(shown);
      act(() => vi.advanceTimersByTime(5_000));
      expect(screen.queryByText("A long error to read slowly")).not.toBeNull();
      act(() => vi.advanceTimersByTime(2_000));
      // Gone from the store, and fading where it stood: hidden from assistive
      // tech and from the pointer while it does.
      const leaving = screen.getByText("A long error to read slowly").closest(".toast")!;
      expect(leaving.getAttribute("data-closing")).not.toBeNull();
      expect(leaving.getAttribute("aria-hidden")).toBe("true");
      act(() => vi.advanceTimersByTime(EXIT_MS));
      expect(screen.queryByText("A long error to read slowly")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("where the toasts sit", () => {
  // A 760px-tall window with a two-toast stack (about 90px).
  const H = 760;
  const STACK = 90;

  it("stays at the bottom with no dialog open", () => {
    expect(toastsAtTop([], H, STACK)).toBe(false);
  });

  it("stays at the bottom under a short dialog", () => {
    expect(toastsAtTop([{ top: 260, bottom: 500 }], H, STACK)).toBe(false);
  });

  it("goes to the top when a tall dialog's buttons are where the toasts would be", () => {
    // Settings at the minimum window ends 32px above the edge.
    expect(toastsAtTop([{ top: 32, bottom: H - 32 }], H, STACK)).toBe(true);
  });

  it("ignores full-window views, which are not cards", () => {
    expect(toastsAtTop([{ top: 0, bottom: H }], H, STACK)).toBe(false);
  });
});
