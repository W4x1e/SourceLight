import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { setLocale } from "..";
import { tagFromLocal, tagName, type TagNames } from ".";
import de from "./de";
import es from "./es";
import fr from "./fr";
import ja from "./ja";

/**
 * The tables are keyed by the tagger's own English words, so a word renamed
 * or dropped in the core would leave a translation that never shows. This
 * reads the vocabulary from where the core reads it.
 */
function vocabulary(): Set<string> {
  const root = resolve(__dirname, "../../..");
  const subjects = readFileSync(resolve(root, "src-tauri/resources/tagger/subjects.txt"), "utf8")
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith("#"))
    // A `[group]` is named without its brackets, as the core reads it.
    .map((l) => l.split(">")[0]!.trim().replace(/^\[(.*)\]$/, "$1"));
  const models = readFileSync(resolve(root, "src-tauri/src/models.rs"), "utf8");
  const genres = /pub const GENRES: &\[&str\] = &\[([^\]]*)\]/.exec(models)?.[1] ?? "";
  return new Set([...subjects, ...[...genres.matchAll(/"([^"]+)"/g)].map((m) => m[1]!)]);
}

const words = vocabulary();

describe.each(Object.entries({ de, es, fr, ja } as Record<string, TagNames>))("%s tag names", (_, table) => {
  it("only names words the tagger has", () => {
    for (const key of Object.keys(table)) expect(words.has(key), key).toBe(true);
  });

  it("has no empty names", () => {
    for (const [key, name] of Object.entries(table)) expect(name.trim().length, key).toBeGreaterThan(0);
  });
});

describe("tagName", () => {
  afterEach(async () => {
    await setLocale("en");
  });

  it("shows English tags as they are", () => {
    expect(tagName("dog")).toBe("dog");
  });

  it("names a tag in the active language, and reads it back", async () => {
    await setLocale("de");
    const name = tagName("dog");
    if (de["dog"]) {
      expect(name).toBe(de["dog"]);
      expect(tagFromLocal(name.toUpperCase())).toBe("dog");
    }
    expect(tagName("a tag nobody knows")).toBe("a tag nobody knows");
  });
});
