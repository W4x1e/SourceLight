/**
 * The tagger's words in the user's language.
 *
 * Subject tags and genres are the built-in tagger's fixed English vocabulary
 * (`src-tauri/resources/tagger/subjects.txt` and `GENRES` in models.rs), and
 * they are stored and filtered on in English: that is the data. These tables
 * only change what is shown — and let a search typed in the user's language
 * find the English tag it means. A tag not in the table (one an LLM tagger
 * wrote freely, or one the user typed) is shown as it is.
 */
export type TagNames = Readonly<Record<string, string>>;

const LOADERS: Record<string, () => Promise<TagNames>> = {
  de: () => import("./de").then((m) => m.default),
  es: () => import("./es").then((m) => m.default),
  fr: () => import("./fr").then((m) => m.default),
  ja: () => import("./ja").then((m) => m.default),
};

let names: TagNames = {};
/** Names the local model gave the tags no table covers (`lib/tagNames`). */
let learned: TagNames = {};
let reverse: ReadonlyMap<string, string> = new Map();
/** The pseudo-locale marks every tag, so a place that shows one raw stands out. */
let mark: ((s: string) => string) | null = null;

/** Loads the table for `locale` (called by `setLocale`); none for English. */
export async function loadTagNames(locale: string, pseudo?: (s: string) => string): Promise<void> {
  const next: TagNames = locale in LOADERS ? await LOADERS[locale]!().catch(() => ({})) : {};
  names = next;
  learned = {};
  index(locale);
  mark = locale === "en-XA" && pseudo ? pseudo : null;
}

function index(locale: string) {
  reverse = new Map(
    [...Object.entries(names), ...Object.entries(learned)].map(([en, local]) => [
      local.toLocaleLowerCase(locale),
      en,
    ]),
  );
}

/** Whether the fixed table names `tag` in the active language. */
export function tableNamesTag(tag: string): boolean {
  return tag in names;
}

/** Takes the names the local model gave tags outside the table, for `locale`. */
export function setLearnedTagNames(locale: string, next: TagNames): void {
  learned = next;
  index(locale);
}

/** A tag as the user reads it: "dog" is "Hund" in German. */
export function tagName(tag: string): string {
  if (mark) return mark(tag);
  return names[tag] ?? learned[tag] ?? tag;
}

/**
 * The English tag a word in the user's language names, if it names one:
 * "hund" → "dog". Case-insensitive. Lets a search typed in German reach tags
 * that are stored in English.
 */
export function tagFromLocal(word: string): string | undefined {
  return reverse.get(word.toLocaleLowerCase());
}
