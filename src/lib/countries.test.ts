import { afterEach, describe, expect, it } from "vitest";
import { setLocale } from "../i18n";
import { countryName, localPlaceName } from "./countries";
import { COUNTRY_CODES } from "./countryCodes";

afterEach(() => setLocale("en"));

describe("countryName", () => {
  it("is the English name in English", async () => {
    await setLocale("en");
    expect(countryName("Germany")).toBe("Germany");
  });

  it("names a country in the interface language", async () => {
    await setLocale("de");
    expect(countryName("Germany")).toBe("Deutschland");
    expect(countryName("United Kingdom")).toBe("Vereinigtes Königreich");
    await setLocale("ja");
    expect(countryName("Japan")).toBe("日本");
    expect(countryName("United Kingdom")).toBe("イギリス");
  });

  it("falls back to what it was given", async () => {
    await setLocale("de");
    expect(countryName("Atlantis")).toBe("Atlantis");
  });

  it("covers the core's whole country table", () => {
    expect(Object.keys(COUNTRY_CODES).length).toBeGreaterThan(240);
    expect(COUNTRY_CODES["United Kingdom"]).toBe("GB");
  });
});

describe("localPlaceName", () => {
  it("swaps a trailing country", async () => {
    await setLocale("de");
    expect(localPlaceName("Edinburgh, United Kingdom", "United Kingdom")).toBe("Edinburgh, Vereinigtes Königreich");
    expect(localPlaceName("Edinburgh, United Kingdom")).toBe("Edinburgh, Vereinigtes Königreich");
  });

  it("leaves a name that does not end in a country", async () => {
    await setLocale("ja");
    expect(localPlaceName("Granny's cottage")).toBe("Granny's cottage");
    expect(localPlaceName("Paris, Texas", "France")).toBe("Paris, Texas");
  });
});
