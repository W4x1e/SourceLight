import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { newSpot } from "../types";
import SpotPanel, { type SpotPanelProps } from "./SpotPanel";

/**
 * The panel came out of `Develop.tsx`, which cannot be mounted in a test.
 * These pin the wiring the extraction could have broken: which index each
 * callback gets, and that mode switches commit while drags do not.
 */
function rig(overrides: Partial<SpotPanelProps> = {}) {
  const props: SpotPanelProps = {
    spots: [newSpot(0.2, 0.2), { ...newSpot(0.6, 0.6), heal: false }],
    selected: 1,
    onSelect: vi.fn(),
    onRemove: vi.fn(),
    onEdit: vi.fn(),
    onCommit: vi.fn(),
    onDone: vi.fn(),
    ...overrides,
  };
  render(<SpotPanel {...props} />);
  return props;
}

describe("SpotPanel", () => {
  it("says there are no spots rather than showing an empty list", () => {
    rig({ spots: [], selected: 0 });
    expect(screen.getByText(/no spots yet/i)).toBeTruthy();
  });

  it("lists each spot by mode and number, and selects on click", () => {
    const p = rig();
    expect(screen.getByText("Heal 1")).toBeTruthy();
    // A button the keyboard reaches, once per press.
    fireEvent.click(screen.getByRole("button", { name: "Clone 2" }));
    expect(p.onSelect).toHaveBeenCalledTimes(1);
    expect(p.onSelect).toHaveBeenCalledWith(1);
  });

  it("deletes the spot whose button was pressed, without selecting it", () => {
    const p = rig();
    fireEvent.click(screen.getAllByRole("button", { name: "Delete spot" })[0]!);
    expect(p.onRemove).toHaveBeenCalledWith(0);
    expect(p.onSelect).not.toHaveBeenCalled();
  });

  it("switching the selected spot's mode is one committed step", () => {
    const p = rig();
    fireEvent.click(screen.getByRole("button", { name: "Heal" }));
    expect(p.onEdit).toHaveBeenCalledWith(
      1,
      expect.objectContaining({ heal: true }),
      "Heal mode",
      true,
    );
  });

  it("Done leaves spot mode", () => {
    const p = rig();
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(p.onDone).toHaveBeenCalled();
  });
});
