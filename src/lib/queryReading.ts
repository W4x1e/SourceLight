import type { PhotoFilter } from "../types";
import { formatDate, getLocale, t, type Key } from "../i18n";
import { fold, LAST_YEAR, THIS_YEAR, type QueryLexicon } from "../i18n/query";
import de from "../i18n/query/de";
import es from "../i18n/query/es";
import fr from "../i18n/query/fr";
import ja from "../i18n/query/ja";
import { tagName } from "../i18n/tags";
import { conditionName } from "./content";

/**
 * Reading a search.
 *
 * "Anna in Edinburgh last summer, sunset" is four things, and three of them
 * are facts the catalogue knows exactly: a person, a place and a range of
 * dates. Content search used to get the whole sentence as a description of
 * what the photo shows, which is the one thing it is not. So the search box
 * reads what it recognises first and hands only the rest — "sunset" — to
 * content search.
 *
 * Deterministic and offline: whole words matched against names the library
 * already has, and a handful of date phrases. No language model, so it is
 * instant, and a wrong guess is always the same wrong guess — one click on
 * its chip undoes it, and "as typed" undoes the lot.
 *
 * What it recognises:
 * - **people** the user has named, case-insensitively; a first name alone
 *   when only one person has it;
 * - **places** by name, by the part before a comma ("Edinburgh" for
 *   "Edinburgh, United Kingdom"), and by country;
 * - **dates**: years, `2024-06`, months, seasons, Christmas, "last summer",
 *   "this week", "yesterday";
 * - **cameras and lenses** the library has, by name or model ("Z 6", "50mm");
 * - **genres and conditions**, but only where they plainly are one: a part of
 *   the query on its own ("Anna, golden hour"), all that is left of it
 *   ("portrait 2024"), or said as one ("street photography", "portraits").
 *   Inside a description they stay description: "busy street" is not street
 *   photography, and "golden hour beach" ranks better as content than as a
 *   tag the tagger may have missed.
 * - **what to leave out**: "not", "without", "except", "excluding" or "no"
 *   before a keyword, genre or condition — or a dash on it, "-clouds" — leaves
 *   its photos out. Keywords are read only here: said plainly they are
 *   better left to content search, but "not clouds" can only mean the tag.
 *
 * Where one span reads two ways, the likelier is taken and the other offered
 * ("Paris" the place first, "Paris" the person as the alternative). A longer
 * match beats a shorter one; then a date, a place, a person, gear, a genre, a
 * condition, in that order; a month without a year comes last, since "June"
 * is as often a person.
 */

export interface Vocabulary {
  /** Named people, most photographed first: a name two people share goes to the first. */
  people: { id: number; name: string }[];
  places: { id: number; name: string; country: string | null }[];
  genres: string[];
  conditions: string[];
  cameras: string[];
  lenses: string[];
  /** The keywords photos have, read only after "not". */
  keywords?: string[];
}

export type Part =
  | { kind: "person"; text: string; id: number; label: string }
  | { kind: "place"; text: string; ids: number[]; label: string }
  | { kind: "date"; text: string; from: string; to: string; label: string }
  | { kind: "genre"; text: string; value: string; label: string }
  | { kind: "condition"; text: string; value: string; label: string }
  | { kind: "camera"; text: string; values: string[]; label: string }
  | { kind: "lens"; text: string; values: string[]; label: string }
  | { kind: "exclude"; text: string; facet: "genre" | "subject" | "condition"; value: string; label: string };

export interface Reading {
  /** What was recognised, in the order it was typed. */
  parts: Part[];
  /** What is left for content search; empty when nothing is. */
  rest: string;
  /** A span that reads two ways: the reading taken, and the one offered instead. */
  alternatives: { chosen: Part; other: Part }[];
}

// ---------------------------------------------------------------------------
// Words

interface Token {
  /** As typed, for showing back. Empty on the second word of a phrase one
   *  typed word stood for (Japanese 先週, "last week"). */
  raw: string;
  /** The word the parser reads: lower-cased, and in English where the active
   *  language's lexicon knows it (see `i18n/query`). */
  norm: string;
  /** Which comma-separated part of the query it is in. */
  segment: number;
}

/** Lower-cased and accent-folded, with a possessive dropped: "Anna's" is Anna, "José" is jose. */
function normalise(word: string): string {
  return fold(word).replace(/['’]s$/, "");
}

const LEXICONS: Record<string, QueryLexicon> = { de, es, fr, ja };

const MONTH_TABLES = new Map<string, ReadonlyMap<string, string>>();
/** The active language's month names, long and short, each to its English name. */
function localMonths(locale: string): ReadonlyMap<string, string> {
  let table = MONTH_TABLES.get(locale);
  if (!table) {
    const map = new Map<string, string>();
    if (locale in LEXICONS) {
      for (let m = 0; m < 12; m++) {
        const at = Date.UTC(2020, m, 15);
        for (const month of ["long", "short"] as const) {
          const name = fold(new Intl.DateTimeFormat(locale, { month, timeZone: "UTC" }).format(at)).replace(/\.$/, "");
          if (!/\d/.test(name)) map.set(name, MONTHS[m]!);
        }
      }
    }
    table = map;
    MONTH_TABLES.set(locale, table);
  }
  return table;
}

const strip = (w: string) => w.replace(/^[("'“‘「『[]+/, "").replace(/[)"'”’」』\].!?:。！？]+$/, "");

/** The English word(s) the parser reads for one typed word. */
function canonical(raw: string, lex: QueryLexicon | undefined, months: ReadonlyMap<string, string>): readonly string[] {
  let w = fold(raw);
  for (const e of lex?.elisions ?? []) if (w.startsWith(e) && w.length > e.length) w = w.slice(e.length);
  const known = lex?.words[w];
  if (known) return typeof known === "string" ? [known] : known;
  const month = months.get(w.replace(/\.$/, ""));
  if (month) return [month];
  return [normalise(w)];
}

/** Words as a language without spaces between them has them. */
function segmentWords(text: string, locale: string): string[] {
  const Segmenter = (Intl as { Segmenter?: typeof Intl.Segmenter }).Segmenter;
  if (!Segmenter) return [text];
  return [...new Segmenter(locale, { granularity: "word" }).segment(text)]
    .filter((s) => s.isWordLike)
    .map((s) => s.segment);
}

function tokenize(query: string): Token[] {
  const locale = getLocale();
  const lex = LEXICONS[locale];
  const months = localMonths(locale);
  const out: Token[] = [];
  let segment = 0;
  const push = (raw: string, norms: readonly string[]) =>
    norms.forEach((norm, i) => out.push({ raw: i === 0 ? raw : "", norm, segment }));
  for (const m of query.matchAll(/[^\s,;、，]+|[,;、，]/g)) {
    const t = m[0];
    if (t === "," || t === ";" || t === "、" || t === "，") {
      segment += 1;
      continue;
    }
    const raw = strip(t);
    if (!raw) continue;
    if (!lex?.segment) {
      push(raw, canonical(raw, lex, months));
      continue;
    }
    // A language without spaces: its phrases first, then the words between.
    let rest = raw;
    while (rest) {
      let first: { at: number; text: string; norms: readonly string[] } | null = null;
      for (const [re, make] of lex.phrases ?? []) {
        const hit = re.exec(rest);
        if (hit && (first === null || hit.index < first.at)) first = { at: hit.index, text: hit[0], norms: make(hit) };
      }
      const before = first ? rest.slice(0, first.at) : rest;
      for (const w of segmentWords(before, locale)) push(w, canonical(w, lex, months));
      if (!first) break;
      push(first.text, first.norms);
      rest = rest.slice(first.at + first.text.length);
    }
  }
  return out;
}

function words(name: string): string[] {
  return name
    .split(/[\s,;]+/)
    .map((w) => normalise(strip(w)))
    .filter(Boolean);
}

/** Typed words joined back up: with spaces, except between two that were written without. */
function join(raws: string[]): string {
  const cjk = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}]/u;
  let out = "";
  for (const r of raws.filter(Boolean)) {
    out += out && !(cjk.test(out.at(-1)!) && cjk.test(r[0]!)) ? ` ${r}` : r;
  }
  return out;
}

/** Words that only join an entity to the sentence: "in" Edinburgh, "with" Anna. */
const GLUE = new Set(["in", "at", "on", "from", "during", "of", "with", "and", "&", "near", "around", "by", "since"]);
/** Words about the search, not the photo: "show me photos of …". */
const META = new Set([
  "photo",
  "photos",
  "photograph",
  "photographs",
  "picture",
  "pictures",
  "pic",
  "pics",
  "image",
  "images",
  "shot",
  "shots",
  "taken",
  "show",
  "me",
  "find",
  "all",
  "my",
  "our",
]);
const ARTICLES = new Set(["the", "a", "an", "some"]);
const FILLER = (w: string) => GLUE.has(w) || META.has(w) || ARTICLES.has(w);
/** What makes a genre word a genre: "street photography", "landscape shots". */
const PHOTO_WORDS = new Set([
  "photo",
  "photos",
  "photograph",
  "photographs",
  "photography",
  "picture",
  "pictures",
  "pic",
  "pics",
  "image",
  "images",
  "shot",
  "shots",
]);

// ---------------------------------------------------------------------------
// Dates

const MONTHS = [
  "january",
  "february",
  "march",
  "april",
  "may",
  "june",
  "july",
  "august",
  "september",
  "october",
  "november",
  "december",
];
const MONTH_ABBR: Record<string, number> = {
  jan: 0,
  feb: 1,
  mar: 2,
  apr: 3,
  jun: 5,
  jul: 6,
  aug: 7,
  sep: 8,
  sept: 8,
  oct: 9,
  nov: 10,
  dec: 11,
};
/** First and last month of each season, northern hemisphere; winter runs into the next year. */
const SEASONS: Record<string, [number, number]> = {
  spring: [2, 4],
  summer: [5, 7],
  autumn: [8, 10],
  fall: [8, 10],
  winter: [11, 13],
};

const pad = (n: number) => String(n).padStart(2, "0");
/** A calendar date as the filter holds it; months and days may overflow, as `Date.UTC` allows. */
function ymd(y: number, m: number, d: number): string {
  const t = new Date(Date.UTC(y, m, d));
  return `${t.getUTCFullYear()}-${pad(t.getUTCMonth() + 1)}-${pad(t.getUTCDate())}`;
}

/** The season labels, by the season word the query parser recognised. */
const SEASON_LABEL: Record<string, Key> = {
  spring: "filters.reading.spring",
  summer: "filters.reading.summer",
  autumn: "filters.reading.autumn",
  winter: "filters.reading.winter",
};

interface Range {
  from: string;
  to: string;
  label: string;
}

function monthIn(word: string, needsYear: boolean): number | null {
  const full = MONTHS.indexOf(word);
  // "May" alone is as often a verb.
  if (full >= 0) return needsYear || word !== "may" ? full : null;
  return needsYear && word in MONTH_ABBR ? (MONTH_ABBR[word] ?? null) : null;
}

function yearIn(word: string, now: Date): number | null {
  if (word === LAST_YEAR) return now.getFullYear() - 1;
  if (word === THIS_YEAR) return now.getFullYear();
  return /^(19|20)\d\d$/.test(word) ? Number(word) : null;
}

function monthRange(y: number, m: number): Range {
  return { from: ymd(y, m, 1), to: ymd(y, m + 1, 0), label: formatDate(new Date(Date.UTC(y, m, 1)), { month: "long", year: "numeric", timeZone: "UTC" }) };
}

function seasonRange(y: number, name: string): Range {
  const [a, b] = SEASONS[name] ?? [0, 0];
  const season = name === "fall" ? "autumn" : name;
  return { from: ymd(y, a, 1), to: ymd(y, b + 1, 0), label: t(SEASON_LABEL[season] ?? "filters.reading.spring", { year: String(y) }) };
}

function christmasRange(y: number): Range {
  return { from: `${y}-12-24`, to: `${y}-12-26`, label: t("filters.reading.christmas", { year: String(y) }) };
}

/**
 * The date phrases that start at `i`, each with how many words it takes and
 * whether it is only a month without a year (the weakest reading).
 */
function datesAt(toks: Token[], i: number, now: Date): { len: number; range: Range; weak: boolean }[] {
  const w0 = toks[i]?.norm ?? "";
  const w1 = toks[i + 1]?.norm ?? "";
  const y = now.getFullYear();
  const m = now.getMonth();
  const today = ymd(y, m, now.getDate());
  const out: { len: number; range: Range; weak: boolean }[] = [];

  // The most recent occurrence that is over, and the one this year.
  const lastOf = (make: (year: number) => Range) =>
    [y, y - 1, y - 2].map(make).find((r) => r.to < today) ?? make(y - 1);
  // "last summer", and — where a language puts it after — "summer last":
  // French "l'été dernier", Spanish "el verano pasado" read that way.
  const postfix = LEXICONS[getLocale()]?.postfix && (w1 === "last" || w1 === "this");
  const which = w0 === "last" || w0 === "this" ? w0 : postfix ? w1 : null;
  if (which) {
    const last = which === "last";
    const noun = which === w0 ? w1 : w0;
    let r: Range | null = null;
    if (noun in SEASONS) {
      // This winter is the one under way in January, else the one to come.
      const thisYear = noun === "winter" && m <= 1 ? y - 1 : y;
      r = last ? lastOf((yy) => seasonRange(yy, noun)) : seasonRange(thisYear, noun);
    } else if (monthIn(noun, true) !== null) {
      const mm = monthIn(noun, true) ?? 0;
      r = last ? lastOf((yy) => monthRange(yy, mm)) : monthRange(y, mm);
    } else if (noun === "christmas" || noun === "xmas") {
      r = last ? lastOf(christmasRange) : christmasRange(y);
    } else if (noun === "year") {
      const yy = last ? y - 1 : y;
      r = { from: `${yy}-01-01`, to: `${yy}-12-31`, label: String(yy) };
    } else if (noun === "month") {
      const r0 = monthRange(last ? (m === 0 ? y - 1 : y) : y, last ? (m + 11) % 12 : m);
      r = r0;
    } else if (noun === "week") {
      // Weeks start on Monday.
      const monday = now.getDate() - ((now.getDay() + 6) % 7);
      r = last
        ? { from: ymd(y, m, monday - 7), to: ymd(y, m, monday - 1), label: t("filters.reading.lastWeek") }
        : { from: ymd(y, m, monday), to: today, label: t("filters.reading.thisWeek") };
    }
    if (r) out.push({ len: 2, range: r, weak: false });
  }

  const year1 = yearIn(w1, now);
  if (year1 !== null) {
    if (w0 in SEASONS) out.push({ len: 2, range: seasonRange(year1, w0), weak: false });
    const mm = monthIn(w0, true);
    if (mm !== null) out.push({ len: 2, range: monthRange(year1, mm), weak: false });
    if (w0 === "christmas" || w0 === "xmas") out.push({ len: 2, range: christmasRange(year1), weak: false });
  }

  const year0 = yearIn(w0, now);
  if (year0 !== null) out.push({ len: 1, range: { from: `${year0}-01-01`, to: `${year0}-12-31`, label: String(year0) }, weak: false });
  const span = /^((?:19|20)\d\d)[-–]((?:19|20)\d\d)$/.exec(w0);
  if (span && Number(span[1]) <= Number(span[2])) {
    out.push({ len: 1, range: { from: `${span[1]}-01-01`, to: `${span[2]}-12-31`, label: `${span[1]}–${span[2]}` }, weak: false });
  }
  const iso = /^((?:19|20)\d\d)[-/](\d{1,2})(?:[-/](\d{1,2}))?$/.exec(w0);
  if (iso) {
    const [yy, mo, d] = [Number(iso[1]), Number(iso[2]) - 1, iso[3] ? Number(iso[3]) : null];
    if (mo >= 0 && mo < 12) {
      if (d === null) out.push({ len: 1, range: monthRange(yy, mo), weak: false });
      else if (d >= 1 && d <= 31) {
        const day = ymd(yy, mo, d);
        out.push({ len: 1, range: { from: day, to: day, label: day }, weak: false });
      }
    }
  }
  if (w0 === "today") out.push({ len: 1, range: { from: today, to: today, label: t("filters.reading.today") }, weak: false });
  if (w0 === "yesterday") {
    const day = ymd(y, m, now.getDate() - 1);
    out.push({ len: 1, range: { from: day, to: day, label: t("filters.reading.yesterday") }, weak: false });
  }
  // A month alone is the latest one that has begun.
  const bare = monthIn(w0, false);
  if (bare !== null) out.push({ len: 1, range: monthRange(bare <= m ? y : y - 1, bare), weak: true });
  return out;
}

// ---------------------------------------------------------------------------
// Names

interface Entry {
  words: string[];
  make: (text: string) => Part;
  /** Genres and conditions: only taken where they plainly are one. */
  structural?: boolean;
  /** A genre said as a plural, which says so on its own: "portraits". */
  plural?: boolean;
}

const PRIORITY: Record<Part["kind"], number> = {
  exclude: -1,
  date: 0,
  place: 1,
  person: 2,
  camera: 3,
  lens: 4,
  genre: 5,
  condition: 6,
};

function entries(v: Vocabulary): Entry[] {
  const out: Entry[] = [];
  const key = (ws: string[]) => ws.join(" ");

  // People: full names, then a first name that is nobody else's.
  const people = new Map<string, { id: number; name: string }>();
  for (const p of v.people) {
    const k = key(words(p.name));
    if (k && !people.has(k)) people.set(k, p);
  }
  const firsts = new Map<string, { id: number; name: string } | null>();
  for (const p of people.values()) {
    const ws = words(p.name);
    if (ws.length < 2 || !ws[0]) continue;
    firsts.set(ws[0], firsts.has(ws[0]) ? null : p);
  }
  for (const [k, p] of people) {
    out.push({ words: k.split(" "), make: (text) => ({ kind: "person", text, id: p.id, label: p.name }) });
  }
  for (const [first, p] of firsts) {
    if (p && !people.has(first)) {
      out.push({ words: [first], make: (text) => ({ kind: "person", text, id: p.id, label: p.name }) });
    }
  }

  // Places: by name, by the name before its comma, and by country. Two
  // places answering to the same words are both meant.
  const places = new Map<string, { ids: number[]; label: string }>();
  const add = (ws: string[], id: number, label: string) => {
    const k = key(ws);
    if (!k) return;
    const hit = places.get(k);
    if (hit) {
      if (!hit.ids.includes(id)) hit.ids.push(id);
    } else places.set(k, { ids: [id], label });
  };
  for (const p of v.places) {
    add(words(p.name), p.id, p.name);
    const head = p.name.split(",")[0]?.trim() ?? "";
    if (head && head !== p.name) add(words(head), p.id, head);
    if (p.country) add(words(p.country), p.id, p.country);
  }
  for (const [k, p] of places) {
    out.push({ words: k.split(" "), make: (text) => ({ kind: "place", text, ids: [...p.ids], label: p.label }) });
  }

  // Gear: the name as EXIF has it, the model without its make ("Z 6"), and a
  // lens by its focal length ("50mm") — every lens that has it.
  const gear = (kind: "camera" | "lens", names: string[]) => {
    const found = new Map<string, { values: string[]; label: string }>();
    const put = (ws: string[], value: string, label: string) => {
      const k = key(ws);
      if (!k) return;
      const hit = found.get(k);
      if (hit) {
        if (!hit.values.includes(value)) hit.values.push(value);
      } else found.set(k, { values: [value], label });
    };
    for (const name of names) {
      const ws = words(name);
      put(ws, name, name);
      if (kind === "camera" && ws.length > 1 && /\d/.test(ws.slice(1).join(""))) put(ws.slice(1), name, name);
      if (kind === "lens") {
        for (const w of ws) if (/^\d+(-\d+)?mm$/.test(w)) put([w], name, w);
      }
    }
    for (const [k, g] of found) {
      out.push({
        words: k.split(" "),
        make: (text) => ({ kind, text, values: [...g.values], label: g.values.length === 1 ? (g.values[0] ?? g.label) : g.label }),
      });
    }
  };
  gear("camera", v.cameras);
  gear("lens", v.lenses);

  // Genres and conditions answer to their English words and, in another
  // language, to their names there too ("Landschaft" is landscape).
  const local = (english: string, name: string) => {
    const ws = words(name);
    return ws.length && key(ws) !== key(words(english)) ? [ws] : [];
  };
  for (const g of v.genres) {
    const make = (text: string): Part => ({ kind: "genre", text, value: g, label: tagName(g) });
    out.push({ words: words(g), make, structural: true });
    if (!g.endsWith("s")) out.push({ words: words(`${g}s`), make, structural: true, plural: true });
    for (const ws of local(g, tagName(g))) out.push({ words: ws, make, structural: true });
  }
  for (const c of v.conditions) {
    const make = (text: string): Part => ({ kind: "condition", text, value: c, label: conditionName(c) });
    out.push({ words: words(c), make, structural: true });
    for (const ws of local(c, conditionName(c))) out.push({ words: ws, make, structural: true });
  }
  return out;
}

/** Words that turn what follows into something to leave out. */
const NEGATIONS = new Set(["not", "without", "except", "excluding", "no"]);

/** What can be left out, genres first so a word that is both is read as one. */
function excludable(v: Vocabulary): { words: string[]; facet: "genre" | "subject" | "condition"; value: string }[] {
  return [
    ...v.genres.map((g) => ({ words: words(g), facet: "genre" as const, value: g })),
    ...v.conditions.map((c) => ({ words: words(c), facet: "condition" as const, value: c })),
    ...(v.keywords ?? []).map((k) => ({ words: words(k), facet: "subject" as const, value: k })),
  ].filter((e) => e.words.length > 0);
}

// ---------------------------------------------------------------------------
// Reading

interface Candidate {
  start: number;
  end: number;
  part: Part;
  rank: number;
  structural: boolean;
  plural: boolean;
}

export function readQuery(query: string, vocab: Vocabulary, now: Date): Reading {
  const t = tokenize(query);
  const text = (s: number, e: number) => join(t.slice(s, e).map((x) => x.raw));
  const all: Candidate[] = [];
  const known = entries(vocab);
  for (let i = 0; i < t.length; i++) {
    for (const d of datesAt(t, i, now)) {
      const part: Part = { kind: "date", text: text(i, i + d.len), ...d.range };
      all.push({ start: i, end: i + d.len, part, rank: d.weak ? 9 : PRIORITY.date, structural: false, plural: false });
    }
    for (const e of known) {
      const n = e.words.length;
      if (i + n > t.length) continue;
      if (e.words.every((w, k) => t[i + k]?.norm === w)) {
        all.push({
          start: i,
          end: i + n,
          part: e.make(text(i, i + n)),
          rank: PRIORITY[e.make("").kind],
          structural: !!e.structural,
          plural: !!e.plural,
        });
      }
    }
  }

  // What to leave out: "not clouds", "without people", "-clouds". The
  // negation is part of the span, so it beats the plain word it contains.
  const leaveOut = excludable(vocab);
  for (let i = 0; i < t.length; i++) {
    const tok = t[i]!;
    const negated = NEGATIONS.has(tok.norm);
    const dashed = !negated && tok.norm.length > 1 && tok.norm.startsWith("-");
    if (!negated && !dashed) continue;
    const from = negated ? i + 1 : i;
    const norm = (k: number) => (dashed && k === 0 ? tok.norm.slice(1) : t[from + k]?.norm);
    for (const e of leaveOut) {
      const n = e.words.length;
      if (from + n > t.length || !e.words.every((w, k) => norm(k) === w)) continue;
      all.push({
        start: i,
        end: from + n,
        part: { kind: "exclude", text: text(i, from + n), facet: e.facet, value: e.value, label: `not ${e.value}` },
        rank: PRIORITY.exclude,
        structural: false,
        plural: false,
      });
    }
  }

  // Longest first; then by kind; then as typed.
  const order = (a: Candidate, b: Candidate) =>
    b.end - b.start - (a.end - a.start) || a.rank - b.rank || a.start - b.start;
  const taken: Candidate[] = [];
  const alternatives: Reading["alternatives"] = [];
  const overlaps = (c: Candidate) => taken.find((x) => c.start < x.end && x.start < c.end);
  for (const c of all.filter((x) => !x.structural).sort(order)) {
    const hit = overlaps(c);
    if (!hit) {
      taken.push(c);
    } else if (
      hit.start === c.start &&
      hit.end === c.end &&
      hit.part.kind !== c.part.kind &&
      !alternatives.some((a) => a.chosen === hit.part)
    ) {
      alternatives.push({ chosen: hit.part, other: c.part });
    }
  }

  // Genres and conditions, where they plainly are one: said as one, or all
  // that is left of their part of the query once the rest is read.
  const claimed = (i: number) => taken.some((c) => i >= c.start && i < c.end);
  const structural: Candidate[] = [];
  for (const c of all.filter((x) => x.structural).sort(order)) {
    if (overlaps(c) || structural.some((x) => c.start < x.end && x.start < c.end)) continue;
    structural.push(c);
  }
  for (const c of structural) {
    const next = t[c.end];
    if (c.part.kind === "genre" && next && PHOTO_WORDS.has(next.norm)) {
      taken.push({ ...c, end: c.end + 1, part: { ...c.part, text: text(c.start, c.end + 1) } });
      continue;
    }
    if (c.plural) {
      taken.push(c);
      continue;
    }
    const segment = t[c.start]?.segment;
    const inStructural = (i: number) => structural.some((x) => i >= x.start && i < x.end);
    const leftover = t.some(
      (x, i) => x.segment === segment && !claimed(i) && !inStructural(i) && !FILLER(x.norm),
    );
    if (!leftover) taken.push(c);
  }

  // What is left, less the words that only tied an entity into the sentence.
  const covered = (i: number) => taken.some((c) => i >= c.start && i < c.end);
  const startsPart = (i: number) => taken.some((c) => c.start === i);
  let rest = t
    .map((x, i) => ({ x, i }))
    .filter(({ x, i }) => !covered(i) && !(GLUE.has(x.norm) && startsPart(i + 1)))
    .map(({ x }) => x);
  while (rest.length && FILLER(rest[0]?.norm ?? "")) rest = rest.slice(1);
  while (rest.length && FILLER(rest[rest.length - 1]?.norm ?? "")) rest = rest.slice(0, -1);

  return {
    parts: taken.sort((a, b) => a.start - b.start).map((c) => c.part),
    rest: join(rest.map((x) => x.raw)),
    alternatives,
  };
}

// ---------------------------------------------------------------------------
// Into the filter

/** What a reading added to the filter, so it can be taken back out whole. */
export interface Applied {
  personIds: number[];
  placeIds: number[];
  genres: string[];
  conditions: string[];
  cameras: string[];
  lenses: string[];
  /** What it left out. */
  excludeGenres: string[];
  excludeSubjects: string[];
  excludeConditions: string[];
  /** The dates it set, and what the filter had before. */
  date: { from: string; to: string; label: string; before: { from: string | null; to: string | null } } | null;
  /** Places recognised together — a country — to show as one chip. */
  groups: { label: string; placeIds: number[] }[];
}

const union = <T,>(have: T[], add: T[]) => {
  const added = add.filter((x, i) => !have.includes(x) && add.indexOf(x) === i);
  return { list: [...have, ...added], added };
};

/**
 * The filter with a reading's parts added, and a record of what was added.
 * Parts join what is already chosen rather than replacing it; several dates
 * become one range from the earliest to the latest.
 */
export function applyReading(filter: PhotoFilter, r: Reading): { filter: PhotoFilter; applied: Applied } {
  const of = <K extends Part["kind"]>(kind: K) => r.parts.filter((p): p is Extract<Part, { kind: K }> => p.kind === kind);
  const people = union(filter.personIds ?? [], of("person").map((p) => p.id));
  const places = union(filter.placeIds, of("place").flatMap((p) => p.ids));
  const genres = union(filter.genres, of("genre").map((p) => p.value));
  const conditions = union(filter.conditions, of("condition").map((p) => p.value));
  const cameras = union(filter.cameras ?? [], of("camera").flatMap((p) => p.values));
  const lenses = union(filter.lenses ?? [], of("lens").flatMap((p) => p.values));
  const out = (facet: "genre" | "subject" | "condition") =>
    of("exclude")
      .filter((p) => p.facet === facet)
      .map((p) => p.value);
  const excludeGenres = union(filter.excludeGenres ?? [], out("genre"));
  const excludeSubjects = union(filter.excludeSubjects ?? [], out("subject"));
  const excludeConditions = union(filter.excludeConditions ?? [], out("condition"));
  const dates = of("date");
  const date =
    dates.length === 0
      ? null
      : {
          from: dates.map((d) => d.from).sort()[0] ?? "",
          to: dates.map((d) => d.to).sort().at(-1) ?? "",
          label: dates.length === 1 ? (dates[0]?.label ?? "") : dates.map((d) => d.label).join(" – "),
          before: { from: filter.dateFrom ?? null, to: filter.dateTo ?? null },
        };
  const groups = of("place")
    .filter((p) => p.ids.length > 1)
    .map((p) => ({ label: p.label, placeIds: p.ids }));
  return {
    filter: {
      ...filter,
      personIds: people.list,
      placeIds: places.list,
      genres: genres.list,
      conditions: conditions.list,
      cameras: cameras.list,
      lenses: lenses.list,
      excludeGenres: excludeGenres.list,
      excludeSubjects: excludeSubjects.list,
      excludeConditions: excludeConditions.list,
      dateFrom: date ? date.from : filter.dateFrom,
      dateTo: date ? date.to : filter.dateTo,
    },
    applied: {
      personIds: people.added,
      placeIds: places.added,
      genres: genres.added,
      conditions: conditions.added,
      cameras: cameras.added,
      lenses: lenses.added,
      excludeGenres: excludeGenres.added,
      excludeSubjects: excludeSubjects.added,
      excludeConditions: excludeConditions.added,
      date,
      groups,
    },
  };
}

/** The filter without what a reading added — anything chosen before it stays. */
export function withoutReading(filter: PhotoFilter, a: Applied): PhotoFilter {
  const drop = <T,>(list: T[] | undefined, gone: T[]) => (list ?? []).filter((x) => !gone.includes(x));
  const datesAreOurs = a.date !== null && filter.dateFrom === a.date.from && filter.dateTo === a.date.to;
  return {
    ...filter,
    personIds: drop(filter.personIds, a.personIds),
    placeIds: drop(filter.placeIds, a.placeIds),
    genres: drop(filter.genres, a.genres),
    conditions: drop(filter.conditions, a.conditions),
    cameras: drop(filter.cameras, a.cameras),
    lenses: drop(filter.lenses, a.lenses),
    excludeGenres: drop(filter.excludeGenres, a.excludeGenres ?? []),
    excludeSubjects: drop(filter.excludeSubjects, a.excludeSubjects ?? []),
    excludeConditions: drop(filter.excludeConditions, a.excludeConditions ?? []),
    dateFrom: datesAreOurs ? (a.date?.before.from ?? null) : filter.dateFrom,
    dateTo: datesAreOurs ? (a.date?.before.to ?? null) : filter.dateTo,
  };
}
