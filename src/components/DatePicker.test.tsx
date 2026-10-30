import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import DatePicker from "./DatePicker";

/** The filter rail's date picker. */
describe("DatePicker", () => {
  it("closes on Escape and keeps the key from the grid behind it", () => {
    // The grid clears its selection on an Escape that reaches the window.
    const behind = vi.fn();
    window.addEventListener("keydown", behind);
    render(<DatePicker value={null} onChange={() => {}} placeholder="Any" />);
    fireEvent.click(screen.getByRole("button", { name: /Any/ }));
    expect(screen.getByTitle("Previous month")).toBeTruthy();

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByTitle("Previous month")).toBeNull();
    expect(behind).not.toHaveBeenCalled();
    window.removeEventListener("keydown", behind);
  });
});
