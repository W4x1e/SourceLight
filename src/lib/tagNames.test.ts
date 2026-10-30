import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  api: {
    tagTranslations: vi.fn(async () => ({ "old tag": "alter Tag" })),
    translateTags: vi.fn(async (_: string, names: string[]) =>
      Object.fromEntries(names.map((n) => [n, `DE:${n}`])),
    ),
  },
}));

import { api } from "./api";
import { learnTagNames } from "./tagNames";
import { setLocale } from "../i18n";
import { tagName } from "../i18n/tags";
import type { Facets } from "../bindings";

const facets = (subjects: string[], conditions: string[] = []) =>
  ({
    subjects: subjects.map((value) => ({ value, count: 1 })),
    conditions: conditions.map((value) => ({ value, count: 1 })),
  }) as unknown as Facets;

describe("learning names for free-text tags", () => {
  beforeEach(() => vi.clearAllMocks());
  afterEach(async () => {
    await setLocale("en");
  });

  it("asks only for the tags no table names, and shows the answers", async () => {
    await setLocale("de");
    await learnTagNames(facets(["dog", "old tag", "long-tailed tit on a frosted hawthorn"], ["golden hour", "misty glow"]), true);
    expect(api.translateTags).toHaveBeenCalledTimes(1);
    // "dog" is in the German table and "golden hour" in the conditions table;
    // "old tag" was already named.
    expect(vi.mocked(api.translateTags).mock.calls[0]).toEqual([
      "de",
      ["long-tailed tit on a frosted hawthorn", "misty glow"],
    ]);
    expect(tagName("long-tailed tit on a frosted hawthorn")).toBe("DE:long-tailed tit on a frosted hawthorn");
    expect(tagName("old tag")).toBe("alter Tag");
    expect(tagName("dog")).not.toBe("DE:dog");
  });

  it("does nothing in English", async () => {
    await learnTagNames(facets(["misty glow"]), true);
    expect(api.tagTranslations).not.toHaveBeenCalled();
    expect(tagName("misty glow")).toBe("misty glow");
  });

  it("asks once per language unless a pass has just tagged", async () => {
    await setLocale("fr");
    await learnTagNames(facets(["misty glow"]));
    await learnTagNames(facets(["misty glow"]));
    expect(api.tagTranslations).toHaveBeenCalledTimes(1);
    await learnTagNames(facets(["misty glow"]), true);
    expect(api.tagTranslations).toHaveBeenCalledTimes(2);
  });

  it("shows a tag in English when no model named it", async () => {
    vi.mocked(api.translateTags).mockResolvedValueOnce({});
    await setLocale("es");
    await learnTagNames(facets(["misty glow"]), true);
    expect(tagName("misty glow")).toBe("misty glow");
  });
});
