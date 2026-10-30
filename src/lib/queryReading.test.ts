import { afterEach, describe, expect, it } from "vitest";
import { setLocale } from "../i18n";
import { applyReading, readQuery, withoutReading, type Part, type Vocabulary } from "./queryReading";
import { EMPTY_FILTER } from "../types";

// Sunday 27 September 2026.
const NOW = new Date(2026, 8, 27, 15, 0);

const VOCAB: Vocabulary = {
  people: [
    { id: 1, name: "Anna" },
    { id: 2, name: "Ben Okafor" },
    { id: 3, name: "Paris" },
    { id: 4, name: "June" },
    { id: 5, name: "Ada Lovelace" },
  ],
  places: [
    { id: 10, name: "Edinburgh, United Kingdom", country: "United Kingdom" },
    { id: 11, name: "London, United Kingdom", country: "United Kingdom" },
    { id: 12, name: "Paris, France", country: "France" },
    { id: 13, name: "Cape Point", country: "South Africa" },
  ],
  genres: ["landscape", "portrait", "street", "still-life", "sports"],
  conditions: ["golden hour", "overcast", "night"],
  cameras: ["Sony ILCE-6400", "Nikon Z 6"],
  lenses: ["E 18-135mm F3.5-5.6 OSS", "NIKKOR Z 50mm f/1.8 S", "FE 50mm F1.8"],
};

const read = (q: string) => readQuery(q, VOCAB, NOW);
/** Each part as `kind:label`, for compact expectations. */
const labels = (q: string) => read(q).parts.map((p) => `${p.kind}:${p.label}`);
const dateOf = (q: string) => {
  const d = read(q).parts.find((p): p is Extract<Part, { kind: "date" }> => p.kind === "date");
  return d ? [d.from, d.to, d.label] : null;
};

describe("reading a search", () => {
  it("takes the issue's own example apart", () => {
    const r = read("Anna in Edinburgh last summer, sunset");
    expect(r.parts.map((p) => `${p.kind}:${p.label}`)).toEqual([
      "person:Anna",
      "place:Edinburgh",
      "date:Summer 2026",
    ]);
    expect(r.rest).toBe("sunset");
    expect(dateOf("last summer")).toEqual(["2026-06-01", "2026-08-31", "Summer 2026"]);
  });

  describe("people", () => {
    it("are matched whole-word and without regard to case", () => {
      expect(labels("ANNA")).toEqual(["person:Anna"]);
      expect(labels("annabel")).toEqual([]);
      expect(labels("ben okafor")).toEqual(["person:Ben Okafor"]);
    });

    it("answer to a first name nobody else has, and to a possessive", () => {
      expect(labels("Ada")).toEqual(["person:Ada Lovelace"]);
      const r = read("Anna's birthday");
      expect(r.parts.map((p) => p.label)).toEqual(["Anna"]);
      expect(r.rest).toBe("birthday");
    });

    it("together, leave only what the photo shows", () => {
      const r = read("sunset with Anna and Ben");
      expect(r.parts.map((p) => p.label)).toEqual(["Anna", "Ben Okafor"]);
      expect(r.rest).toBe("sunset");
    });
  });

  describe("places", () => {
    it("are matched by name, by the name before its comma, and by country", () => {
      expect(read("Edinburgh, United Kingdom").parts).toMatchObject([{ kind: "place", ids: [10] }]);
      expect(read("edinburgh").parts).toMatchObject([{ kind: "place", ids: [10], label: "Edinburgh" }]);
      expect(read("united kingdom").parts).toMatchObject([
        { kind: "place", ids: [10, 11], label: "United Kingdom" },
      ]);
    });

    it("come first when a person has the same name, with the person offered", () => {
      const r = read("Paris");
      expect(r.parts).toMatchObject([{ kind: "place", ids: [12] }]);
      expect(r.alternatives).toHaveLength(1);
      expect(r.alternatives[0]?.other).toMatchObject({ kind: "person", id: 3 });
    });
  });

  describe("dates", () => {
    it.each([
      ["2023", "2023-01-01", "2023-12-31", "2023"],
      ["2024-06", "2024-06-01", "2024-06-30", "June 2024"],
      ["2024-06-15", "2024-06-15", "2024-06-15", "2024-06-15"],
      ["2019-2021", "2019-01-01", "2021-12-31", "2019–2021"],
      ["june 2024", "2024-06-01", "2024-06-30", "June 2024"],
      ["Sept 2022", "2022-09-01", "2022-09-30", "September 2022"],
      ["may 2024", "2024-05-01", "2024-05-31", "May 2024"],
      ["summer 2023", "2023-06-01", "2023-08-31", "Summer 2023"],
      ["winter 2023", "2023-12-01", "2024-02-29", "Winter 2023"],
      ["autumn 2025", "2025-09-01", "2025-11-30", "Autumn 2025"],
      ["christmas 2023", "2023-12-24", "2023-12-26", "Christmas 2023"],
      ["last christmas", "2025-12-24", "2025-12-26", "Christmas 2025"],
      ["last winter", "2025-12-01", "2026-02-28", "Winter 2025"],
      ["this winter", "2026-12-01", "2027-02-28", "Winter 2026"],
      ["last month", "2026-08-01", "2026-08-31", "August 2026"],
      ["this year", "2026-01-01", "2026-12-31", "2026"],
      ["last year", "2025-01-01", "2025-12-31", "2025"],
      ["this week", "2026-09-21", "2026-09-27", "This week"],
      ["last week", "2026-09-14", "2026-09-20", "Last week"],
      ["yesterday", "2026-09-26", "2026-09-26", "Yesterday"],
      ["october", "2025-10-01", "2025-10-31", "October 2025"],
      ["march", "2026-03-01", "2026-03-31", "March 2026"],
    ])("%s", (q, from, to, label) => {
      expect(dateOf(q)).toEqual([from, to, label]);
      expect(read(q).rest).toBe("");
    });

    it("leave words that only look like them", () => {
      for (const q of ["may the fourth", "christmas tree", "summer dress", "the fall of rome", "dec"]) {
        expect(labels(q), q).toEqual([]);
      }
    });

    it("give way to a person of the same name when it is only a month", () => {
      const r = read("June");
      expect(r.parts).toMatchObject([{ kind: "person", id: 4 }]);
      expect(r.alternatives[0]?.other).toMatchObject({ kind: "date", label: "June 2026" });
      expect(labels("june 2024")).toEqual(["date:June 2024"]);
    });
  });

  describe("gear", () => {
    it("is matched by name, by model, and a lens by its focal length", () => {
      expect(read("z 6").parts).toMatchObject([{ kind: "camera", values: ["Nikon Z 6"] }]);
      expect(read("ILCE-6400").parts).toMatchObject([{ kind: "camera", values: ["Sony ILCE-6400"] }]);
      expect(read("50mm").parts).toMatchObject([
        { kind: "lens", values: ["NIKKOR Z 50mm f/1.8 S", "FE 50mm F1.8"] },
      ]);
    });
  });

  describe("genres and conditions", () => {
    it("are taken where they plainly are one", () => {
      expect(labels("portrait 2024")).toEqual(["genre:portrait", "date:2024"]);
      expect(labels("street photography in London")).toEqual(["genre:street", "place:London"]);
      expect(read("street photography in London").rest).toBe("");
      expect(labels("portraits of Anna")).toEqual(["genre:portrait", "person:Anna"]);
      expect(labels("Anna, golden hour")).toEqual(["person:Anna", "condition:golden hour"]);
      expect(labels("landscape")).toEqual(["genre:landscape"]);
    });

    it("stay part of a description", () => {
      for (const q of ["busy street at night", "golden hour beach", "portrait of a dog in the snow"]) {
        const r = read(q);
        expect(r.parts, q).toEqual([]);
        expect(r.rest, q).toBe(q);
      }
    });
  });

  it("leaves text it does not know alone", () => {
    const r = read("golden retriever on a beach");
    expect(r.parts).toEqual([]);
    expect(r.rest).toBe("golden retriever on a beach");
    expect(read("").parts).toEqual([]);
  });

  it("drops the words that only asked for photos", () => {
    const r = read("show me photos of Anna from 2023");
    expect(r.parts.map((p) => p.kind)).toEqual(["person", "date"]);
    expect(r.rest).toBe("");
  });
});

describe("a reading in the filter", () => {
  it("joins what is already chosen, and comes back out without touching it", () => {
    const before = { ...EMPTY_FILTER, personIds: [2], dateFrom: "2020-01-01", dateTo: "2020-12-31" };
    const r = read("Anna and Ben in United Kingdom 2023");
    const { filter, applied } = applyReading(before, r);
    expect(filter.personIds).toEqual([2, 1]);
    expect(filter.placeIds).toEqual([10, 11]);
    expect([filter.dateFrom, filter.dateTo]).toEqual(["2023-01-01", "2023-12-31"]);
    expect(applied.personIds).toEqual([1]);
    expect(applied.groups).toEqual([{ label: "United Kingdom", placeIds: [10, 11] }]);

    const after = withoutReading(filter, applied);
    expect(after.personIds).toEqual([2]);
    expect(after.placeIds).toEqual([]);
    expect([after.dateFrom, after.dateTo]).toEqual(["2020-01-01", "2020-12-31"]);
  });

  it("makes several dates one range", () => {
    const { filter } = applyReading(EMPTY_FILTER, read("summer 2023 and summer 2024"));
    expect([filter.dateFrom, filter.dateTo]).toEqual(["2023-06-01", "2024-08-31"]);
  });

  it("leaves dates changed since alone", () => {
    const { filter, applied } = applyReading(EMPTY_FILTER, read("2023"));
    const moved = { ...filter, dateTo: "2023-06-30" };
    expect(withoutReading(moved, applied).dateTo).toBe("2023-06-30");
  });
});

describe("reading a search in other languages", () => {
  afterEach(async () => {
    await setLocale("en");
  });
  const range = (q: string) => dateOf(q)?.slice(0, 2);

  it("reads German dates, joining words and months", async () => {
    await setLocale("de");
    expect(range("letzten Sommer")).toEqual(["2026-06-01", "2026-08-31"]);
    expect(range("Weihnachten 2024")).toEqual(["2024-12-24", "2024-12-26"]);
    expect(range("März 2025")).toEqual(["2025-03-01", "2025-03-31"]);
    const r = read("Fotos von Anna in Edinburgh gestern");
    expect(r.parts.map((p) => p.kind)).toEqual(["person", "place", "date"]);
    expect(r.rest).toBe("");
  });

  it("reads French with the adjective after its noun, and elisions", async () => {
    await setLocale("fr");
    expect(range("l'été dernier")).toEqual(["2026-06-01", "2026-08-31"]);
    expect(range("la semaine dernière")).toEqual(["2026-09-14", "2026-09-20"]);
    expect(range("aout 2024")).toEqual(["2024-08-01", "2024-08-31"]);
    expect(labels("photos d'Anna")).toEqual(["person:Anna"]);
  });

  it("reads Spanish the same way", async () => {
    await setLocale("es");
    expect(range("el verano pasado")).toEqual(["2026-06-01", "2026-08-31"]);
    expect(range("este año")).toEqual(["2026-01-01", "2026-12-31"]);
    expect(read("fotos de Anna en París, atardecer").rest).toBe("atardecer");
  });

  it("reads Japanese without spaces", async () => {
    await setLocale("ja");
    // The summer of last year — not the latest summer, as English "last summer" is.
    expect(range("去年の夏")).toEqual(["2025-06-01", "2025-08-31"]);
    expect(range("今年の冬")?.[0]).toBe("2026-12-01");
    expect(range("去年")).toEqual(["2025-01-01", "2025-12-31"]);
    expect(range("先週")).toEqual(["2026-09-14", "2026-09-20"]);
    expect(range("2024年6月")).toEqual(["2024-06-01", "2024-06-30"]);
    const r = read("去年の夏のAnnaの写真");
    expect(r.parts.map((p) => p.kind)).toEqual(["date", "person"]);
  });

  it("keeps English working whatever the language", async () => {
    await setLocale("de");
    expect(range("last summer")).toEqual(["2026-06-01", "2026-08-31"]);
    // …without reading English as if it put "this" after its noun.
    await setLocale("en");
    expect(labels("June this year")).toEqual(["person:June", "date:2026"]);
  });

  it("shows back what was typed, not the words it read", async () => {
    await setLocale("de");
    expect(read("letzten Sommer").parts[0]?.text).toBe("letzten Sommer");
    await setLocale("ja");
    expect(read("先週 夕焼け").parts[0]?.text).toBe("先週");
    expect(read("先週 夕焼け").rest).toBe("夕焼け");
  });
});

describe("leaving things out", () => {
  const V: Vocabulary = { ...VOCAB, keywords: ["clouds", "dog", "sky", "landscape"] };
  const readV = (q: string) => readQuery(q, V, NOW);
  const parts = (q: string) =>
    readV(q).parts.map((p) => (p.kind === "exclude" ? `${p.facet}:${p.label}` : `${p.kind}:${p.label}`));

  it("reads a keyword after a negation as one to leave out", () => {
    const r = readV("sunset, not clouds");
    expect(parts("sunset, not clouds")).toEqual(["subject:not clouds"]);
    expect(r.rest).toBe("sunset");
    expect(parts("without dog")).toEqual(["subject:not dog"]);
  });

  it("takes a dash as a negation", () => {
    const r = readV("-clouds beach");
    expect(parts("-clouds beach")).toEqual(["subject:not clouds"]);
    expect(r.rest).toBe("beach");
  });

  it("leaves out genres and conditions too, a genre first when a word is both", () => {
    expect(parts("not golden hour")).toEqual(["condition:not golden hour"]);
    expect(parts("no landscape")).toEqual(["genre:not landscape"]);
  });

  it("leaves a keyword said plainly to content search", () => {
    const r = readV("clouds over the sea");
    expect(r.parts).toEqual([]);
    expect(r.rest).toBe("clouds over the sea");
  });

  it("goes into the filter and comes back out whole", () => {
    const r = readV("Anna, not clouds, -landscape");
    const { filter, applied } = applyReading(EMPTY_FILTER, r);
    expect(filter.excludeSubjects).toEqual(["clouds"]);
    expect(filter.excludeGenres).toEqual(["landscape"]);
    expect(filter.personIds).toEqual([1]);
    const back = withoutReading(filter, applied);
    expect(back.excludeSubjects).toEqual([]);
    expect(back.excludeGenres).toEqual([]);
    expect(back.personIds).toEqual([]);
  });
});
