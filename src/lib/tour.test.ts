import { describe, expect, it } from "vitest";
import { ACTIONS } from "./keymap";
import { CARD_GAP, EDGE, TOUR, placeCard } from "./tour";

const viewport = { width: 1280, height: 800 };
const card = { width: 320, height: 180 };

describe("placeCard", () => {
  it("puts the card to the right of a rail button, level with it", () => {
    const target = { left: 4, top: 300, width: 76, height: 56 };
    const at = placeCard(target, card, viewport);
    expect(at.left).toBe(4 + 76 + CARD_GAP);
    expect(at.top).toBe(300 + 28 - 90);
  });

  it("goes below a target with no room on its right", () => {
    const target = { left: 1000, top: 10, width: 260, height: 40 };
    const at = placeCard(target, card, viewport);
    expect(at.top).toBe(10 + 40 + CARD_GAP);
    expect(at.left + card.width).toBeLessThanOrEqual(viewport.width - EDGE);
  });

  it("goes above when there is no room right or below", () => {
    const target = { left: 1000, top: 700, width: 260, height: 60 };
    const at = placeCard(target, card, viewport);
    expect(at.top).toBe(700 - CARD_GAP - card.height);
  });

  it("keeps a rail target's card inside the window near the bottom edge", () => {
    const target = { left: 4, top: 760, width: 76, height: 36 };
    const at = placeCard(target, card, viewport);
    expect(at.top + card.height).toBeLessThanOrEqual(viewport.height - EDGE);
  });

  it("centres with no target, and over a target that fills the window", () => {
    const centred = { left: (1280 - 320) / 2, top: (800 - 180) / 2 };
    expect(placeCard(null, card, viewport)).toEqual(centred);
    expect(placeCard({ left: 0, top: 0, width: 1280, height: 800 }, card, viewport)).toEqual(centred);
  });
});

describe("TOUR", () => {
  it("has unique step ids and only real workspaces and keys", () => {
    expect(new Set(TOUR.map((s) => s.id)).size).toBe(TOUR.length);
    const ids = new Set(ACTIONS.map((a) => a.id));
    for (const step of TOUR) {
      if (step.mode) expect(["browse", "cull", "places", "people"]).toContain(step.mode);
      for (const part of Object.values(step.keys ?? {})) {
        if ("key" in part) expect(ids.has(part.key)).toBe(true);
      }
    }
  });

  it("opens on a centred card and ends on the Settings button", () => {
    expect(TOUR[0]?.target).toBeNull();
    expect(TOUR[TOUR.length - 1]?.target).toBe("rail-settings");
  });
});
