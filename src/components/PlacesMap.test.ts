import { describe, expect, it } from "vitest";
import { clampView, toLatLon } from "./PlacesMap";

describe("toLatLon", () => {
  it("reads the equirectangular box back as a position", () => {
    // Edinburgh, drawn at (lon + 180, 90 - lat).
    const { lat, lon } = toLatLon(-3.19 + 180, 90 - 55.95);
    expect(lat).toBeCloseTo(55.95);
    expect(lon).toBeCloseTo(-3.19);
  });

  it("never pins off the edge of the world", () => {
    expect(toLatLon(-5, -5)).toEqual({ lat: 90, lon: -180 });
    expect(toLatLon(400, 200)).toEqual({ lat: -90, lon: 180 });
  });
});

describe("clampView", () => {
  it("stops zooming in at the coastline's resolution and out at the whole world", () => {
    expect(clampView({ x: 100, y: 50, w: 1, h: 0.5 }).w).toBe(24);
    expect(clampView({ x: 0, y: 0, w: 900, h: 450 })).toEqual({ x: 0, y: 0, w: 360, h: 180 });
  });

  it("keeps a pan inside the world, at 2:1", () => {
    const v = clampView({ x: 350, y: -20, w: 60, h: 10 });
    expect(v).toEqual({ x: 300, y: 0, w: 60, h: 30 });
  });
});
