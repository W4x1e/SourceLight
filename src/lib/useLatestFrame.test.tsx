import { act, render } from "@testing-library/react";
import { useEffect, useRef } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useLatestFrame } from "./useLatestFrame";

/**
 * Drives `useLatestFrame` the way the app does: a component asks for a target
 * URL, "renders" it over a fixed delay, then calls `settled`. The freeze this
 * hook fixes was a browser-only symptom until now — the harness that first
 * proved the bound is 1 lived in `harness/frames.tsx` and was thrown away.
 */
function Rig({ target, frameMs, stats }: { target: string; frameMs: number; stats: Stats }) {
  const [shown, settled] = useLatestFrame(target);
  const last = useRef("");

  useEffect(() => {
    if (!shown || shown === last.current) return;
    last.current = shown;
    stats.started += 1;
    stats.inFlight += 1;
    stats.peak = Math.max(stats.peak, stats.inFlight);
    const id = setTimeout(() => {
      stats.inFlight -= 1;
      stats.finished += 1;
      settled();
    }, frameMs);
    return () => clearTimeout(id);
  }, [shown, settled, frameMs, stats]);

  return null;
}

interface Stats {
  started: number;
  finished: number;
  inFlight: number;
  peak: number;
}
const fresh = (): Stats => ({ started: 0, finished: 0, inFlight: 0, peak: 0 });

afterEach(() => vi.useRealTimers());

describe("useLatestFrame", () => {
  it("keeps at most one frame in flight through a burst", () => {
    vi.useFakeTimers();
    const stats = fresh();
    const { rerender } = render(<Rig target="pan:0" frameMs={40} stats={stats} />);

    // 200 updates far faster than a 40ms frame — a drag at mouse-polling rate.
    act(() => {
      for (let i = 1; i <= 200; i++) {
        rerender(<Rig target={`pan:${i}`} frameMs={40} stats={stats} />);
        vi.advanceTimersByTime(2);
      }
      vi.advanceTimersByTime(100); // let the last frame land
    });

    // The whole point: work is bounded by completion, not by input rate.
    expect(stats.peak).toBe(1);
    // And far fewer renders happened than updates were requested.
    expect(stats.started).toBeLessThan(30);
  });

  it("always converges on the newest target", () => {
    vi.useFakeTimers();
    const { rerender, container } = render(<Show target="a" />);
    function Show({ target }: { target: string }) {
      const [shown, settled] = useLatestFrame(target);
      const last = useRef("");
      useEffect(() => {
        if (shown === last.current) return;
        last.current = shown;
        const id = setTimeout(settled, 40);
        return () => clearTimeout(id);
      }, [shown, settled]);
      return <span data-testid="shown">{shown}</span>;
    }

    act(() => {
      rerender(<Show target="b" />);
      rerender(<Show target="c" />);
      vi.advanceTimersByTime(200);
    });

    // Not stuck on "a" (the first-version deadlock) nor on the intermediate
    // "b" — the latest request wins once the pipe drains.
    expect(container.querySelector('[data-testid="shown"]')?.textContent).toBe("c");
  });

  it("releases the pipe when settled is called, unwedging a stuck frame", () => {
    // The contract behind wiring settled to onError as well as onLoad: a frame
    // that never loads must not freeze the preview forever. Here the "frame"
    // never settles on its own; calling settled (as onError would) must let the
    // newest pending target through.
    let settle: () => void = () => {};
    function Show({ target }: { target: string }) {
      const [shown, s] = useLatestFrame(target);
      settle = s;
      return <span data-testid="shown">{shown}</span>;
    }

    // The target shown at mount is itself a frame in flight — the image is
    // loading it — so the next one waits for it too.
    const { rerender, getByTestId } = render(<Show target="a" />);
    rerender(<Show target="b" />); // held behind a
    expect(getByTestId("shown").textContent).toBe("a");
    act(() => settle());
    expect(getByTestId("shown").textContent).toBe("b"); // in flight, never settles on its own
    rerender(<Show target="c" />); // held behind b
    expect(getByTestId("shown").textContent).toBe("b");

    // onError fires for b; the newest held target must then appear.
    act(() => settle());
    expect(getByTestId("shown").textContent).toBe("c");
  });

  it("counts the first frame as in flight, so a request right after mount waits for it", () => {
    // Nothing issues the mount's frame, so the hook used to count it idle and
    // let the next request start beside it: two renders at once. Harmless for
    // an image; for the direct surface, two frames racing into one swapchain.
    vi.useFakeTimers();
    const stats = fresh();
    const { rerender } = render(<Rig target="pan:0" frameMs={40} stats={stats} />);
    act(() => {
      rerender(<Rig target="pan:1" frameMs={40} stats={stats} />);
      vi.advanceTimersByTime(50);
    });
    act(() => vi.advanceTimersByTime(50));
    expect(stats.peak).toBe(1);
    expect(stats.finished).toBe(2);
  });

  it("an empty target is nothing in flight", () => {
    let shown = "";
    function Show({ target }: { target: string }) {
      [shown] = useLatestFrame(target);
      return null;
    }
    const { rerender } = render(<Show target="" />);
    rerender(<Show target="a" />);
    expect(shown).toBe("a");
  });
});
