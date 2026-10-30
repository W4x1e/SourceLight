import { act, fireEvent, render, screen } from "@testing-library/react";
import { useRef } from "react";
import { describe, expect, it } from "vitest";
import { useFocusTrap } from "./useFocusTrap";

function View() {
  const ref = useRef<HTMLDivElement>(null);
  useFocusTrap(true, ref);
  return (
    <div ref={ref} tabIndex={-1} data-testid="view">
      <button>first</button>
      <button>last</button>
    </div>
  );
}

const frame = () => act(() => new Promise((r) => requestAnimationFrame(() => r(null))));

/** Tab inside the Lightbox, Compare and the slideshow. */
describe("useFocusTrap", () => {
  it("takes focus, keeps Tab inside, and gives focus back on close", async () => {
    render(<button>opener</button>);
    const opener = screen.getByRole("button", { name: "opener" });
    opener.focus();

    const { unmount } = render(<View />);
    await frame();
    expect(document.activeElement).toBe(screen.getByTestId("view"));

    // From the view itself, Tab goes to its first control, not behind it.
    fireEvent.keyDown(window, { key: "Tab" });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "first" }));

    screen.getByRole("button", { name: "last" }).focus();
    fireEvent.keyDown(window, { key: "Tab" });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "first" }));
    fireEvent.keyDown(window, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "last" }));

    unmount();
    expect(document.activeElement).toBe(opener);
  });
});
