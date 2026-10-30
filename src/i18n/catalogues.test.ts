import { describe, expect, it } from "vitest";
import { AREAS, en } from "./en";
import type { Message } from ".";
import de from "./de";
import es from "./es";
import fr from "./fr";
import ja from "./ja";

/**
 * The checks a translator's typo would otherwise only surface on screen: a
 * placeholder renamed or dropped, a plural with no `other`, a key English
 * does not have. The type system catches the last one too, but only in the
 * files `tsc` sees — this catches it however the catalogue was built.
 */

const LANGUAGES = { de, es, fr, ja } as const;

function placeholders(msg: Message): string[] {
  const forms = typeof msg === "string" ? [msg] : Object.values(msg);
  return [...new Set(forms.flatMap((m) => [...m.matchAll(/\{(\w+)\}/g)].map((x) => x[1]!)))].sort();
}

describe("English", () => {
  it("has no key in two areas", () => {
    const total = Object.values(AREAS).reduce<number>((n, a) => n + Object.keys(a).length, 0);
    expect(Object.keys(en).length).toBe(total);
  });

  it("keys every string by its area", () => {
    for (const [area, strings] of Object.entries(AREAS)) {
      for (const key of Object.keys(strings)) expect(key.startsWith(`${area}.`), key).toBe(true);
    }
  });

  it("gives every plural an `other`", () => {
    for (const [key, msg] of Object.entries(en) as [string, Message][]) {
      if (typeof msg !== "string") expect(msg.other, key).toBeTypeOf("string");
    }
  });

  it("has no empty strings", () => {
    for (const [key, msg] of Object.entries(en) as [string, Message][]) {
      const forms = typeof msg === "string" ? [msg] : Object.values(msg);
      for (const f of forms) expect(f.length, key).toBeGreaterThan(0);
    }
  });
});

describe.each(Object.entries(LANGUAGES))("%s", (_, catalogue) => {
  const entries = Object.entries(catalogue) as [string, Message][];

  it("only has keys English has", () => {
    for (const [key] of entries) expect(key in en, key).toBe(true);
  });

  it("keeps each message's placeholders", () => {
    for (const [key, msg] of entries) {
      const source = (en as Record<string, Message>)[key];
      if (source) expect(placeholders(msg), key).toEqual(placeholders(source));
    }
  });

  it("gives every plural an `other`, and only plurals get forms", () => {
    for (const [key, msg] of entries) {
      const source = (en as Record<string, Message>)[key];
      expect(typeof msg, key).toBe(typeof source);
      if (typeof msg !== "string") expect(msg.other, key).toBeTypeOf("string");
    }
  });
});
