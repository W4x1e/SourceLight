import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useStore } from "../store";
import ErasePanel, { type ErasePanelProps } from "./ErasePanel";

/**
 * Develop cannot be mounted in a test, so these pin the panel's side of the
 * wiring: when Erase can be pressed, which erase a remove button names, and
 * what shows while an erase runs.
 */
function rig(overrides: Partial<ErasePanelProps> = {}) {
  const piece = { fill: "0".repeat(64), x: 0, y: 0, size: 512 };
  const props: ErasePanelProps = {
    erases: [
      { label: "Selection", pieces: [piece] },
      { label: "Brushed", pieces: [piece] },
    ],
    how: "click",
    onHow: vi.fn(),
    brush: { size: 0.04, remove: false },
    onBrush: vi.fn(),
    selected: false,
    points: 0,
    picking: false,
    running: false,
    pct: null,
    started: false,
    error: null,
    onErase: vi.fn(),
    onClear: vi.fn(),
    onCancel: vi.fn(),
    onRemove: vi.fn(),
    onDone: vi.fn(),
    ...overrides,
  };
  render(<ErasePanel {...props} />);
  return props;
}

const eraseButton = () => screen.getAllByRole("button", { name: "Erase" }).at(-1) as HTMLButtonElement;

describe("ErasePanel", () => {
  it("erases only once something is selected", () => {
    rig();
    expect(eraseButton().disabled).toBe(true);
  });

  it("erases the selection when pressed", () => {
    const p = rig({ selected: true, points: 1 });
    fireEvent.click(eraseButton());
    expect(p.onErase).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByText("Clear selection"));
    expect(p.onClear).toHaveBeenCalledOnce();
  });

  it("waits while a click is being worked out", () => {
    rig({ selected: true, points: 1, picking: true });
    expect(eraseButton().disabled).toBe(true);
    expect(screen.getByText("Selecting…")).toBeTruthy();
  });

  it("lists the erases and puts back the one asked for", () => {
    const p = rig();
    expect(screen.getByText("Erase 1")).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button", { name: "Put back what this erased" })[1]!);
    expect(p.onRemove).toHaveBeenCalledWith(1);
  });

  it("says nothing is erased rather than showing an empty list", () => {
    rig({ erases: [] });
    expect(screen.getByText(/nothing erased yet/i)).toBeTruthy();
  });

  it("brushes on or takes away, at a size", () => {
    const p = rig({ how: "brush" });
    fireEvent.click(screen.getByText("Take away"));
    expect(p.onBrush).toHaveBeenCalledWith({ size: 0.04, remove: true });
    fireEvent.click(screen.getByText("Click to select"));
    expect(p.onHow).toHaveBeenCalledWith("click");
  });

  describe("while the model downloads", () => {
    afterEach(() => useStore.setState({ jobs: {} }));

    it("shows the download alone, with one Cancel", () => {
      // An empty "Starting…" bar with its own Cancel sat over the download's.
      useStore.setState({
        jobs: { "model:erase": { label: "Downloading model", current: 30, total: 100, unit: "bytes" } },
      });
      rig({ selected: true, running: true, pct: null, started: false });
      expect(screen.getByText("Downloading model")).toBeTruthy();
      expect(screen.queryByText("Starting…")).toBeNull();
      expect(screen.getAllByText("Cancel")).toHaveLength(1);
    });
  });

  it("shows progress with Cancel while an erase runs", () => {
    const p = rig({ selected: true, running: true, pct: 40, started: true });
    expect(screen.getByText("40%")).toBeTruthy();
    fireEvent.click(screen.getByText("Cancel"));
    expect(p.onCancel).toHaveBeenCalledOnce();
  });
});
