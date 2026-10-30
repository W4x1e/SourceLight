import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it } from "vitest";
import { en } from "../i18n/en";
import { setLocale } from "../i18n";
import {
  conditionName,
  lookBlurb,
  lookFamilyName,
  lookMatches,
  lookTitle,
  maskLabel,
  slug,
  presetDescription,
  presetGroupName,
  presetMatches,
  presetName,
} from "./content";

afterEach(() => setLocale("en"));

const rust = (p: string) => readFileSync(`src-tauri/src/${p}`, "utf8");
/** The edit model moved to the crate the phone app shares (src-tauri/crates/sl-develop). */
const develop = (p: string) => readFileSync(`src-tauri/crates/sl-develop/src/develop/${p}`, "utf8");

const lookFiles = [...develop("lut_packs.rs").matchAll(/file: "([^"]+\.cube)"/g)].map((m) => m[1]!);
const conditions = [...rust("tagger/builtin.rs").match(/CONDITIONS: &\[&str\] = &\[([^\]]*)\]/)![1]!.matchAll(/"([^"]+)"/g)].map(
  (m) => m[1]!,
);
const families = [...develop("lut_packs.rs").matchAll(/Self::\w+ => "(\w+)"/g)].map((m) => m[1]!);
/** Built-in presets and their groups, each as [id, English name], from `presets.json`. */
const pack = JSON.parse(develop("presets.json")) as {
  groups: { id: string; name: string; presets: { id: string; name: string }[] }[];
};
const groupEntries = pack.groups.map((g) => [g.id, g.name] as const);
const presetEntries = pack.groups.flatMap((g) => g.presets.map((p) => [p.id, p.name] as const));

describe("the core still has what the catalogue names", () => {
  it("has every look file, preset, family, mask label and condition", () => {
    expect(lookFiles.length).toBeGreaterThan(10);
    for (const f of lookFiles) {
      expect(en, f).toHaveProperty([`content.look.${slug(f)}.title`]);
      expect(en, f).toHaveProperty([`content.look.${slug(f)}.blurb`]);
    }
    for (const f of families) expect(en, f).toHaveProperty([`content.lookFamily.${f}`]);
    expect(presetEntries.length).toBeGreaterThan(60);
    expect(groupEntries.length).toBeGreaterThan(8);
    for (const [id] of presetEntries) {
      expect(en, id).toHaveProperty([`content.preset.${slug(id)}.name`]);
      expect(en, id).toHaveProperty([`content.preset.${slug(id)}.description`]);
    }
    for (const [id] of groupEntries) {
      expect(en, id).toHaveProperty([`content.presetGroup.${slug(id)}.name`]);
      expect(en, id).toHaveProperty([`content.presetGroup.${slug(id)}.description`]);
    }
    for (const c of conditions) expect(en, c).toHaveProperty([`content.condition.${slug(c)}`]);
    const mask = develop("mask.rs");
    expect(mask).toContain('label: "Subject".into()');
    expect(mask).toContain('label: "Sky".into()');
  });

  it("has no catalogue entry for something the core dropped", () => {
    const slugs = (k: string) => new Set(Object.keys(en).filter((x) => x.startsWith(k)).map((x) => x.slice(k.length).split(".")[0]));
    expect(slugs("content.look.")).toEqual(new Set(lookFiles.map(slug)));
    expect(slugs("content.preset.")).toEqual(new Set(presetEntries.map(([id]) => slug(id))));
    expect(slugs("content.presetGroup.")).toEqual(new Set(groupEntries.map(([id]) => slug(id))));
    expect(slugs("content.condition.")).toEqual(new Set(conditions.map(slug)));
  });

  it("matches the core's English exactly", () => {
    const lut = develop("lut_packs.rs");
    for (const f of lookFiles) {
      const at = lut.indexOf(`file: "${f}"`);
      const title = /title: "([^"]+)"/.exec(lut.slice(at))![1];
      expect(en[`content.look.${slug(f)}.title` as keyof typeof en], f).toBe(title);
    }
    for (const c of conditions) expect(en[`content.condition.${slug(c)}` as keyof typeof en]).toBe(c);
    for (const [id, name] of presetEntries) expect(en[`content.preset.${slug(id)}.name` as keyof typeof en], id).toBe(name);
    for (const [id, name] of groupEntries)
      expect(en[`content.presetGroup.${slug(id)}.name` as keyof typeof en], id).toBe(name);
  });
});

describe("the helpers", () => {
  const built = { name: "portrait-negative.cube", title: "Portrait Negative", blurb: "Warm, forgiving skin and muted greens — the everyday portrait stock.", imported: false };

  it("translate a built-in look and keep an imported one", async () => {
    await setLocale("de");
    expect(lookTitle(built)).toBe("Porträt-Negativ");
    expect(lookBlurb(built)).toContain("Hauttöne");
    const mine = { ...built, imported: true, title: "Portrait Negative" };
    expect(lookTitle(mine)).toBe("Portrait Negative");
    expect(lookTitle({ ...built, name: "unknown.cube", title: "Unknown" })).toBe("Unknown");
  });

  it("name families, unless the user made the group", async () => {
    await setLocale("fr");
    expect(lookFamilyName("Cinema")).toBe("Cinéma");
    expect(lookFamilyName("Cinema", true)).toBe("Cinema");
    expect(lookFamilyName("My Pack")).toBe("My Pack");
  });

  it("search matches the translated and the English words", async () => {
    await setLocale("de");
    expect(lookMatches(built, "porträt")).toBe(true);
    expect(lookMatches(built, "portrait")).toBe(true);
    expect(lookMatches(built, "cinema")).toBe(false);
    const preset = { id: "film-warm", name: "Warm", description: "Warm highlights, lifted blacks, grain and a little glow." };
    expect(presetName(preset.id, preset.name)).toBe("Warm");
    expect(presetGroupName("mono", "Black and White")).toBe("Schwarzweiß");
    expect(presetDescription("nope", "as sent")).toBe("as sent");
    expect(presetMatches(preset, "korn")).toBe(true);
    expect(presetMatches(preset, "lifted blacks")).toBe(true);
  });

  it("word the default mask labels only, when drawn", async () => {
    await setLocale("ja");
    expect(maskLabel("Subject")).toBe("被写体");
    expect(maskLabel("Sky")).toBe("空");
    expect(maskLabel("My sky")).toBe("My sky");
  });

  it("name a condition", async () => {
    await setLocale("es");
    expect(conditionName("golden hour")).toBe("hora dorada");
    expect(conditionName("mystery")).toBe("mystery");
  });
});
