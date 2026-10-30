import { createElement, Fragment, type ReactNode } from "react";
import { en } from "./en";
import { loadTagNames } from "./tags";

/**
 * The interface's words, in the user's language.
 *
 * English is the source: `en/` holds every string the interface shows, one file
 * per area, and its keys are the type every other catalogue is checked against
 * — a key that is misspelt or missing from English is a compile error, and a
 * placeholder the message needs but the call leaves out is one too. Any other
 * language is a partial copy: a key it has not translated yet falls back to the
 * English, so a new string never shows up blank or as its key.
 *
 * **`t` is a plain function**, not a hook, so the store's toasts, the Mac menu
 * bar and the dialogs can use it as well as components. Components pick up a
 * change of language because the root re-renders when it changes (see
 * `LocaleRoot` in main.tsx) and nothing in the tree is memoised. The one rule
 * that follows: **never call `t` at module scope** — a label table at the top of
 * a file is built once, in whatever language was active at import. Make it a
 * function, or keep the key in the table and call `t` where it is shown.
 *
 * Messages are `"Imported {count} photos"`-style strings. A number passed as a
 * parameter is formatted for the language (1,234 · 1.234 · 1 234), so years,
 * ids and anything already formatted go in as strings. A message that varies by
 * count is an object of plural forms — `{ one: "{count} photo", other: "{count}
 * photos" }` — picked by `Intl.PluralRules` from the `count` parameter, so a
 * language with more forms (Polish, Russian) can use them.
 */

export type PluralForms = { readonly [F in Intl.LDMLPluralRule]?: string } & { readonly other: string };
export type Message = string | PluralForms;

type English = typeof en;
export type Key = keyof English;

/** A language's catalogue: any subset of the English keys. */
export type Catalogue = { readonly [K in Key]?: Message };

/** The placeholder names in one message string: "{a} of {b}" → "a" | "b". */
type Names<S> = S extends `${string}{${infer P}}${infer Rest}` ? P | Names<Rest> : never;
type MessageParams<M> = M extends string
  ? Names<M>
  : M extends PluralForms
    ? Names<M[keyof M]> | "count"
    : never;
/** A key's parameters, or none. Distributes over a union of keys, so a key
 *  picked from a table can be passed as long as every candidate fits. */
type ParamsOf<K extends Key, V> = K extends Key
  ? [MessageParams<English[K]>] extends [never]
    ? []
    : [params: { [P in MessageParams<English[K]>]: V }]
  : never;

/** The languages the app ships, by their own names — which is how a speaker looks for theirs. */
export const LOCALES = [
  { code: "en", name: "English" },
  { code: "de", name: "Deutsch" },
  { code: "es", name: "Español" },
  { code: "fr", name: "Français" },
  { code: "ja", name: "日本語" },
] as const;

/** `en-XA` is the pseudo-locale: every string accented and padded, so one still
 *  hard-coded in English stands out. Development builds only. */
export type Locale = (typeof LOCALES)[number]["code"] | "en-XA";

/** What the user chose in Settings: a language, or whatever the system uses. */
export type LanguagePref = "system" | Locale;

const LOADERS: Record<Exclude<Locale, "en">, () => Promise<Catalogue>> = {
  de: () => import("./de").then((m) => m.default),
  es: () => import("./es").then((m) => m.default),
  fr: () => import("./fr").then((m) => m.default),
  ja: () => import("./ja").then((m) => m.default),
  "en-XA": () => import("./pseudo").then((m) => m.pseudoCatalogue(en)),
};

let active: Locale = "en";
/** Bumped by every change that alters what `t`, `tagName` and friends return,
 *  so the root re-renders (see `LocaleRoot`): a language, or tag names learned. */
let version = 0;
let table: Catalogue = en;
let plurals = new Intl.PluralRules("en");
let numbers = new Intl.NumberFormat("en");
const listeners = new Set<(l: Locale) => void>();

/** The language the interface is showing now. */
export function getLocale(): Locale {
  return active;
}

/** The BCP 47 tag to hand to `Intl` and `<html lang>`: the pseudo-locale formats as English. */
export function intlLocale(locale: Locale = active): string {
  return locale === "en-XA" ? "en" : locale;
}

/**
 * The shipped language that best fits the system's preferences: an exact
 * match first, then the same language in another region (a Swiss German
 * system gets German), and English when none fits.
 */
export function resolveLocale(
  pref: LanguagePref,
  preferred: readonly string[] = typeof navigator === "undefined" ? [] : navigator.languages ?? [navigator.language],
): Locale {
  if (pref !== "system") return pref;
  const codes = LOCALES.map((l) => l.code as string);
  for (const tag of preferred) {
    const lower = tag.toLowerCase();
    if (codes.includes(lower)) return lower as Locale;
    const base = lower.split("-")[0]!;
    if (codes.includes(base)) return base as Locale;
  }
  return "en";
}

/**
 * Switches the interface to `locale`, loading its catalogue first so nothing
 * renders half in one language. A catalogue that fails to load leaves English
 * showing rather than breaking the app.
 */
export async function setLocale(locale: Locale): Promise<void> {
  let next: Catalogue = en;
  if (locale !== "en") {
    try {
      next = await LOADERS[locale]();
    } catch (e) {
      console.warn(`language ${locale} unavailable:`, e);
      locale = "en";
    }
  }
  // The tagger's words load alongside, so the first render has them too.
  const marker = locale === "en-XA" ? (await import("./pseudo")).pseudo : undefined;
  await loadTagNames(locale, marker);
  active = locale;
  table = next;
  plurals = new Intl.PluralRules(intlLocale(locale));
  numbers = new Intl.NumberFormat(intlLocale(locale));
  if (typeof document !== "undefined") document.documentElement.lang = intlLocale(locale);
  version += 1;
  for (const l of listeners) l(locale);
}

/** A value that changes whenever the interface's words do — a new language, or
 *  tag names learned for this one. What the root re-renders on. */
export function textVersion(): number {
  return version;
}

/** Re-renders the interface with words that changed without a change of
 *  language: tag names the local model has just named (`lib/tagNames`). */
export function refreshText(): void {
  version += 1;
  for (const l of listeners) l(active);
}

/**
 * The language AI-written text should be in — descriptions, and the names of
 * tags — as the code the core takes, or null for English. The pseudo-locale
 * is English underneath.
 */
export function contentLanguage(): string | null {
  return active === "en" || active === "en-XA" ? null : active;
}

/** Called after every change of language. Returns the unsubscribe. */
export function onLocaleChange(fn: (l: Locale) => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** The message for `key` in the active language, falling back to English. */
function lookup(key: Key, count: unknown): string {
  const msg: Message = table[key] ?? en[key];
  if (typeof msg === "string") return msg;
  const form = typeof count === "number" ? plurals.select(count) : "other";
  return msg[form] ?? msg.other;
}

/**
 * Intl formatters, built once per language and set of options.
 *
 * Building one costs a fraction of a millisecond and using one almost nothing,
 * and they were built on every call: the library grid asks for a heading per
 * day, so 20,000 photos spent seven seconds of a switch back to Browse making
 * date formatters, and did it again for every frame the filter rail spent
 * folding away (the grid's rows are rebuilt as its column count changes).
 */
const formatters = new Map<string, Intl.DateTimeFormat | Intl.NumberFormat | Intl.ListFormat>();
function cached<F extends Intl.DateTimeFormat | Intl.NumberFormat | Intl.ListFormat>(
  kind: string,
  locale: string,
  opts: object | undefined,
  make: () => F,
): F {
  const key = `${kind}|${locale}|${opts ? JSON.stringify(opts) : ""}`;
  let f = formatters.get(key) as F | undefined;
  if (!f) {
    f = make();
    formatters.set(key, f);
  }
  return f;
}

/** The date formatter for these options, in the active language unless `locale` says otherwise. */
export function dateFormat(opts?: Intl.DateTimeFormatOptions, locale: string = intlLocale()): Intl.DateTimeFormat {
  return cached("date", locale, opts, () => new Intl.DateTimeFormat(locale, opts));
}

/** A number formatted for the active language: 1,234 · 1.234 · 1 234. */
export function formatNumber(n: number, opts?: Intl.NumberFormatOptions): string {
  if (!opts) return numbers.format(n);
  const locale = intlLocale();
  return cached("number", locale, opts, () => new Intl.NumberFormat(locale, opts)).format(n);
}

/** A date formatted for the active language. */
export function formatDate(d: Date | number, opts?: Intl.DateTimeFormatOptions): string {
  return dateFormat(opts).format(d);
}

/** A list joined for the active language: "a, b and c" · "a, b und c". */
export function formatList(items: readonly string[], type: "conjunction" | "disjunction" = "conjunction"): string {
  const locale = intlLocale();
  return cached("list", locale, { type }, () => new Intl.ListFormat(locale, { style: "long", type })).format(items);
}

/**
 * A label as it reads mid-sentence: "White balance" in "Put white balance back
 * to its defaults". English, French and Spanish lower its first letter;
 * German capitalises every noun and Japanese has no case, so they keep it as
 * it is. An acronym ("HSL") is left alone everywhere.
 */
export function inSentence(label: string): string {
  if (active === "de" || active === "ja") return label;
  const second = label.charAt(1);
  if (second && second === second.toLocaleUpperCase() && second !== second.toLocaleLowerCase()) return label;
  return label.charAt(0).toLocaleLowerCase(intlLocale()) + label.slice(1);
}

const PLACEHOLDER = /\{(\w+)\}/g;

/**
 * The interface string for `key`, with its `{placeholders}` filled in.
 *
 *     t("import.done", { count: 12 })   // "Imported 12 photos"
 */
export function t<K extends Key>(key: K, ...args: ParamsOf<K, string | number>): string {
  const params = args[0] as Record<string, string | number> | undefined;
  const msg = lookup(key, params?.count);
  if (!params) return msg;
  return msg.replace(PLACEHOLDER, (whole, name: string) => {
    const v = params[name];
    if (v === undefined) return whole;
    return typeof v === "number" ? numbers.format(v) : v;
  });
}

/**
 * `t` for a message with markup in it: each placeholder can be an element, so a
 * sentence keeps its word order in every language while a number in it stays
 * styled.
 *
 *     tx("browse.selected", { count: <span className="tabular-nums">{n}</span> })
 *
 * A plural message needs the number to choose its form, so pass it as `n` in
 * the second argument when `count` is an element: `tx(key, { count: <b>…</b> }, n)`.
 */
export function tx<K extends Key>(key: K, params: ParamsOf<K, ReactNode>[0] & object, count?: number): ReactNode {
  const values = params as Record<string, ReactNode>;
  const c = count ?? (typeof values.count === "number" ? values.count : undefined);
  const msg = lookup(key, c);
  const out: ReactNode[] = [];
  let last = 0;
  let i = 0;
  for (const m of msg.matchAll(PLACEHOLDER)) {
    if (m.index > last) out.push(msg.slice(last, m.index));
    const v = values[m[1]!];
    out.push(
      v === undefined
        ? m[0]
        : createElement(Fragment, { key: i++ }, typeof v === "number" ? numbers.format(v) : v),
    );
    last = m.index + m[0].length;
  }
  if (last < msg.length) out.push(msg.slice(last));
  return out;
}
