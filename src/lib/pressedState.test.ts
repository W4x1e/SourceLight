import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import ts from "typescript-api";
import { describe, expect, it } from "vitest";

/**
 * A control that shows it is on only by its look tells a screen reader
 * nothing: Export's format buttons read "JPEG button, PNG button…"
 * with no word of which was chosen, and the rail said "Cull button" whether
 * or not you were in Cull. Anything drawn as the active segment, tool or mode
 * has to say so in ARIA as well.
 */

const ACTIVE = /\b(?:seg-on|tool-on|mode-on)\b/;
const STATE = new Set(["aria-pressed", "aria-checked", "aria-selected", "aria-current"]);

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return files(path);
    return path.endsWith(".tsx") && !path.endsWith(".test.tsx") ? [path] : [];
  });
}

describe("active-looking controls", () => {
  it("say they are active", { timeout: 60_000 }, () => {
    const offenders: string[] = [];
    for (const path of files(join(__dirname, ".."))) {
      const text = readFileSync(path, "utf8");
      if (!ACTIVE.test(text)) continue;
      const src = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
      const visit = (n: ts.Node) => {
        if (ts.isJsxOpeningElement(n) || ts.isJsxSelfClosingElement(n)) {
          let cls = "";
          let stated = false;
          for (const a of n.attributes.properties) {
            if (!ts.isJsxAttribute(a)) {
              stated = true; // a spread may carry it; trust it
              continue;
            }
            const name = a.name.getText();
            if (name === "className") cls = a.initializer?.getText() ?? "";
            if (STATE.has(name)) stated = true;
          }
          if (ACTIVE.test(cls) && !stated) {
            offenders.push(`${path}:${src.getLineAndCharacterOfPosition(n.getStart()).line + 1}`);
          }
        }
        ts.forEachChild(n, visit);
      };
      visit(src);
    }
    expect(offenders).toEqual([]);
  });

  // A star alone reads "black star, button" five times over: each needs a
  // name saying which rating it gives.
  it("stars have names", { timeout: 60_000 }, () => {
    const offenders: string[] = [];
    for (const path of files(join(__dirname, ".."))) {
      const text = readFileSync(path, "utf8");
      if (!text.includes("★")) continue;
      const src = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
      const visit = (n: ts.Node) => {
        if (ts.isJsxElement(n) && n.openingElement.tagName.getText() === "button") {
          const content = n.children.map((c) => c.getText()).join("").trim();
          const named = n.openingElement.attributes.properties.some(
            (a) => !ts.isJsxAttribute(a) || a.name.getText() === "aria-label",
          );
          if (content === "★" && !named) {
            offenders.push(`${path}:${src.getLineAndCharacterOfPosition(n.getStart()).line + 1}`);
          }
        }
        ts.forEachChild(n, visit);
      };
      visit(src);
    }
    expect(offenders).toEqual([]);
  });
});
