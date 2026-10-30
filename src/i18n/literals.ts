import ts from "typescript";

/**
 * Finds user-visible English written straight into a component: JSX
 * text, and the string literals of the attributes a person reads or hears —
 * `title`, `placeholder`, `aria-label`, `alt`, `label`. `i18n:missing` only
 * compares the catalogues' keys, so text that never reached a catalogue was
 * invisible to it, and a German user met "Find a keyword…" in the default
 * rail section.
 *
 * Text counts when it has two words of two or more Latin letters, or one word
 * of four or more — "OK" and "8-bit" pass, "Bit depth" and "Keywords" don't.
 * A literal that must stay as written (a product name, a format, an example
 * filename) is allowed by putting `{/* i18n-ok *\/}` or `// i18n-ok` on the
 * line before it.
 */

export interface Literal {
  file: string;
  line: number;
  text: string;
}

const READ_ATTRS = new Set(["title", "placeholder", "aria-label", "alt", "label", "aria-description"]);

/** Whether `text` reads as words a person would expect in their language. */
export function isWords(text: string): boolean {
  const words = text.match(/[A-Za-z]{2,}/g) ?? [];
  return words.length >= 2 || words.some((w) => w.length >= 4);
}

export function literalsIn(file: string, source: string): Literal[] {
  const sf = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const lines = source.split(/\r?\n/);
  const allowed = (pos: number) => {
    const line = sf.getLineAndCharacterOfPosition(pos).line;
    return [lines[line], lines[line - 1]].some((l) => l?.includes("i18n-ok"));
  };
  const out: Literal[] = [];
  const add = (node: ts.Node, text: string) => {
    const clean = text.replace(/\s+/g, " ").trim();
    if (!isWords(clean) || isKey(clean) || allowed(node.getStart(sf))) return;
    out.push({ file, line: sf.getLineAndCharacterOfPosition(node.getStart(sf)).line + 1, text: clean });
  };
  /** The text an expression can show: its literals, through `a ? "x" : "y"`,
   *  `cond && "x"`, `a ?? "x"` and template literals — not a call's arguments
   *  (`t("key")`), a comparison's operands or anything else computed. */
  const shown = (e: ts.Expression): ts.Node[] => {
    if (ts.isStringLiteral(e) || ts.isNoSubstitutionTemplateLiteral(e) || ts.isTemplateExpression(e)) return [e];
    if (ts.isParenthesizedExpression(e)) return shown(e.expression);
    if (ts.isConditionalExpression(e)) return [...shown(e.whenTrue), ...shown(e.whenFalse)];
    if (ts.isBinaryExpression(e)) {
      const op = e.operatorToken.kind;
      if (op === ts.SyntaxKind.AmpersandAmpersandToken) return shown(e.right);
      if (op === ts.SyntaxKind.BarBarToken || op === ts.SyntaxKind.QuestionQuestionToken)
        return [...shown(e.left), ...shown(e.right)];
      if (op === ts.SyntaxKind.PlusToken) return [...shown(e.left), ...shown(e.right)];
    }
    return [];
  };
  const textOf = (n: ts.Node) =>
    ts.isTemplateExpression(n)
      ? [n.head.text, ...n.templateSpans.map((s) => s.literal.text)].join(" ")
      : (n as ts.StringLiteral).text;
  const addExpression = (e: ts.Expression) => {
    for (const n of shown(e)) add(n, textOf(n));
  };
  // Keys passed to a component that translates them (`label="develop.tool.crop"`).
  const isKey = (s: string) => /^[a-z]+(\.[A-Za-z0-9]+)+$/.test(s);
  const visit = (node: ts.Node) => {
    if (ts.isJsxText(node)) add(node, node.getText(sf));
    else if (ts.isJsxAttribute(node) && READ_ATTRS.has(node.name.getText(sf)) && node.initializer) {
      const init = node.initializer;
      if (ts.isStringLiteral(init)) {
        if (!isKey(init.text)) add(init, init.text);
      } else if (ts.isJsxExpression(init) && init.expression) addExpression(init.expression);
    } else if (ts.isJsxExpression(node) && node.expression && node.parent && ts.isJsxElement(node.parent)) {
      // {"Some words"}, {`Some ${x} words`} or {a ? "x" : "y"} as a child.
      addExpression(node.expression);
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
  return out;
}
