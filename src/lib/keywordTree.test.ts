import { describe, expect, it } from "vitest";
import {
  buildKeywordTree,
  findInTree,
  flatten,
  inBranch,
  parentsOf,
  pathOf,
  selectionCounts,
  type KeywordRow,
} from "./keywordTree";
import type { KeywordNode } from "../bindings";

const node = (name: string, parent: string | null = null, kept = false): KeywordNode => ({ name, parent, kept });
const counts = (o: Record<string, number>) => Object.entries(o).map(([value, count]) => ({ value, count }));
const names = (rows: KeywordRow[]): unknown[] =>
  rows.map((r) => (r.children.length ? [r.name, names(r.children)] : r.name));

const TREE = [
  node("animals", null, true),
  node("dog", "animals"),
  node("cat", "animals"),
  node("pets", "animals", true),
  node("car"),
  node("sky"),
];

describe("buildKeywordTree", () => {
  it("files keywords under their parents: the user's own first, then the most-used, branches by name", () => {
    const rows = buildKeywordTree(TREE, counts({ animals: 3, dog: 2, cat: 1, car: 5, sky: 1 }), []);
    expect(names(rows)).toEqual([["animals", ["cat", "dog", "pets"]], "car", "sky"]);
    expect(rows.find((r) => r.name === "animals")?.count).toBe(3);
  });

  it("keeps the tree someone built with nothing under the filter, but drops unfiled words", () => {
    const rows = buildKeywordTree(TREE, counts({ car: 1 }), []);
    // The branch stays (its keywords are filed); the tagger's "sky" goes.
    expect(names(rows)).toEqual([["animals", ["cat", "dog", "pets"]], "car"]);
    expect(rows.find((r) => r.name === "animals")?.filed).toBe(true);
  });

  it("keeps a chosen keyword, so the choice can be taken back", () => {
    expect(names(buildKeywordTree(TREE, counts({}), ["sky", "gone"]))).toContain("sky");
    expect(names(buildKeywordTree(TREE, counts({}), ["gone"]))).toContain("gone");
  });

  it("puts a keyword only the counts know about at the top", () => {
    expect(names(buildKeywordTree([], counts({ boat: 2 }), []))).toEqual(["boat"]);
  });

  it("cuts a loop rather than losing the keywords in it", () => {
    const loop = [node("a", "b"), node("b", "a")];
    const rows = buildKeywordTree(loop, counts({ a: 1, b: 1 }), []);
    expect(
      flatten(rows, () => true)
        .map((f) => f.row.name)
        .sort(),
    ).toEqual(["a", "b"]);
  });
});

describe("paths and branches", () => {
  const parents = parentsOf(TREE);

  it("reads a keyword's path root first", () => {
    expect(pathOf(parents, "dog")).toEqual(["animals", "dog"]);
    expect(pathOf(parents, "car")).toEqual(["car"]);
    expect(pathOf(parentsOf([node("a", "b"), node("b", "a")]), "a")).toEqual(["b", "a"]);
  });

  it("knows what is inside a branch", () => {
    expect(inBranch(parents, "dog", "animals")).toBe(true);
    expect(inBranch(parents, "animals", "animals")).toBe(true);
    expect(inBranch(parents, "animals", "dog")).toBe(false);
  });
});

describe("findInTree", () => {
  const rows = buildKeywordTree(TREE, counts({ dog: 2, cat: 1, car: 5 }), []);

  it("keeps the matches and the branches that lead to them", () => {
    expect(names(findInTree(rows, "Do"))).toEqual([["animals", ["dog"]]]);
    expect(names(findInTree(rows, "ca"))).toEqual([["animals", ["cat"]], "car"]);
  });

  it("keeps everything for an empty query", () => {
    expect(findInTree(rows, "  ")).toBe(rows);
  });
});

describe("flatten", () => {
  it("walks open branches in order, with their depth", () => {
    const tree = buildKeywordTree([node("a"), node("b", "a"), node("c", "b")], counts({ c: 1 }), []);
    expect(flatten(tree, () => false).map((f) => f.row.name)).toEqual(["a"]);
    expect(flatten(tree, () => true).map((f) => [f.row.name, f.depth])).toEqual([
      ["a", 0],
      ["b", 1],
      ["c", 2],
    ]);
  });
});

describe("selectionCounts", () => {
  it("counts how many of the photos have each keyword", () => {
    const got = selectionCounts([{ subjects: ["dog", "sky"] }, { subjects: ["dog"] }]);
    expect(got.get("dog")).toBe(2);
    expect(got.get("sky")).toBe(1);
    expect(got.get("cat")).toBeUndefined();
  });
});

describe("the top level", () => {
  it("keeps a keyword just made in sight above hundreds of the tagger's words", () => {
    const words = Array.from({ length: 300 }, (_, i) => node(`word${i}`));
    const rows = buildKeywordTree(
      [...words, node("landforms", null, true)],
      counts(Object.fromEntries(words.map((w, i) => [w.name, 300 - i]))),
      [],
    );
    expect(rows[0]?.name).toBe("landforms");
    expect(rows[1]?.name).toBe("word0");
  });
});
