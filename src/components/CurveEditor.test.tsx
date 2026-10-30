import { fireEvent, render, screen } from "@testing-library/react";
import { beforeAll, describe, expect, it, vi } from "vitest";
import CurveEditor from "./CurveEditor";

/** The curve by keyboard: it was pointer-only, with no other way in. */
describe("CurveEditor keyboard", () => {
  const three = [
    { x: 0, y: 0 },
    { x: 0.5, y: 0.5 },
    { x: 1, y: 1 },
  ];

  it("moves a point with the arrows and commits once, when the key comes up", () => {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    render(<CurveEditor points={three} onChange={onChange} onCommit={onCommit} />);
    const mid = screen.getAllByRole("slider")[1]!;
    expect(mid.getAttribute("aria-valuetext")).toContain("50");

    fireEvent.keyDown(mid, { key: "ArrowUp" });
    fireEvent.keyDown(mid, { key: "ArrowUp", shiftKey: true });
    expect(onChange).toHaveBeenLastCalledWith([
      { x: 0, y: 0 },
      { x: 0.5, y: 0.56 },
      { x: 1, y: 1 },
    ].map((p) => expect.objectContaining({ x: p.x, y: expect.closeTo(p.y, 5) })));
    expect(onCommit).not.toHaveBeenCalled();

    fireEvent.keyUp(mid, { key: "ArrowUp" });
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it("keeps a point between its neighbours", () => {
    const onChange = vi.fn();
    render(<CurveEditor points={three} onChange={onChange} onCommit={() => {}} />);
    const mid = screen.getAllByRole("slider")[1]!;
    for (let i = 0; i < 20; i++) fireEvent.keyDown(mid, { key: "ArrowRight", shiftKey: true });
    const last = onChange.mock.calls.at(-1)![0] as { x: number }[];
    expect(last[1]!.x).toBeCloseTo(0.99, 5);
  });

  it("removes a middle point with Delete but never an end", () => {
    const onCommit = vi.fn();
    render(<CurveEditor points={three} onChange={() => {}} onCommit={onCommit} />);
    const [first, mid] = screen.getAllByRole("slider");
    fireEvent.keyDown(first!, { key: "Delete" });
    expect(onCommit).not.toHaveBeenCalled();
    fireEvent.keyDown(mid!, { key: "Delete" });
    expect(onCommit).toHaveBeenCalledWith([
      { x: 0, y: 0 },
      { x: 1, y: 1 },
    ]);
  });

  it("adds a point on the curve halfway to the next with Insert", () => {
    const onCommit = vi.fn();
    render(<CurveEditor points={three} onChange={() => {}} onCommit={onCommit} />);
    fireEvent.keyDown(screen.getAllByRole("slider")[1]!, { key: "Insert" });
    const pts = onCommit.mock.calls[0]![0] as { x: number; y: number }[];
    expect(pts).toHaveLength(4);
    expect(pts[2]!.x).toBeCloseTo(0.75, 5);
    expect(pts[2]!.y).toBeCloseTo(0.75, 2);
  });
});

/** Removing a point by pressing it twice, which a finger can do too: the
 *  point's own double-click never reached it once the drawing held the
 *  pointer. */
describe("CurveEditor double press", () => {
  // jsdom has no PointerEvent, so a pressed point would come without its
  // place on screen.
  beforeAll(() => {
    if (!("PointerEvent" in window)) {
      class FakePointerEvent extends MouseEvent {
        pointerId: number;
        constructor(type: string, init: PointerEventInit = {}) {
          super(type, init);
          this.pointerId = init.pointerId ?? 0;
        }
      }
      (window as unknown as { PointerEvent: unknown }).PointerEvent = FakePointerEvent;
    }
  });
  const three = [
    { x: 0, y: 0 },
    { x: 0.5, y: 0.5 },
    { x: 1, y: 1 },
  ];
  // A point on the drawing, which is 100 px square here.
  const at = (x: number, y: number) => ({ clientX: x * 100, clientY: (1 - y) * 100, pointerId: 1 });

  function drawn(onCommit: (p: unknown) => void) {
    const { container } = render(<CurveEditor points={three} onChange={() => {}} onCommit={onCommit} />);
    const svg = container.querySelector("svg")!;
    svg.getBoundingClientRect = () => ({ left: 0, top: 0, width: 100, height: 100, right: 100, bottom: 100, x: 0, y: 0, toJSON: () => ({}) });
    // jsdom has no pointer capture.
    (svg as unknown as { setPointerCapture: () => void }).setPointerCapture = () => {};
    return svg;
  }

  it("removes a middle point pressed twice", () => {
    const onCommit = vi.fn();
    const svg = drawn(onCommit);
    fireEvent.pointerDown(svg, at(0.5, 0.5));
    fireEvent.pointerUp(svg, at(0.5, 0.5));
    fireEvent.pointerDown(svg, at(0.51, 0.5));
    expect(onCommit).toHaveBeenLastCalledWith([
      { x: 0, y: 0 },
      { x: 1, y: 1 },
    ]);
  });

  it("never removes an end", () => {
    const onCommit = vi.fn();
    const svg = drawn(onCommit);
    for (let i = 0; i < 2; i++) {
      fireEvent.pointerDown(svg, at(0, 0));
      fireEvent.pointerUp(svg, at(0, 0));
    }
    for (const call of onCommit.mock.calls) expect(call[0]).toHaveLength(3);
  });
});
