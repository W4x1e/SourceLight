import { t, type Key } from "../i18n";
import { en } from "../i18n/en";
import { tagName } from "../i18n/tags";

/**
 * Names that come from the Rust core, shown in the user's language.
 *
 * The core sends English (`develop/lut_packs.rs`, `develop/presets.json`,
 * `develop/mask.rs`, `tagger/builtin.rs`). It stays the data: what is saved and
 * what is sent back is always the English or the id. These helpers only choose
 * what to *draw*, and fall back to what the core sent for anything they have no
 * words for — a LUT the user imported, a mask they named, a future built-in the
 * catalogue has not caught up with. `content.test.ts` reads the Rust sources and
 * fails when a keyed item is renamed or removed there.
 */

/** "portrait-negative.cube" → "portraitNegative"; "golden hour" → "goldenHour". */
export function slug(name: string): string {
  return name
    .replace(/\.cube$/i, "")
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean)
    .map((w, i) => (i === 0 ? w.toLowerCase() : w[0]!.toUpperCase() + w.slice(1).toLowerCase()))
    .join("");
}

const has = (k: string): k is Key => Object.prototype.hasOwnProperty.call(en, k);

/** The translation under `key`, or `fallback` when the catalogue has no such key. */
function known(key: string, fallback: string): string {
  return has(key) ? t(key) : fallback;
}

/** The subset of a `LutInfo` the helpers read. */
export interface LookLike {
  name: string;
  title: string;
  blurb: string;
  imported?: boolean;
}

/** A built-in look's title; an imported LUT's own. */
export function lookTitle(l: LookLike): string {
  return l.imported ? l.title : known(`content.look.${slug(l.name)}.title`, l.title);
}

export function lookBlurb(l: LookLike): string {
  return l.imported || !l.blurb ? l.blurb : known(`content.look.${slug(l.name)}.blurb`, l.blurb);
}

/** A look family heading. A group the user named (`grouped`) is theirs and stays as typed. */
export function lookFamilyName(family: string, grouped = false): string {
  return grouped ? family : known(`content.lookFamily.${family}`, family);
}

/** True when `query` (already lower-cased) is in the title or blurb, in the language shown or in English. */
export function lookMatches(l: LookLike, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return `${lookTitle(l)} ${lookBlurb(l)} ${l.title} ${l.blurb}`.toLowerCase().includes(q);
}

/** A built-in preset's name and description, in the user's language. */
export function presetName(id: string, fallback: string): string {
  return known(`content.preset.${slug(id)}.name`, fallback);
}

export function presetDescription(id: string, fallback: string): string {
  return known(`content.preset.${slug(id)}.description`, fallback);
}

/** A built-in preset group's name and description, in the user's language. */
export function presetGroupName(id: string, fallback: string): string {
  return known(`content.presetGroup.${slug(id)}.name`, fallback);
}

export function presetGroupDescription(id: string, fallback: string): string {
  return known(`content.presetGroup.${slug(id)}.description`, fallback);
}

/** True when `query` is in a preset's name or description, translated or English. */
export function presetMatches(p: { id: string; name: string; description: string }, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return `${presetName(p.id, p.name)} ${presetDescription(p.id, p.description)} ${p.name} ${p.description}`
    .toLowerCase()
    .includes(q);
}

/**
 * An AI mask's label. The core stores "Subject" / "Sky" in the edit stack, so
 * those are mapped when drawn, never when created; anything else is shown as is.
 */
export function maskLabel(label: string): string {
  if (label === "Subject") return t("content.mask.subject");
  if (label === "Sky") return t("content.mask.sky");
  return label;
}

/** A lighting condition ("golden hour") as the interface names it. The English stays the filter value. */
export function conditionName(english: string): string {
  return known(`content.condition.${slug(english)}`, tagName(english));
}

/** Whether `conditionName` has a fixed translation for `english`. */
export function tableNamesCondition(english: string): boolean {
  return has(`content.condition.${slug(english)}`);
}
