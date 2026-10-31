import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import ts from "typescript-api";
import { describe, expect, it } from "vitest";

/**
 * A control hidden with `opacity-0` until its row is hovered is invisible to
 * the keyboard: tabbing lands on it, focus ring and all, and nothing shows.
 * Mask and spot delete buttons were like that, so Enter on an unseen
 * focus deleted something. Anything clickable that waits for a hover has to
 * show on focus as well.
 */

const FOCUS = /(?:^|\s)(?:focus-visible|focus-within|group-focus-within|focus):opacity-100(?:\s|$)/;

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return files(path);
    return path.endsWith(".tsx") && !path.endsWith(".test.tsx") ? [path] : [];
  });
}

function classOf(el: ts.JsxOpeningLikeElement): string {
  for (const a of el.attributes.properties) {
    if (!ts.isJsxAttribute(a) || a.name.getText() !== "className" || !a.initializer) continue;
    // A literal, or the literal parts of a template: enough to see the classes.
    return a.initializer.getText();
  }
  return "";
}

/** Whether `node` holds a button or another control reached by Tab. */
function interactive(node: ts.Node): boolean {
  let found = false;
  const visit = (n: ts.Node) => {
    if (found) return;
    if (ts.isJsxOpeningElement(n) || ts.isJsxSelfClosingElement(n)) {
      const tag = n.tagName.getText();
      if (tag === "button" || tag === "a" || tag === "input" || tag === "select") found = true;
    }
    ts.forEachChild(n, visit);
  };
  visit(node);
  return found;
}

describe("hover-only controls", () => {
  it("show on keyboard focus too", { timeout: 60_000 }, () => {
    const offenders: string[] = [];
    for (const path of files(join(__dirname, ".."))) {
      const text = readFileSync(path, "utf8");
      if (!text.includes("group-hover:opacity-100")) continue;
      const src = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
      const visit = (n: ts.Node) => {
        const opening = ts.isJsxElement(n) ? n.openingElement : ts.isJsxSelfClosingElement(n) ? n : null;
        if (opening) {
          const cls = classOf(opening);
          if (/\bopacity-0\b/.test(cls) && cls.includes("group-hover:opacity-100") && !FOCUS.test(cls.replace(/["'`]/g, " ")) && interactive(n)) {
            const line = src.getLineAndCharacterOfPosition(opening.getStart()).line + 1;
            offenders.push(`${path}:${line}`);
          }
        }
        ts.forEachChild(n, visit);
      };
      visit(src);
    }
    expect(offenders).toEqual([]);
  });
});
