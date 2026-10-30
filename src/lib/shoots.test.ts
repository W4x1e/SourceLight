import { describe, expect, it } from "vitest";
import { nextStep, progressLine } from "./shoots";
import type { Shoot } from "../bindings";

const shoot = (s: Partial<Shoot>): Shoot => ({
  id: 1,
  name: "Smith wedding",
  createdAt: "2026-09-27T10:00:00Z",
  photos: 100,
  judged: 100,
  picks: 10,
  editedPicks: 10,
  exportedPicks: 10,
  notCopied: 0,
  notBackedUp: 0,
  ...s,
});

describe("a shoot's next step", () => {
  it("culls, then develops the picks, then exports them", () => {
    expect(nextStep(shoot({ judged: 40 })).step).toBe("cull");
    expect(nextStep(shoot({ editedPicks: 3, exportedPicks: 0 })).step).toBe("develop");
    expect(nextStep(shoot({ exportedPicks: 4 })).step).toBe("export");
    expect(nextStep(shoot({})).step).toBe("done");
  });

  it("is done when the picks are out, edited or not, or when there are none", () => {
    expect(nextStep(shoot({ editedPicks: 0 }))).toEqual({ step: "done", label: "Done" });
    expect(nextStep(shoot({ picks: 0, editedPicks: 0, exportedPicks: 0 })).label).toBe("No picks");
    expect(nextStep(shoot({ photos: 0, judged: 0 })).label).toBe("Still being scanned");
  });

  it("says how far along it is in a line", () => {
    expect(progressLine(shoot({ judged: 60, picks: 14, editedPicks: 3, exportedPicks: 0, photos: 120 }))).toBe(
      "120 photos · 50% judged · 14 picks · 3 edited · 0 exported",
    );
    expect(progressLine(shoot({ photos: 1, judged: 0, picks: 0 }))).toBe("1 photo · 0% judged");
  });
});
