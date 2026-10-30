import { describe, expect, it } from "vitest";
import { DEFAULT_STACK, type Stack } from "../../src/types";
import { isMoved, restOf, same, TOOLS } from "./EditPanels";

const base = (): Stack => structuredClone(DEFAULT_STACK);

describe("same", () => {
  it("ignores the order fields came in, and float dust", () => {
    expect(same({ a: 1, b: { c: 0.5 } }, { b: { c: 0.5000000001 }, a: 1 })).toBe(true);
    expect(same([1, 2], [1, 2])).toBe(true);
    expect(same([1, 2], [2, 1])).toBe(false);
    expect(same({ a: 1 }, { a: 1, b: 2 })).toBe(false);
    expect(same("Portra", "Portra")).toBe(true);
  });
});

describe("the editor's tools", () => {
  it("are all at rest on an unedited photo", () => {
    for (const { id } of TOOLS) expect(isMoved(id, base())).toBe(false);
  });

  it("stay at rest when the computer sends the same edit in another order", () => {
    const s = base();
    // Rust's field order, not the page's.
    s.effects = {
      grainHighlightRolloff: 0.7,
      halationThreshold: 0.72,
      halation: 0,
      grainSize: 1,
      grain: 0,
      vignetteMidpoint: 0.5,
      vignette: 0,
    };
    s.geometry = { flip: false, quarterTurns: 0, straighten: 0, crop: { height: 1, width: 1, y: 0, x: 0 } };
    expect(isMoved("effects", s)).toBe(false);
    expect(isMoved("crop", s)).toBe(false);
  });

  it("each see a change to their own part, and only theirs", () => {
    const changes: [string, (s: Stack) => void][] = [
      ["light", (s) => (s.exposure = 0.5)],
      ["colour", (s) => (s.warmth = 0.2)],
      ["curve", (s) => (s.curveRed = [{ x: 0.5, y: 0.6 }])],
      ["mixer", (s) => (s.mixerSaturation[3] = -0.4)],
      ["grading", (s) => (s.grading.shadows.saturation = 0.3)],
      ["detail", (s) => (s.detail.sharpen = 0.4)],
      ["effects", (s) => (s.detail.clarity = 0.2)],
      ["optics", (s) => (s.lens.distortion = 0.1)],
      ["crop", (s) => (s.geometry.flip = true)],
      ["luts", (s) => (s.lut = { name: "warm-negative.cube", intensity: 1 })],
    ];
    for (const [tool, edit] of changes) {
      const s = base();
      edit(s);
      for (const { id } of TOOLS) expect(isMoved(id, s), `${tool} seen by ${id}`).toBe(id === tool);
    }
  });

  it("see a curve whose added points were all taken off as no curve", () => {
    const s = base();
    s.curve = [
      { x: 0, y: 0 },
      { x: 1, y: 1 },
    ];
    expect(isMoved("curve", s)).toBe(false);
    s.curveBlue = [
      { x: 0, y: 0.1 },
      { x: 1, y: 1 },
    ];
    expect(isMoved("curve", s)).toBe(true);
  });

  it("are back at rest after their Reset, leaving the rest of the edit", () => {
    const s = base();
    s.detail = { ...s.detail, sharpen: 0.4, clarity: 0.3 };
    const reset = { ...s, ...restOf("detail", s) };
    expect(isMoved("detail", reset)).toBe(false);
    // Clarity is the Effects tool's, and stays.
    expect(reset.detail.clarity).toBe(0.3);
  });

  it("show a preset by its being on, not by the settings it shares", () => {
    const s = base();
    s.contrast = 0.2; // the Light tool's, which a preset also sets
    expect(isMoved("presets", s)).toBe(false);
    s.preset = { ...s.preset, id: "film-warm", amount: 1 };
    expect(isMoved("presets", s)).toBe(true);
    // Its Reset takes the preset off and puts what it set back to neutral.
    const reset = { ...s, ...restOf("presets", s) };
    expect(isMoved("presets", reset)).toBe(false);
    expect(reset.contrast).toBe(0);
    expect(reset.exposure).toBe(s.exposure);
  });

  it("give back a preset's tone share, unless the slider moved since", () => {
    const s = base();
    const none = { added: 0, left: null };
    s.highlights = -0.4; // the preset took 0.2 off a sky pulled back to -0.2
    s.shadows = 0.5; // moved by hand since the preset left it at 0.3
    s.preset = { id: "landscape-alpine", amount: 1, tone: [{ added: -0.2, left: -0.4 }, { added: 0.1, left: 0.3 }, none, none] };
    const reset = { ...s, ...restOf("presets", s) };
    expect(reset.highlights).toBeCloseTo(-0.2);
    expect(reset.shadows).toBe(0.5);
  });
});
