import { afterEach, describe, expect, it } from "vitest";
import { isValidElement } from "react";
import { formatNumber, getLocale, inSentence, resolveLocale, setLocale, t, tx } from ".";
import { pseudo } from "./pseudo";

afterEach(async () => {
  await setLocale("en");
});

describe("t", () => {
  it("returns the English string when no language is set", () => {
    expect(getLocale()).toBe("en");
    expect(t("common.cancel")).toBe("Cancel");
  });

  it("fills placeholders and formats numbers for the language", () => {
    expect(t("common.progress", { current: 1200, total: 30000 })).toBe("1,200 / 30,000");
  });

  it("leaves a string parameter as given", () => {
    expect(t("common.showInFileManager", { fileManager: "Finder" })).toBe("Show in Finder");
  });

  it("picks the plural form from count", () => {
    expect(t("common.photos", { count: 1 })).toBe("1 photo");
    expect(t("common.photos", { count: 0 })).toBe("0 photos");
    expect(t("common.photos", { count: 2500 })).toBe("2,500 photos");
  });

  it("formats numbers in the active language", async () => {
    await setLocale("de");
    expect(formatNumber(1234567)).toBe("1.234.567");
    expect(document.documentElement.lang).toBe("de");
  });

  it("falls back to English for a key the language lacks", async () => {
    await setLocale("de");
    // Whatever German has, no key ever comes back as its own name.
    expect(t("common.untitled")).not.toBe("common.untitled");
  });

  it("formats in the pseudo-locale with placeholders intact", async () => {
    await setLocale("en-XA");
    const s = t("common.photos", { count: 3 });
    expect(s.startsWith("[")).toBe(true);
    expect(s).toContain("3");
    expect(s).not.toContain("photos");
  });
});

describe("tx", () => {
  it("keeps elements in the sentence, in order", () => {
    const out = tx("common.selected", { count: <b>7</b> }) as unknown[];
    expect(out).toHaveLength(2);
    expect(isValidElement(out[0])).toBe(true);
    expect(out[1]).toBe(" selected");
  });

  it("uses the explicit count for the plural form", () => {
    const one = tx("common.photos", { count: <b>1</b> }, 1) as unknown[];
    expect(one[1]).toBe(" photo");
    const many = tx("common.photos", { count: <b>5</b> }, 5) as unknown[];
    expect(many[1]).toBe(" photos");
  });
});

describe("resolveLocale", () => {
  it("honours an explicit choice", () => {
    expect(resolveLocale("fr", ["de-DE"])).toBe("fr");
  });
  it("matches the system's language, ignoring region", () => {
    expect(resolveLocale("system", ["de-CH", "en-GB"])).toBe("de");
    expect(resolveLocale("system", ["ja"])).toBe("ja");
  });
  it("takes the first language it ships", () => {
    expect(resolveLocale("system", ["nl-NL", "es-MX"])).toBe("es");
  });
  it("falls back to English", () => {
    expect(resolveLocale("system", ["nl-NL"])).toBe("en");
    expect(resolveLocale("system", [])).toBe("en");
  });
});

describe("pseudo", () => {
  it("accents letters but not placeholders", () => {
    expect(pseudo("Hi {name}")).toBe("[Ĥî {name}···]");
  });
});

describe("inSentence", () => {
  it("lowers the first letter in English", () => {
    expect(inSentence("White balance")).toBe("white balance");
  });
  it("leaves an acronym alone", () => {
    expect(inSentence("HSL")).toBe("HSL");
  });
  it("keeps German nouns capitalised", async () => {
    await setLocale("de");
    expect(inSentence("Weißabgleich")).toBe("Weißabgleich");
  });
});
