import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { isWords, literalsIn } from "./literals";

/** Every component, Develop's pieces in `components/dev` included, and the
 *  shell. The developer-only tools live in `src/dev`, outside this walk. */
function sources(): string[] {
  const walk = (dir: string): string[] =>
    readdirSync(dir).flatMap((f) => {
      const p = join(dir, f);
      if (statSync(p).isDirectory()) return walk(p);
      return p.endsWith(".tsx") && !p.endsWith(".test.tsx") ? [p] : [];
    });
  return [...walk(join(__dirname, "..", "components")), join(__dirname, "..", "App.tsx")];
}

/**
 * No English written straight into a component. `i18n:missing` only
 * compares the catalogues, so text that never reached them was invisible to
 * it: a German user met "Find a keyword…" in the default rail section. Put
 * the words in the catalogues; a literal that must stay as written (a product
 * name, a format) is marked `i18n-ok` on its line or the line before.
 */
describe("text in components", () => {
  it("finds the components", () => {
    expect(sources().length).toBeGreaterThan(50);
  });

  it("is all in the catalogues", () => {
    const found = sources().flatMap((f) => literalsIn(f, readFileSync(f, "utf8")));
    expect(found.map((l) => `${l.file.split(/[\\/]/).pop()}:${l.line}  ${l.text}`)).toEqual([]);
    // Parsing every component takes seconds under a loaded full run.
  }, 60_000);
});

describe("literalsIn", () => {
  it("catches text, read attributes and the branches of a choice", () => {
    const src = `const A = () => (
      <div title="Hide the timeline">
        Photos with
        {on ? "Show less" : t("x")}
        {n === 1 ? "OK" : "8-bit"}
        <b aria-label={\`Rename \${x}\`}>{t("a.b")}</b>
        <i label="develop.tool.crop" />
      </div>
    );`;
    expect(literalsIn("a.tsx", src).map((l) => l.text)).toEqual([
      "Hide the timeline",
      "Photos with",
      "Show less",
      "Rename",
    ]);
  });

  it("lets a marked literal stay", () => {
    const src = `const A = () => (
      // i18n-ok
      <h1 title="SourceLight">x</h1>
    );`;
    expect(literalsIn("a.tsx", src)).toEqual([]);
  });

  it("reads words, not codes", () => {
    expect(isWords("Keywords")).toBe(true);
    expect(isWords("Bit depth")).toBe(true);
    expect(isWords("JPEG")).toBe(true);
    expect(isWords("8-bit")).toBe(false);
    expect(isWords("OK")).toBe(false);
  });
});
