import { readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

// Read from disk rather than importing: Vitest stubs CSS imports, and even
// `?raw` comes back empty. Vitest runs in Node, so this is the reliable route
// to the actual stylesheet text.
const css = readFileSync(resolve(process.cwd(), "src/styles.css"), "utf8");

/**
 * Contrast gate for the neutral text ramp.
 *
 * `--color-faint` was #66676e — 3.18:1 on panel, below the 4.5:1 AA floor — and
 * it was the most-used text colour in the app (151 uses, more than ink), almost
 * always at 10–11px. It got there because the ramp had a hole: muted at 6.5:1
 * and faint at 3.2:1 with nothing between, so "quieter than muted" meant
 * "fails". This pins the fix so the ramp cannot drift dark again.
 *
 * Values are parsed out of styles.css rather than duplicated here, so the test
 * fails if the stylesheet changes rather than passing against a stale copy.
 */

function token(name: string): string {
  const m = css.match(new RegExp(`--color-${name}:\\s*(#[0-9a-fA-F]{6})\\s*;`));
  if (!m?.[1]) throw new Error(`--color-${name} not found in styles.css, or is not a plain hex`);
  return m[1];
}

/** WCAG 2.1 relative luminance. */
function luminance(hex: string): number {
  const n = parseInt(hex.slice(1), 16);
  const channel = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  return (
    0.2126 * channel((n >> 16) & 255) +
    0.7152 * channel((n >> 8) & 255) +
    0.0722 * channel(n & 255)
  );
}

function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)];
  const [hi, lo] = x > y ? [x, y] : [y, x];
  return (hi + 0.05) / (lo + 0.05);
}

/** Every surface token the app paints text on (hover was missing). */
const SURFACES = ["base", "panel", "raised", "hover"] as const;

/** `a` laid over `b` at `alpha`, as a CSS colour-mix would. */
function mix(a: string, b: string, alpha: number): string {
  const ch = (hex: string, shift: number) => (parseInt(hex.slice(1), 16) >> shift) & 255;
  const out = [16, 8, 0].map((s) => Math.round(ch(a, s) * alpha + ch(b, s) * (1 - alpha)));
  return "#" + out.map((v) => v.toString(16).padStart(2, "0")).join("");
}

/** The fills of states, which text is read on too: the active segment,
 *  and an active facet row (accent-dim at 25% over the panel). */
function stateFills(): [string, string][] {
  const segOn = css.match(/--seg-on-fill:\s*(#[0-9a-fA-F]{6})\s*;/)?.[1];
  if (!segOn) throw new Error("--seg-on-fill not found, or not a plain hex");
  return [
    ["seg-on fill", segOn],
    ["active facet row", mix(token("accent-dim"), token("panel"), 0.25)],
  ];
}

/** Tiers that carry text, and so must clear AA at the small sizes this UI uses. */
const TEXT_TIERS = ["ink", "muted", "faint", "accent"] as const;

describe("neutral text ramp", () => {
  it.each(TEXT_TIERS)("%s clears WCAG AA on every surface", (tier) => {
    for (const surface of SURFACES) {
      const ratio = contrast(token(tier), token(surface));
      expect(
        ratio,
        `--color-${tier} on --color-${surface} is ${ratio.toFixed(2)}:1, below the 4.5:1 AA floor`,
      ).toBeGreaterThanOrEqual(4.5);
    }
    for (const [name, fill] of stateFills()) {
      const ratio = contrast(token(tier), fill);
      expect(ratio, `--color-${tier} on the ${name} is ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("danger text clears AA where it is set: on raised, hover and its own wash", () => {
    const ink = token("danger-ink");
    const wash = mix(token("danger"), token("panel"), 0.1);
    for (const [name, fill] of [
      ["raised", token("raised")],
      ["hover", token("hover")],
      ["the danger wash", wash],
    ] as const) {
      const ratio = contrast(ink, fill);
      expect(ratio, `danger-ink on ${name} is ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("a focused text field's border is a 3:1 change", () => {
    // WCAG 1.4.11: the focus indicator against the field and the panel.
    expect(contrast(token("muted"), token("base"))).toBeGreaterThanOrEqual(3);
    expect(contrast(token("muted"), token("panel"))).toBeGreaterThanOrEqual(3);
  });

  it("keeps the tiers far enough apart to read as a hierarchy", () => {
    // Raising faint to pass AA squeezes it toward muted. Below roughly 1.3x the
    // two stop reading as distinct tiers and the ramp may as well be one step.
    const ratio = luminance(token("muted")) / luminance(token("faint"));
    expect(ratio).toBeGreaterThan(1.3);
    expect(luminance(token("ink")) / luminance(token("muted"))).toBeGreaterThan(1.3);
  });

  it("keeps a decorative step below the text floor", () => {
    // `dim` exists so that "quieter than faint" has somewhere to go that is not
    // a failing text colour. It is deliberately below AA and must stay non-text.
    const onPanel = contrast(token("dim"), token("panel"));
    expect(onPanel).toBeLessThan(4.5);
    expect(luminance(token("dim"))).toBeLessThan(luminance(token("faint")));
  });
});

/**
 * `text-<name>` is ambiguous in Tailwind v4: it means a font size if `--text-<name>`
 * exists, and a colour if `--color-<name>` does. Where both could apply, the
 * colour wins.
 *
 * `--color-base` collides with Tailwind's built-in `text-base` size. So five
 * dialog headings written as `text-base` silently got `color: #101013` — the
 * app's darkest background — and no font size at all, which is why the Settings
 * and Shortcuts modal titles rendered as dark-on-dark and clipped. The one
 * dialog that also set `text-ink` escaped, purely because `.text-ink` happens to
 * sort after `.text-base`.
 *
 * This guards the whole class of collision, not just the one instance.
 */
describe("colour tokens that collide with Tailwind size names", () => {
  /** Tailwind v4's built-in font-size scale. */
  const SIZE_NAMES = ["xs", "sm", "base", "lg", "xl", "2xl", "3xl", "4xl", "5xl"];

  const colourTokens = [...css.matchAll(/--color-([a-z0-9-]+):/g)].map((m) => m[1]!);
  const collisions = SIZE_NAMES.filter((n) => colourTokens.includes(n));

  function sources(dir: string): string[] {
    return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
      const full = resolve(dir, e.name);
      if (e.isDirectory()) return sources(full);
      return /\.tsx?$/.test(e.name) && !e.name.includes(".test.") ? [full] : [];
    });
  }

  it("is a known, non-empty set — base collides with Tailwind's text-base", () => {
    expect(collisions).toContain("base");
  });

  /**
   * Whole-class-token match. Deliberately not `\b`: a hyphen is a non-word
   * character, so word boundaries around these names are easy to get subtly
   * wrong — the first version of this test used one and silently matched
   * nothing, which is a worse failure than the bug it was guarding.
   */
  const usesClass = (src: string, cls: string) =>
    src.split(/[\s"'`{}]+/).includes(cls);

  it.each(collisions)("no component uses the ambiguous `text-%s`", (name) => {
    const offenders = sources(resolve(process.cwd(), "src")).filter((f) =>
      usesClass(readFileSync(f, "utf8"), `text-${name}`),
    );
    expect(
      offenders,
      `text-${name} resolves to --color-${name}, not a font size. Use a --text-* scale class and set the colour explicitly.`,
    ).toEqual([]);
  });
});

/** The components' sources, by path from src/components, Develop's in dev/ too. */
function componentFiles(): string[] {
  return ["", "dev/"].flatMap((sub) =>
    readdirSync(resolve(process.cwd(), "src/components", sub))
      .filter((f) => f.endsWith(".tsx") && !f.endsWith(".test.tsx"))
      .map((f) => sub + f),
  );
}

const componentSource = (f: string) => readFileSync(resolve(process.cwd(), "src/components", f), "utf8");

/**
 * A floating panel capped at a fixed pixel height overflows any window shorter
 * than that plus its scrim padding — and the app's minimum window is 620px
 * tall. Settings (720px) did exactly that: at the minimum size its title and
 * its Done button were both off-screen, with nothing to scroll them back.
 * A cap must also be bounded by the viewport:
 * `max-h-[min(720px,calc(100vh-4rem))]`.
 */
describe("floating panels fit short windows", () => {
  it("never caps a panel at a bare pixel height", () => {
    const offenders = componentFiles().flatMap((f) =>
      [...componentSource(f).matchAll(/max-h-\[\d+px\]/g)].map((m) => `${f}: ${m[0]}`),
    );
    expect(offenders).toEqual([]);
  });

  /**
   * No cap at all is the same bug: the export dialog had none, and at the
   * minimum window its Export button sat below the bottom edge. Every
   * centred modal must bound its height by the viewport.
   */
  it("bounds every centred modal by the window", () => {
    const modals = componentFiles().filter((f) => /fixed inset-0[^"]*place-items-center/.test(componentSource(f)));
    expect(modals).toContain("ExportDialog.tsx");
    expect(modals).toContain("dev/GroupsDialog.tsx");
    const unbounded = modals.filter((f) => !/max-h-\[[^\]]*vh[^\]]*\]/.test(componentSource(f)));
    expect(unbounded).toEqual([]);
  });
});

/**
 * Tracked uppercase micro-labels ("FILTERS", "DATE", "SHADOWS") on every group
 * were the most repeated template tell in the app, and the rework retired them
 * for sentence case (`.section-label`). Nothing may bring the tracking back.
 * Plain `uppercase` is still fine for real acronyms, like the export formats.
 */
describe("labels are sentence case", () => {
  it("no component tracks out uppercase lettering", () => {
    const offenders = componentFiles().filter((f) => /\btracking-(wide|wider|widest)\b/.test(componentSource(f)));
    expect(offenders).toEqual([]);
  });
});

/**
 * A sparkle glyph standing in for "AI" was on a dozen buttons, and it was the
 * most generic mark in the app. The glyph is gone from the icon set; this
 * keeps a hand-drawn one from coming back under the old name.
 */
describe("no sparkle marker", () => {
  it("no source file refers to a sparkles icon", () => {
    const dir = resolve(process.cwd(), "src");
    const walk = (d: string): string[] =>
      readdirSync(d, { withFileTypes: true }).flatMap((e) =>
        e.isDirectory()
          ? walk(resolve(d, e.name))
          : /\.tsx?$/.test(e.name) && !e.name.includes(".test.")
            ? [resolve(d, e.name)]
            : [],
      );
    const offenders = walk(dir).filter((f) => readFileSync(f, "utf8").includes('"sparkles"'));
    expect(offenders).toEqual([]);
  });
});
