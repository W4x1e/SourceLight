import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { newMask, type Mask } from "../types";
import MaskPanel, { maskName, type MaskPanelProps } from "./MaskPanel";

/**
 * The panel came out of `Develop.tsx`, which cannot be mounted in a test.
 * These pin the wiring the move could have broken: which callback fires with
 * which index, and that the row actions do not also select the row.
 */
function rig(overrides: Partial<MaskPanelProps> = {}) {
  const props: MaskPanelProps = {
    masks: [newMask("radial"), { ...newMask("brush"), name: "Sky" }],
    selectedMask: 0,
    setSelectedMask: vi.fn(),
    selectedComp: 0,
    setSelectedComp: vi.fn(),
    renamingMask: null,
    setRenamingMask: vi.fn(),
    brush: { size: 0.06, hardness: 0.5, erase: false },
    setBrush: vi.fn(),
    maskOverlay: true,
    setMaskOverlay: vi.fn(),
    picking: false,
    pickPoints: [],
    setPickMode: vi.fn(),
    aiBusy: false,
    aiStage: null,
    aiError: null,
    runAiSubject: vi.fn(),
    setMaskMode: vi.fn(),
    addMask: vi.fn(),
    removeMask: vi.fn(),
    renameMask: vi.fn(),
    editMask: vi.fn(),
    addComponent: vi.fn(),
    removeComponent: vi.fn(),
    editComponent: vi.fn(),
    setCompMode: vi.fn(),
    toggleCompInvert: vi.fn(),
    commit: vi.fn(),
    ...overrides,
  };
  render(<MaskPanel {...props} />);
  return props;
}

describe("MaskPanel", () => {
  it("explains what to do when there are no masks", () => {
    rig({ masks: [] });
    expect(screen.getByText(/No masks yet/)).toBeTruthy();
  });

  it("names masks by their own name, else shape and number", () => {
    expect(maskName(newMask("radial"), 0)).toBe("Radial 1");
    expect(maskName({ ...newMask("linear"), name: "Sky" } as Mask, 3)).toBe("Sky");
    rig();
    expect(screen.getByText("Radial 1")).toBeTruthy();
    expect(screen.getByText("Sky")).toBeTruthy();
  });

  it("adds a mask, or a component to the selected mask, of the chosen shape", () => {
    const p = rig();
    // Both rows offer "+ Linear" while a mask is selected; the titles differ.
    fireEvent.click(screen.getByTitle("New Linear mask"));
    expect(p.addMask).toHaveBeenCalledWith(
      expect.objectContaining({ components: [expect.objectContaining({ mode: "add" })] }),
    );
    fireEvent.click(screen.getByTitle(/Add Linear component/));
    expect(p.addComponent).toHaveBeenCalledWith("linear");
  });

  it("selects a mask and resets to its first component", () => {
    const p = rig();
    fireEvent.click(screen.getByText("Sky"));
    expect(p.setSelectedMask).toHaveBeenCalledWith(1);
    expect(p.setSelectedComp).toHaveBeenCalledWith(0);
  });

  it("lists masks as buttons the keyboard reaches, the selected one pressed", () => {
    // The rows were click-only divs: Tab skipped the whole list.
    const p = rig();
    const sky = screen.getByRole("button", { name: "Sky" });
    expect(screen.getByRole("button", { name: "Radial 1" }).getAttribute("aria-pressed")).toBe("true");
    expect(sky.getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(sky);
    expect(p.setSelectedMask).toHaveBeenCalledTimes(1);
    expect(p.setSelectedMask).toHaveBeenCalledWith(1);
  });

  it("deletes and renames by index without selecting the row", () => {
    const p = rig();
    fireEvent.click(screen.getAllByRole("button", { name: "Delete mask" })[1]!);
    expect(p.removeMask).toHaveBeenCalledWith(1);
    fireEvent.click(screen.getAllByTitle("Rename mask")[0]!);
    expect(p.setRenamingMask).toHaveBeenCalledWith(0);
    expect(p.setSelectedMask).not.toHaveBeenCalled();
  });

  it("commits a rename on Enter", () => {
    const p = rig({ renamingMask: 0 });
    const input = screen.getByDisplayValue("Radial 1");
    fireEvent.change(input, { target: { value: "Face" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(p.renameMask).toHaveBeenCalledWith(0, "Face");
    expect(p.setRenamingMask).toHaveBeenCalledWith(null);
  });

  it("runs the AI subject pick and toggles click-to-select", () => {
    const p = rig();
    fireEvent.click(screen.getByRole("button", { name: /Select subject/ }));
    expect(p.runAiSubject).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: /Click to select/ }));
    expect(p.setPickMode).toHaveBeenCalledWith(true);
  });

  it("shows an AI failure in words", () => {
    rig({ aiError: "the model is not downloaded" });
    expect(screen.getByText("the model is not downloaded")).toBeTruthy();
  });

  it("Done leaves masking", () => {
    const p = rig();
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(p.setMaskMode).toHaveBeenCalledWith(false);
  });
});
