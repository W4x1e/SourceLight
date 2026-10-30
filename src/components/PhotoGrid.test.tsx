import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../lib/api", () => ({ api: {}, fileUrl: (p: string) => p }));
vi.mock("../lib/dialogs", () => ({ toast: vi.fn(), confirm: vi.fn() }));

import { useStore } from "../store";
import { EMPTY_FILTER } from "../types";
import PhotoGrid from "./PhotoGrid";

/** The grid with nothing in it: which of its empty states it shows. */
describe("PhotoGrid when empty", () => {
  beforeEach(() => {
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    );
    useStore.setState({ photos: [], searchResults: null, filter: EMPTY_FILTER, committedFilter: EMPTY_FILTER });
  });
  afterEach(() => {
    act(() => useStore.setState({ scanning: null }));
    vi.unstubAllGlobals();
  });

  it("says photos are being read during a first scan, not that none were found", () => {
    useStore.setState({ scanning: { phase: "scan", current: 0, total: 0, message: "" } });
    const { rerender } = render(<PhotoGrid />);
    expect(screen.getByText("Reading your photos…")).toBeTruthy();
    expect(screen.getByText("Looking for photos…")).toBeTruthy();
    expect(screen.queryByText("No photos in this library yet")).toBeNull();

    act(() => useStore.setState({ scanning: { phase: "scan", current: 120, total: 3000, message: "" } }));
    rerender(<PhotoGrid />);
    expect(screen.getByText("120 / 3,000")).toBeTruthy();
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("4");
  });

  it("offers a folder as well as a scan when the library is empty", () => {
    render(<PhotoGrid />);
    expect(screen.getByText("No photos in this library yet")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Add folder…" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Scan for new photos" })).toBeTruthy();
  });

  it("points at the filters when they are what left it empty, scan or no scan", () => {
    const filter = { ...EMPTY_FILTER, minRating: 5 };
    useStore.setState({ filter, committedFilter: filter, scanning: { phase: "scan", current: 1, total: 9, message: "" } });
    render(<PhotoGrid />);
    expect(screen.getByText("No photos match these filters")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Add folder…" })).toBeNull();
  });
});
