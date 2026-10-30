import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { StrictMode, useState } from "react";
import ZoomableImage, {
  FITTED,
  type ZoomView,
  fittedDevicePixels,
  type ZoomSource,
  zoomSourceLabel,
  zoomView,
} from "./ZoomableImage";

/**
 * The full-resolution swap. jsdom never loads an image, so `Image` is
 * replaced with one the test resolves by hand — which is also what makes the
 * ordering cases (a load that lands after the photo changed) testable at all.
 */

class FakeImage {
  static made: FakeImage[] = [];
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  naturalWidth = 0;
  src = "";
  constructor() {
    FakeImage.made.push(this);
  }
  load(width: number) {
    this.naturalWidth = width;
    this.onload?.();
  }
}

const RealImage = globalThis.Image;
beforeEach(() => {
  FakeImage.made = [];
  globalThis.Image = FakeImage as unknown as typeof Image;
});
afterEach(() => {
  globalThis.Image = RealImage;
});

function rig(resetKey: number, sources: ZoomSource[]) {
  return (
    <ZoomableImage
      src={`preview-${resetKey}`}
      fullSrc={`full-${resetKey}`}
      alt="photo"
      resetKey={resetKey}
      onSourceChange={(s) => sources.push(s)}
    />
  );
}

const shown = () => screen.getByRole("img").getAttribute("src");
const zoomIn = () => fireEvent.doubleClick(screen.getByRole("img"));

describe("ZoomableImage full-resolution source", () => {
  it("fetches nothing until the first zoom", () => {
    const sources: ZoomSource[] = [];
    render(rig(1, sources));
    expect(FakeImage.made).toHaveLength(0);
    expect(shown()).toBe("preview-1");
    expect(sources.at(-1)).toBe("preview");
  });

  it("warms the full pixels after a moment on screen, without swapping them in", () => {
    vi.useFakeTimers();
    try {
      const sources: ZoomSource[] = [];
      const { rerender } = render(rig(1, sources));
      // Paged past quickly: nothing fetched.
      act(() => vi.advanceTimersByTime(300));
      rerender(rig(2, sources));
      act(() => vi.advanceTimersByTime(300));
      expect(FakeImage.made).toHaveLength(0);
      // Stayed on: fetched once, and the preview stays on screen.
      act(() => vi.advanceTimersByTime(500));
      // Marked, so the backend can drop it once someone has moved on.
      expect(FakeImage.made.map((i) => i.src)).toEqual(["full-2?prefetch=1"]);
      expect(shown()).toBe("preview-2");
      expect(sources.at(-1)).toBe("preview");
    } finally {
      vi.useRealTimers();
    }
  });

  it("swaps in the full pixels once they have loaded", () => {
    const sources: ZoomSource[] = [];
    render(rig(1, sources));
    zoomIn();
    expect(FakeImage.made).toHaveLength(1);
    expect(FakeImage.made[0]!.src).toBe("full-1");
    expect(sources.at(-1)).toBe("loading");
    // Still the preview until the full one has decoded — never a blank frame.
    expect(shown()).toBe("preview-1");

    act(() => FakeImage.made[0]!.load(6000));
    expect(shown()).toBe("full-1");
    expect(sources.at(-1)).toBe("full");
  });

  it("keeps the preview when there is nothing better", () => {
    const sources: ZoomSource[] = [];
    render(rig(1, sources));
    zoomIn();
    // A 204 or a decode failure both arrive as an error.
    act(() => FakeImage.made[0]!.onerror?.());
    expect(shown()).toBe("preview-1");
    expect(sources.at(-1)).toBe("preview");
  });

  it("asks once per photo, not once per zoom", () => {
    render(rig(1, []));
    zoomIn();
    act(() => FakeImage.made[0]!.load(6000));
    zoomIn(); // out
    zoomIn(); // and in again
    expect(FakeImage.made).toHaveLength(1);
  });

  it("ignores a load that lands after the photo changed", () => {
    const sources: ZoomSource[] = [];
    const { rerender } = render(rig(1, sources));
    zoomIn();
    const stale = FakeImage.made[0]!;

    rerender(rig(2, sources));
    act(() => stale.load(6000));
    expect(shown()).toBe("preview-2");

    // The new photo fetches its own when zoomed.
    zoomIn();
    expect(FakeImage.made.at(-1)!.src).toBe("full-2");
  });

  it("says which pixels are on screen", () => {
    expect(zoomSourceLabel("preview")).toMatch(/preview/i);
    expect(zoomSourceLabel("loading")).toMatch(/loading/i);
    expect(zoomSourceLabel("full")).toMatch(/full resolution/i);
  });
});

/**
 * Zooming keeps the point under the cursor where it is. Found by the UI
 * harness: the pan was set from inside the scale's state updater, which
 * React runs twice in development, so every zoom panned twice as far and the
 * view slid off the point you were looking at.
 */
describe("ZoomableImage zoom anchoring", () => {
  it("rescales about the given point", () => {
    // Point 100px right of centre stays put going from 1x to 2x.
    expect(zoomView({ scale: 1, x: 0, y: 0 }, 2, 100, 0)).toEqual({ scale: 2, x: -100, y: 0 });
    // Chained zooms compose: 2x -> 4x about the same point.
    expect(zoomView({ scale: 2, x: -100, y: 0 }, 4, 100, 0)).toEqual({ scale: 4, x: -300, y: 0 });
    // Back to 1 is always fitted; the same scale is no change.
    expect(zoomView({ scale: 3, x: 50, y: 50 }, 1, 10, 10)).toEqual({ scale: 1, x: 0, y: 0 });
    const v = { scale: 2, x: 5, y: 5 };
    expect(zoomView(v, 2, 99, 99)).toBe(v);
  });

  it("pans once, not twice, under StrictMode", () => {
    render(
      <StrictMode>
        <ZoomableImage src="preview-1" alt="photo" resetKey={1} />
      </StrictMode>,
    );
    // jsdom lays nothing out: the box centre is (0, 0) and 1:1 falls back to
    // 2x, so a double-click 100px right of centre must pan by exactly -100px.
    fireEvent.doubleClick(screen.getByRole("img"), { clientX: 100, clientY: 0 });
    expect(screen.getByRole("img").style.transform).toBe("translate(-100px, 0px) scale(2)");
  });
});

describe("ZoomableImage with a shared view", () => {
  /** What Compare does: one view in the parent, handed to every tile. */
  function Pair({ start }: { start: ZoomView }) {
    const [view, setView] = useState<ZoomView>(start);
    return (
      <>
        <ZoomableImage src="preview-1" alt="one" resetKey={1} view={view} onViewChange={setView} />
        <ZoomableImage src="preview-2" alt="two" resetKey={2} view={view} onViewChange={setView} />
      </>
    );
  }
  const transform = (name: string) => screen.getByRole("img", { name }).style.transform;

  it("starts a new set fitted, whatever the last one was zoomed to", () => {
    render(<Pair start={{ scale: 3, x: 10, y: -5 }} />);
    expect(transform("one")).toBe("translate(0px, 0px) scale(1)");
    expect(transform("two")).toBe("translate(0px, 0px) scale(1)");
  });

  it("zooms and pans every tile together", () => {
    // jsdom's pointer events drop the coordinates a drag is made of; a mouse
    // event carries them, so stand one in for the length of this test.
    const RealPointer = window.PointerEvent;
    class Pointer extends MouseEvent {
      pointerId: number;
      constructor(type: string, init: PointerEventInit = {}) {
        super(type, init);
        this.pointerId = init.pointerId ?? 0;
      }
    }
    window.PointerEvent = Pointer as unknown as typeof PointerEvent;
    try {
      dragTogether();
    } finally {
      window.PointerEvent = RealPointer;
    }
  });

  function dragTogether() {
    render(<Pair start={FITTED} />);
    // jsdom lays nothing out, so 1:1 falls back to 2x about the box centre.
    fireEvent.doubleClick(screen.getByRole("img", { name: "one" }), { clientX: 100, clientY: 0 });
    expect(transform("one")).toBe("translate(-100px, 0px) scale(2)");
    expect(transform("two")).toBe("translate(-100px, 0px) scale(2)");

    // A drag on the second tile moves the first as well.
    const two = screen.getByRole("img", { name: "two" }).parentElement!;
    two.setPointerCapture = () => {};
    fireEvent.pointerDown(two, { clientX: 0, clientY: 0, pointerId: 1 });
    fireEvent.pointerMove(two, { clientX: 30, clientY: 20, pointerId: 1 });
    fireEvent.pointerUp(two, { clientX: 30, clientY: 20, pointerId: 1 });
    expect(transform("one")).toBe("translate(-70px, 20px) scale(2)");
    expect(transform("two")).toBe("translate(-70px, 20px) scale(2)");
  }
});

/** When a fitted frame needs more than the 1600px preview has. */
describe("fittedDevicePixels", () => {
  const full = { naturalWidth: 6000, naturalHeight: 4000 };

  it("is the box width in device pixels for a landscape frame in a wide box", () => {
    expect(fittedDevicePixels({ clientWidth: 1800, clientHeight: 1400 }, full, 1.5)).toBe(2700);
  });

  it("is limited by the height when the box is shorter than the frame's aspect", () => {
    expect(fittedDevicePixels({ clientWidth: 2000, clientHeight: 800 }, full, 1)).toBe(1200);
  });

  it("never asks for more than the source has", () => {
    expect(fittedDevicePixels({ clientWidth: 5000, clientHeight: 5000 }, { naturalWidth: 1600, naturalHeight: 1067 }, 2)).toBe(3200);
  });

  it("is zero without a box to fit", () => {
    expect(fittedDevicePixels(null, full, 2)).toBe(0);
  });
});

/** Stepping through a burst at the same zoom. */
describe("ZoomableImage keepView", () => {
  const keep = (resetKey: number, keepView: boolean, zoomed: boolean[]) => (
    <ZoomableImage
      src={`preview-${resetKey}`}
      fullSrc={`full-${resetKey}`}
      alt="photo"
      resetKey={resetKey}
      keepView={keepView}
      onZoomChange={(z) => zoomed.push(z)}
    />
  );

  it("stays zoomed on the next frame, and fetches its full pixels at once", () => {
    const zoomed: boolean[] = [];
    const { rerender } = render(keep(1, true, zoomed));
    zoomIn();
    expect(zoomed.at(-1)).toBe(true);
    rerender(keep(2, true, zoomed));
    expect(zoomed.at(-1)).toBe(true);
    expect(FakeImage.made.map((i) => i.src)).toContain("full-2");
  });

  it("starts the next frame fitted without it", () => {
    const zoomed: boolean[] = [];
    const { rerender } = render(keep(1, false, zoomed));
    zoomIn();
    rerender(keep(2, false, zoomed));
    expect(zoomed.at(-1)).toBe(false);
  });
});
