import { describe, expect, it } from "vitest";
import { arrangePeople, describePeopleRun, facesBacklog, peopleBacklog } from "./people";
import type { FacesSummary } from "../bindings";

const summary = (s: Partial<FacesSummary>): FacesSummary => ({
  photos: 0,
  faces: 0,
  embedded: 0,
  embedFailed: 0,
  firstError: null,
  stopped: false,
  ...s,
});

describe("backlogs", () => {
  it("leaves unrecognised faces out of the automatic pass until the recogniser is installed", () => {
    expect(facesBacklog({ toDetect: 2, toRecognise: 5, recogniserInstalled: false })).toBe(2);
    expect(facesBacklog({ toDetect: 2, toRecognise: 5, recogniserInstalled: true })).toBe(7);
  });

  it("counts every photo for People, which sets the recogniser up itself", () => {
    expect(peopleBacklog({ toDetect: 2, toRecognise: 5, recogniserInstalled: false })).toBe(7);
    expect(peopleBacklog(null)).toBe(0);
  });
});

describe("describePeopleRun", () => {
  it("treats faces found but none recognised as an error, with the reason", () => {
    const r = describePeopleRun({
      summary: summary({ photos: 1086, faces: 272, embedFailed: 272, firstError: "bad input shape" }),
      people: 0,
    });
    expect(r.tone).toBe("error");
    expect(r.text).toContain("272 faces");
    expect(r.text).toContain("bad input shape");
  });

  it("reports what was found and how many people it made", () => {
    const r = describePeopleRun({ summary: summary({ photos: 180, faces: 272, embedded: 272 }), people: 14 });
    expect(r).toEqual({ text: "Found 272 faces in 180 photos — 14 people.", tone: "success" });
  });

  it("says when some faces could not be recognised", () => {
    const r = describePeopleRun({
      summary: summary({ photos: 10, faces: 12, embedded: 10, embedFailed: 2 }),
      people: 3,
    });
    expect(r.tone).toBe("success");
    expect(r.text).toContain("2 couldn't be recognised");
  });

  it("says when there were no faces", () => {
    expect(describePeopleRun({ summary: summary({ photos: 5 }), people: 0 }).text).toBe(
      "No faces found in 5 photos.",
    );
  });

  it("says when a stopped run kept its work", () => {
    expect(describePeopleRun({ summary: summary({ photos: 3, stopped: true }), people: null }).tone).toBe("info");
  });
});

describe("finding and ordering people", () => {
  const people = [
    { id: 1, name: "bo", count: 3 },
    { id: 2, name: null, count: 9 },
    { id: 3, name: "Ada", count: 1 },
    { id: 4, name: null, count: 20 },
    { id: 5, name: "Cyrus Ada", count: 5 },
  ];
  const ids = (list: { id: number }[]) => list.map((p) => p.id);

  it("orders by photos, or by name with the unnamed after", () => {
    expect(ids(arrangePeople(people, "", "photos"))).toEqual([4, 2, 5, 1, 3]);
    expect(ids(arrangePeople(people, "", "name"))).toEqual([3, 1, 5, 4, 2]);
  });

  it("finds a name anywhere in it, and the unnamed by asking for them", () => {
    expect(ids(arrangePeople(people, " ADA", "photos"))).toEqual([5, 3]);
    expect(ids(arrangePeople(people, "unn", "photos"))).toEqual([4, 2]);
    expect(ids(arrangePeople(people, "zed", "photos"))).toEqual([]);
  });
});
