import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { COPIED_BY_DEFAULT, DEVELOP_GROUPS } from "../../lib/developGroups";
import GroupsDialog from "./GroupsDialog";

function rig(withName = false) {
  const resolve = vi.fn();
  render(<GroupsDialog ask={{ title: "Copy settings", confirmLabel: "Copy", withName, resolve }} />);
  return resolve;
}

describe("GroupsDialog", () => {
  it("answers on Enter from a ticked box, as from the name field", () => {
    // Enter did nothing anywhere but the preset name box.
    const resolve = rig();
    fireEvent.submit(screen.getByRole("checkbox", { name: "Light" }));
    expect(resolve).toHaveBeenCalledWith({ groups: COPIED_BY_DEFAULT, name: "" });
  });

  it("ticks every box, then clears them, and copies nothing with none ticked", () => {
    const resolve = rig();
    fireEvent.click(screen.getByRole("button", { name: "Tick all" }));
    expect(screen.getAllByRole("checkbox").every((c) => (c as HTMLInputElement).checked)).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Clear all" }));
    expect(screen.getAllByRole("checkbox").some((c) => (c as HTMLInputElement).checked)).toBe(false);
    expect(screen.getAllByRole("checkbox")).toHaveLength(DEVELOP_GROUPS.length);
    fireEvent.click(screen.getByRole("button", { name: "Copy" }));
    expect(resolve).not.toHaveBeenCalled();
  });

  it("needs a name to save a preset", () => {
    const resolve = rig(true);
    fireEvent.submit(screen.getByRole("checkbox", { name: "Light" }));
    expect(resolve).not.toHaveBeenCalled();
    fireEvent.change(screen.getByPlaceholderText("Preset name…"), { target: { value: "Warm" } });
    fireEvent.submit(screen.getByPlaceholderText("Preset name…"));
    expect(resolve).toHaveBeenCalledWith({ groups: COPIED_BY_DEFAULT, name: "Warm" });
  });
});
