import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setLocale } from "../i18n";
import NumberSlider from "./NumberSlider";

function rig(props: Partial<Parameters<typeof NumberSlider>[0]> = {}) {
  const onChange = vi.fn();
  const onCommit = vi.fn();
  const utils = render(
    <NumberSlider
      label="Radius"
      value={0.5}
      min={-1}
      max={1}
      step={0.01}
      onChange={onChange}
      onCommit={onCommit}
      {...props}
    />,
  );
  return { onChange, onCommit, ...utils, field: screen.getByRole("textbox") };
}

describe("NumberSlider", () => {
  it("resets to its own resting value on double-click, not always to 0", () => {
    // A sharpening radius rests at 1; putting it at 0 was a different edit.
    const { onChange, onCommit } = rig({ min: 0.5, max: 3, value: 2.2, defaultValue: 1 });
    fireEvent.doubleClick(screen.getByText("Radius"));
    expect(onChange).toHaveBeenCalledWith(1);
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it("rests at the bottom of a range that does not include 0", () => {
    const { onChange } = rig({ min: 40, max: 100, value: 70 });
    fireEvent.doubleClick(screen.getByRole("slider"));
    expect(onChange).toHaveBeenCalledWith(40);
  });

  it("does not commit when the field is focused and left unchanged", () => {
    // Clicking into the field and out again left an "Exposure" step in the
    // history that changed nothing.
    const { field, onChange, onCommit } = rig();
    fireEvent.focus(field);
    fireEvent.blur(field);
    expect(onChange).not.toHaveBeenCalled();
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("commits a typed value, clamped to the range", () => {
    const { field, onChange, onCommit } = rig();
    fireEvent.focus(field);
    fireEvent.change(field, { target: { value: "5" } });
    fireEvent.blur(field);
    expect(onChange).toHaveBeenCalledWith(1);
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it("signs a positive value on a control that moves both ways", () => {
    const { field } = rig({ value: 0.35 });
    expect((field as HTMLInputElement).value).toBe("+0.35");
  });

  it("does not sign a one-way control", () => {
    const { field } = rig({ label: "Amount", min: 0, max: 1, value: 0.35 });
    expect((field as HTMLInputElement).value).toBe("0.35");
  });

  it("never shows -0", () => {
    const { field } = rig({ label: "Tint", value: -0.001 });
    expect((field as HTMLInputElement).value).toBe("0.00");
  });

  it("throws a typed value away on Escape", () => {
    // Escape blurred the field, and the blur committed what had been typed.
    const { field, onChange, onCommit } = rig();
    fireEvent.focus(field);
    fireEvent.change(field, { target: { value: "0.9" } });
    fireEvent.keyDown(field, { key: "Escape" });
    fireEvent.blur(field);
    expect(onChange).not.toHaveBeenCalled();
    expect(onCommit).not.toHaveBeenCalled();
    expect((field as HTMLInputElement).value).toBe("+0.50");
  });

  it("takes a comma as the decimal mark", () => {
    const { field, onChange } = rig();
    fireEvent.focus(field);
    fireEvent.change(field, { target: { value: "-0,25" } });
    fireEvent.blur(field);
    expect(onChange).toHaveBeenCalledWith(-0.25);
  });

  it("commits a held arrow-key nudge once, on key-up", () => {
    // One History step per repeat of a held key, before.
    const { field, onChange, onCommit, rerender } = rig();
    fireEvent.focus(field);
    fireEvent.keyDown(field, { key: "ArrowUp" });
    expect(onChange).toHaveBeenLastCalledWith(0.51);
    rerender(<NumberSlider label="Radius" value={0.51} min={-1} max={1} step={0.01} onChange={onChange} onCommit={onCommit} />);
    fireEvent.keyDown(field, { key: "ArrowUp", shiftKey: true });
    expect(onChange).toHaveBeenLastCalledWith(0.61);
    expect(onCommit).not.toHaveBeenCalled();
    fireEvent.keyUp(field, { key: "ArrowUp" });
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it("takes ten steps for Shift+arrow on the track", () => {
    const { onChange } = rig();
    fireEvent.keyDown(screen.getByRole("slider"), { key: "ArrowLeft", shiftKey: true });
    expect(onChange).toHaveBeenCalledWith(0.4);
  });

  it("moves a tenth as far on a Shift-drag, from where it was", () => {
    // jsdom has no PointerEvent, so a press would come without Shift or a place.
    if (!("PointerEvent" in window)) {
      class FakePointerEvent extends MouseEvent {
        pointerId: number;
        constructor(type: string, init: PointerEventInit = {}) {
          super(type, init);
          this.pointerId = init.pointerId ?? 0;
        }
      }
      (window as unknown as { PointerEvent: unknown }).PointerEvent = FakePointerEvent;
    }
    const { onChange, onCommit } = rig();
    const track = screen.getByRole("slider");
    // jsdom lays nothing out: a 200px track.
    track.getBoundingClientRect = () => ({ width: 200, left: 0, top: 0, right: 200, bottom: 18, height: 18, x: 0, y: 0, toJSON: () => ({}) });
    track.setPointerCapture = () => {};
    fireEvent.pointerDown(track, { shiftKey: true, button: 0, clientX: 50, pointerId: 1 });
    // 100px is half the track, a whole unit of a -1..1 control; a tenth of it.
    fireEvent.pointerMove(track, { clientX: 150, pointerId: 1 });
    expect(onChange).toHaveBeenLastCalledWith(0.6);
    fireEvent.pointerUp(track, { pointerId: 1 });
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  describe("in German", () => {
    afterEach(async () => {
      await setLocale("en");
    });

    it("shows the decimal comma", async () => {
      await setLocale("de");
      const { field } = rig({ value: 0.35 });
      expect((field as HTMLInputElement).value).toBe("+0,35");
    });
  });

  it("marks itself moved only away from where it rests", () => {
    const { container, rerender, onChange, onCommit } = rig({ value: 1, min: 0.5, max: 3, defaultValue: 1 });
    expect(container.firstElementChild?.classList.contains("slider-moved")).toBe(false);
    rerender(
      <NumberSlider label="Radius" value={1.4} min={0.5} max={3} step={0.1} defaultValue={1} onChange={onChange} onCommit={onCommit} />,
    );
    expect(container.firstElementChild?.classList.contains("slider-moved")).toBe(true);
  });
});
