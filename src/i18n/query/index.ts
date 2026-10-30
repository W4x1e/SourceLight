/**
 * The search box's words in other languages (docs/I18N.md).
 *
 * `lib/queryReading` understands a query through a small English vocabulary:
 * "last summer", "in", "photos of", "yesterday". Rather than a parser per
 * language, each language maps its own words onto those English ones — a
 * word's *canonical* form — and the parser reads the canonical forms. What the
 * user typed is kept for display, so a chip still says "letzten Sommer".
 *
 * Keys are lower-case with accents removed (see `fold`), so "Été", "ete" and
 * "été" all match. A value of several words stands for a phrase: Japanese
 * 先週 is "last week". Month names are not listed — they come from `Intl` for
 * whichever language is active.
 */
export interface QueryLexicon {
  /** Local word → the English word(s) the parser knows. */
  words: Readonly<Record<string, string | readonly string[]>>;
  /** Contractions to take off the front of a word: French "l'été" is "été". */
  elisions?: readonly string[];
  /**
   * Whole phrases to read before splitting into words, for a language that
   * does not put spaces between them: Japanese 去年の夏 is "last summer".
   */
  phrases?: readonly (readonly [RegExp, (m: RegExpExecArray) => readonly string[]])[];
  /** "Last" and "this" may follow their noun: French "l'été dernier". */
  postfix?: boolean;
  /** Split with `Intl.Segmenter` rather than on spaces. */
  segment?: boolean;
}

/**
 * Words a lexicon may give for a year relative to today, where a language
 * names one that way (Japanese 去年, "the year before this one").
 */
export const LAST_YEAR = "@lastyear";
export const THIS_YEAR = "@thisyear";

/**
 * Lower-case, Latin accents removed: the form lexicon keys and query words
 * are compared in. Only the combining accents Latin letters take are dropped —
 * Japanese kana decompose too (ガ is カ plus a mark), and must come back whole.
 */
export function fold(word: string): string {
  return word
    .toLocaleLowerCase()
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .normalize("NFC");
}
