import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import ts from "typescript-api";
import { describe, expect, it } from "vitest";

/**
 * React needs a component to call the same hooks in the same order on every
 * render. A hook after an early `return` (or in the right side of `&&`) runs on
 * some renders only, and React takes the whole interface down with "Rendered
 * more hooks than during the previous render" — the Activity panel did, the
 * moment it opened. There is no ESLint here to catch it, so this does, for the
 * two shapes it has taken.
 */

const isHookCall = (n: ts.Node): n is ts.CallExpression =>
  ts.isCallExpression(n) && ts.isIdentifier(n.expression) && /^use[A-Z]/.test(n.expression.text);

/** Hook calls in `node`, not counting nested functions (callbacks may call
 *  anything, and a nested component is checked on its own). */
function hooksIn(node: ts.Node): ts.CallExpression[] {
  const out: ts.CallExpression[] = [];
  const visit = (n: ts.Node) => {
    if (n !== node && (ts.isFunctionLike(n) || ts.isClassLike(n))) return;
    if (isHookCall(n)) out.push(n);
    ts.forEachChild(n, visit);
  };
  visit(node);
  return out;
}

/** Whether `node` can return from the function it is in. */
function returns(node: ts.Node): boolean {
  let found = false;
  const visit = (n: ts.Node) => {
    if (found || (n !== node && ts.isFunctionLike(n))) return;
    if (ts.isReturnStatement(n)) found = true;
    else ts.forEachChild(n, visit);
  };
  visit(node);
  return found;
}

export function hookOrderProblems(file: string, source: string): string[] {
  const sf = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const problems: string[] = [];
  const where = (n: ts.Node) => `${file.split(/[\\/]/).pop()}:${sf.getLineAndCharacterOfPosition(n.getStart(sf)).line + 1}`;
  const check = (name: string, body: ts.Block) => {
    let returned = false;
    for (const st of body.statements) {
      if (returned) for (const h of hooksIn(st)) problems.push(`${where(h)} ${name}: ${h.expression.getText(sf)} after an early return`);
      // `cond && useX()`: a hook on one side of a short circuit.
      const visit = (n: ts.Node) => {
        if (ts.isFunctionLike(n)) return;
        if (
          ts.isBinaryExpression(n) &&
          [ts.SyntaxKind.AmpersandAmpersandToken, ts.SyntaxKind.BarBarToken, ts.SyntaxKind.QuestionQuestionToken].includes(
            n.operatorToken.kind,
          )
        ) {
          for (const h of hooksIn(n.right)) problems.push(`${where(h)} ${name}: ${h.expression.getText(sf)} called conditionally`);
        }
        ts.forEachChild(n, visit);
      };
      visit(st);
      if (!ts.isReturnStatement(st) && returns(st)) returned = true;
    }
  };
  const visit = (n: ts.Node) => {
    if (ts.isFunctionDeclaration(n) && n.name && n.body && /^(use)?[A-Z]/.test(n.name.text)) check(n.name.text, n.body);
    if (
      ts.isVariableDeclaration(n) &&
      ts.isIdentifier(n.name) &&
      /^[A-Z]|^use[A-Z]/.test(n.name.text) &&
      n.initializer &&
      (ts.isArrowFunction(n.initializer) || ts.isFunctionExpression(n.initializer)) &&
      ts.isBlock(n.initializer.body)
    ) {
      check(n.name.text, n.initializer.body);
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
  return problems;
}

function sources(): string[] {
  const walk = (dir: string): string[] =>
    readdirSync(dir).flatMap((f) => {
      const p = join(dir, f);
      if (statSync(p).isDirectory()) return walk(p);
      return /\.tsx?$/.test(p) && !/\.test\.tsx?$/.test(p) ? [p] : [];
    });
  return walk(join(__dirname, ".."));
}

describe("hooks", () => {
  it("are called in the same order on every render, in every component", () => {
    const found = sources().flatMap((f) => hookOrderProblems(f, readFileSync(f, "utf8")));
    expect(found).toEqual([]);
    // Parsing every source takes seconds under a loaded full run.
  }, 60_000);

  it("catches a hook after an early return, and one behind &&", () => {
    const src = `
      function Panel() {
        const a = useA();
        if (!a) return null;
        const b = useB();
        return b;
      }
      const Rail = () => {
        const c = useWidth() < 10 && useStore((s) => s.x);
        return c;
      };
      function Fine() {
        const d = useD();
        const cb = () => { if (d) return; useNotAHookCallHere(); };
        if (!d) return null;
        return cb;
      }`;
    expect(hookOrderProblems("x.tsx", src)).toEqual([
      "x.tsx:5 Panel: useB after an early return",
      "x.tsx:9 Rail: useStore called conditionally",
    ]);
  });
});
