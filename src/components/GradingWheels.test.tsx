import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import GradingWheels from "./GradingWheels";
import type { Grading } from "../types";

const grading: Grading = {
  shadows: { hue: 200, saturation: 0.3, luminance: -0.1 },
  midtones: { hue: 0, saturation: 0, luminance: 0 },
  highlights: { hue: 40, saturation: 0.2, luminance: 0.05 },
  blending: 0,
  balance: 0,
};

function rig() {
  const onChange = vi.fn();
  const onCommit = vi.fn();
  render(<GradingWheels grading={grading} onChange={onChange} onCommit={onCommit} />);
  return { onChange, onCommit, shadows: screen.getByRole("slider", { name: "Shadows colour" }) };
}

describe("GradingWheels", () => {
  it("puts a wheel back to no colour on double-click, keeping its brightness", () => {
    // The one control in the rail that double-click did not reset.
    const { onCommit, shadows } = rig();
    fireEvent.doubleClick(shadows);
    expect(onCommit).toHaveBeenCalledTimes(1);
    const [next, label] = onCommit.mock.calls[0]!;
    expect(next.shadows).toEqual({ hue: 0, saturation: 0, luminance: -0.1 });
    expect(next.highlights).toEqual(grading.highlights);
    expect(label).toBe("Reset shadows colour");
  });

  it("is moved from the keyboard, and a held key commits once", () => {
    const { onChange, onCommit, shadows } = rig();
    fireEvent.keyDown(shadows, { key: "ArrowRight" });
    expect(onChange.mock.lastCall?.[0].shadows.hue).toBe(201);
    fireEvent.keyDown(shadows, { key: "ArrowUp", shiftKey: true });
    expect(onChange.mock.lastCall?.[0].shadows.saturation).toBeCloseTo(0.4);
    expect(onCommit).not.toHaveBeenCalled();
    fireEvent.keyUp(shadows, { key: "ArrowUp" });
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it("keeps the arrows from reaching Develop, where they change the photo", () => {
    const { shadows } = rig();
    const onWindow = vi.fn();
    window.addEventListener("keydown", onWindow);
    fireEvent.keyDown(shadows, { key: "ArrowLeft" });
    window.removeEventListener("keydown", onWindow);
    expect(onWindow).not.toHaveBeenCalled();
  });
});
