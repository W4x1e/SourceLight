import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  api: { setDirectRendering: vi.fn(), renderDirect: vi.fn() },
}));

import type { DirectFrame } from "../bindings";
import { api } from "./api";
import { useDirectFrames, useDirectSurface, useTransparentPage } from "./directCanvas";

const m = vi.mocked(api);

const frame = (width: number): DirectFrame => ({
  x: 0,
  y: 0,
  width,
  height: 600,
  zoom: null,
  cx: 0.5,
  cy: 0.5,
  before: false,
  uncropped: false,
  softProof: false,
  split: false,
  draft: null,
});

describe("useDirectSurface", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  });
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("brings the surface up, and takes it down when no longer wanted", async () => {
    m.setDirectRendering.mockResolvedValue("GPU via direct surface");
    const { result, rerender } = renderHook(({ on }) => useDirectSurface(on), {
      initialProps: { on: true },
    });
    await waitFor(() => expect(result.current).toBe(true));
    rerender({ on: false });
    expect(result.current).toBe(false);
    expect(m.setDirectRendering).toHaveBeenLastCalledWith(false);
  });

  it("stays on the images when the surface cannot be made", async () => {
    m.setDirectRendering.mockRejectedValue("direct rendering is not implemented on this platform");
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const { result } = renderHook(() => useDirectSurface(true));
    await waitFor(() => expect(warn).toHaveBeenCalled());
    expect(result.current).toBe(false);
    warn.mockRestore();
  });

  it("asks for nothing outside the app", () => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    renderHook(() => useDirectSurface(true));
    expect(m.setDirectRendering).not.toHaveBeenCalled();
  });
});

describe("useTransparentPage", () => {
  it("cuts the hole only while on, and closes it on the way out", () => {
    const root = document.documentElement;
    const { rerender, unmount } = renderHook(({ on }) => useTransparentPage(on), {
      initialProps: { on: true },
    });
    expect(root.classList.contains("direct-canvas")).toBe(true);
    rerender({ on: false });
    expect(root.classList.contains("direct-canvas")).toBe(false);
    rerender({ on: true });
    unmount();
    expect(root.classList.contains("direct-canvas")).toBe(false);
  });
});

describe("useDirectFrames", () => {
  beforeEach(() => vi.clearAllMocks());

  it("draws one frame at a time and skips to the newest", async () => {
    let finish: () => void = () => undefined;
    m.renderDirect.mockImplementation(() => new Promise<null>((r) => (finish = () => r(null))));
    const drawn: number[] = [];
    const { rerender } = renderHook(
      ({ f }) => useDirectFrames(f, 1, (d) => drawn.push(d.width)),
      { initialProps: { f: frame(100) as DirectFrame | null } },
    );
    await waitFor(() => expect(m.renderDirect).toHaveBeenCalledTimes(1));

    // A drag: three frames asked for while the first is still drawing.
    rerender({ f: frame(200) });
    rerender({ f: frame(300) });
    rerender({ f: frame(400) });
    expect(m.renderDirect).toHaveBeenCalledTimes(1);

    await act(async () => finish());
    await waitFor(() => expect(m.renderDirect).toHaveBeenCalledTimes(2));
    expect(m.renderDirect).toHaveBeenLastCalledWith(frame(400));
    await act(async () => finish());
    expect(drawn).toEqual([100, 400]);
  });

  it("redraws an unchanged view when the edit moves on, and keeps going after a failure", async () => {
    m.renderDirect.mockRejectedValueOnce("device lost").mockResolvedValue(null);
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const { rerender } = renderHook(({ v }) => useDirectFrames(frame(100), v, () => undefined), {
      initialProps: { v: 1 },
    });
    await waitFor(() => expect(m.renderDirect).toHaveBeenCalledTimes(1));
    rerender({ v: 2 });
    await waitFor(() => expect(m.renderDirect).toHaveBeenCalledTimes(2));
    warn.mockRestore();
  });
});
