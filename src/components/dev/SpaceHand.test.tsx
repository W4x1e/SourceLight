import { fireEvent, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { newMask } from "../../types";
import MaskOverlay from "./MaskOverlay";
import ClickSelectOverlay from "./ClickSelectOverlay";

/**
 * Space and a left drag pan over the mask and pick overlays,
 * as the middle button does, instead of painting or picking.
 */
beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    },
  );
  // jsdom has no PointerEvent, and without one the button never arrives.
  vi.stubGlobal(
    "PointerEvent",
    class extends MouseEvent {
      pointerId: number;
      constructor(type: string, init: PointerEventInit = {}) {
        super(type, init);
        this.pointerId = init.pointerId ?? 0;
      }
    },
  );
  HTMLElement.prototype.setPointerCapture = () => {};
});
afterEach(() => {
  vi.unstubAllGlobals();
});

const view = { zoom: 2, cx: 0.5, cy: 0.5 };
const framePx = { width: 400, height: 300 };

function brushOverlay() {
  const onChange = vi.fn();
  const onPan = vi.fn();
  const { container } = render(
    <MaskOverlay
      frameAspect={4 / 3}
      framePx={framePx}
      view={view}
      component={newMask("brush").components![0]!}
      brush={{ size: 0.05, hardness: 0.5, erase: false }}
      onChange={onChange}
      onCommit={vi.fn()}
      onPan={onPan}
    />,
  );
  const outer = container.firstElementChild as HTMLElement;
  const surface = outer.querySelector(".cursor-crosshair, .cursor-grab") as HTMLElement;
  return { outer, surface, onChange, onPan };
}

const space = (type: "keyDown" | "keyUp", target: Element | Window = window) =>
  fireEvent[type](target, { code: "Space", key: " " });

describe("Space-drag panning", () => {
  it("paints with a plain left drag", () => {
    const { outer, surface, onChange, onPan } = brushOverlay();
    fireEvent.pointerOver(outer);
    fireEvent.pointerDown(surface, { button: 0, clientX: 10, clientY: 10, pointerId: 1 });
    fireEvent.pointerMove(outer, { clientX: 30, clientY: 20, pointerId: 1 });
    expect(onChange).toHaveBeenCalled();
    expect(onPan).not.toHaveBeenCalled();
  });

  it("pans instead with Space held over the overlay, and paints again once it's let go", () => {
    const { outer, surface, onChange, onPan } = brushOverlay();
    fireEvent.pointerOver(outer);
    space("keyDown");
    expect(surface.className).toContain("cursor-grab");
    fireEvent.pointerDown(surface, { button: 0, clientX: 10, clientY: 10, pointerId: 1 });
    fireEvent.pointerMove(outer, { clientX: 30, clientY: 20, pointerId: 1 });
    fireEvent.pointerUp(outer, { pointerId: 1 });
    expect(onChange).not.toHaveBeenCalled();
    expect(onPan).toHaveBeenCalledWith(20, 10);

    space("keyUp");
    fireEvent.pointerDown(surface, { button: 0, clientX: 10, clientY: 10, pointerId: 1 });
    expect(onChange).toHaveBeenCalled();
  });

  it("leaves Space alone when the pointer is elsewhere, or a field has focus", () => {
    const { outer, surface, onChange } = brushOverlay();
    // Not over the overlay: the key isn't taken.
    expect(space("keyDown")).toBe(true);
    space("keyUp");

    // Over it, but typing in a field.
    const field = document.createElement("input");
    document.body.appendChild(field);
    fireEvent.pointerOver(outer);
    expect(space("keyDown", field)).toBe(true);
    fireEvent.pointerDown(surface, { button: 0, clientX: 10, clientY: 10, pointerId: 1 });
    expect(onChange).toHaveBeenCalled();
    field.remove();
  });

  it("swallows Space over the overlay, so a focused button doesn't click", () => {
    const { outer } = brushOverlay();
    fireEvent.pointerOver(outer);
    expect(space("keyDown")).toBe(false);
    expect(space("keyUp")).toBe(false);
  });

  it("pans over the pick overlay instead of adding a point", () => {
    const onPick = vi.fn();
    const onPan = vi.fn();
    const { container } = render(
      <ClickSelectOverlay
        frameAspect={4 / 3}
        framePx={framePx}
        view={view}
        points={[]}
        busy={false}
        onPick={onPick}
        onPan={onPan}
      />,
    );
    const outer = container.firstElementChild as HTMLElement;
    fireEvent.pointerOver(outer);
    space("keyDown");
    const surface = outer.querySelector(".cursor-grab") as HTMLElement;
    fireEvent.pointerDown(surface, { button: 0, clientX: 5, clientY: 5, pointerId: 1 });
    fireEvent.pointerMove(outer, { clientX: 15, clientY: 5, pointerId: 1 });
    expect(onPick).not.toHaveBeenCalled();
    expect(onPan).toHaveBeenCalledWith(10, 0);
  });
});
