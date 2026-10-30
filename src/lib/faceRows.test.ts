import { describe, expect, it } from "vitest";
import type { PhotoFace } from "../bindings";
import { everyoneOpen, faceEyesClosed, faceRows, iou, rowFocus } from "./faceRows";

let nextId = 1;
const face = (photoId: number, box: number[], over: Partial<PhotoFace> = {}): PhotoFace => ({
  faceId: nextId++,
  photoId,
  personId: null,
  personName: null,
  boxFrac: box,
  sharpness: 0.5,
  eyeOpen: 0.9,
  portraitPath: null,
  ...over,
});

const LEFT = [0.1, 0.2, 0.2, 0.3];
const LEFT_MOVED = [0.12, 0.21, 0.2, 0.3];
const RIGHT = [0.6, 0.2, 0.2, 0.3];

const who = (rows: ReturnType<typeof faceRows>["rows"]) =>
  rows.map((r) => r.cells.map((c) => (c ? c.photoId : null)));

describe("faceRows", () => {
  it("puts a grouped person's faces in one row across the burst", () => {
    const { rows } = faceRows(
      [1, 2, 3],
      [
        face(1, LEFT, { personId: 7, personName: "Ada" }),
        face(3, LEFT, { personId: 7, personName: "Ada" }),
        face(2, RIGHT, { personId: 8 }),
      ],
    );
    expect(rows[0]).toMatchObject({ personId: 7, name: "Ada" });
    expect(who(rows)).toEqual([
      [1, null, 3],
      [null, 2, null],
    ]);
  });

  it("lines up unknown faces by where they are in the frame", () => {
    const { rows } = faceRows([1, 2], [face(1, LEFT), face(1, RIGHT), face(2, RIGHT), face(2, LEFT_MOVED)]);
    expect(rows).toHaveLength(2);
    for (const row of rows) {
      const [a, b] = row.cells;
      // Each row holds the left face twice or the right face twice.
      expect(Math.abs(a!.boxFrac[0]! - b!.boxFrac[0]!)).toBeLessThan(0.1);
    }
  });

  it("measures overlap as intersection over union", () => {
    expect(iou([0, 0, 1, 1], [0, 0, 1, 1])).toBe(1);
    expect(iou([0, 0, 1, 1], [2, 2, 1, 1])).toBe(0);
    expect(iou([0, 0, 2, 1], [1, 0, 2, 1])).toBeCloseTo(1 / 3);
    expect(iou(null, [0, 0, 1, 1])).toBe(0);
  });

  it("lets an unknown face join a named person's row when it sits where they do", () => {
    const { rows } = faceRows([1, 2], [face(1, LEFT, { personId: 7 }), face(2, LEFT_MOVED)]);
    expect(who(rows)).toEqual([[1, 2]]);
  });

  it("starts a new row for a face that overlaps nobody", () => {
    const { rows } = faceRows([1, 2], [face(1, LEFT), face(2, RIGHT)]);
    expect(rows).toHaveLength(2);
  });

  it("never puts two faces of one frame in one row", () => {
    // The grouping put both under person 7; the smaller one must go elsewhere.
    const big = face(1, [0.1, 0.1, 0.4, 0.4], { personId: 7 });
    const small = face(1, [0.7, 0.7, 0.1, 0.1], { personId: 7 });
    const { rows } = faceRows([1], [small, big]);
    expect(rows).toHaveLength(2);
    expect(rows.find((r) => r.personId === 7)!.cells[0]!.faceId).toBe(big.faceId);
  });

  it("orders rows by how many frames they appear in, and caps them", () => {
    const faces = [face(1, LEFT), face(2, LEFT), face(1, RIGHT), face(1, [0.4, 0.7, 0.1, 0.1])];
    const { rows, more } = faceRows([1, 2], faces, 2);
    expect(rows[0]!.cells.filter(Boolean)).toHaveLength(2);
    expect(rows).toHaveLength(2);
    expect(more).toBe(1);
  });

  it("ignores faces from photos outside the burst", () => {
    expect(faceRows([1], [face(9, LEFT)]).rows).toEqual([]);
  });
});

describe("eyes", () => {
  it("reads a face closed below the cut, unless the photo was marked not closed", () => {
    const shut = face(1, LEFT, { eyeOpen: 0.1 });
    expect(faceEyesClosed(shut)).toBe(true);
    expect(faceEyesClosed(shut, 0)).toBe(false);
    expect(faceEyesClosed(face(1, LEFT, { eyeOpen: null }))).toBe(false);
  });

  it("claims everyone's eyes open only for two or more faces, all read and open", () => {
    const open = () => face(1, LEFT);
    expect(everyoneOpen([open(), open()])).toBe(true);
    expect(everyoneOpen([open()])).toBe(false);
    expect(everyoneOpen([open(), face(1, RIGHT, { eyeOpen: 0.1 })])).toBe(false);
    expect(everyoneOpen([open(), face(1, RIGHT, { eyeOpen: null })])).toBe(false);
    expect(everyoneOpen([open(), face(1, RIGHT, { eyeOpen: 0.1 })], 0)).toBe(true);
  });
});

describe("rowFocus", () => {
  it("names the sharpest face and flags the ones well behind it", () => {
    const a = face(1, LEFT, { sharpness: 0.9 });
    const b = face(2, LEFT, { sharpness: 0.6 });
    const c = face(3, LEFT, { sharpness: 0.3 });
    const { sharpest, soft } = rowFocus({ personId: null, name: null, cells: [a, b, c] });
    expect(sharpest).toBe(a.faceId);
    expect([...soft]).toEqual([c.faceId]);
  });

  it("says nothing about a row with one scored face", () => {
    const { sharpest, soft } = rowFocus({ personId: null, name: null, cells: [face(1, LEFT), null] });
    expect(sharpest).toBeNull();
    expect(soft.size).toBe(0);
  });
});
