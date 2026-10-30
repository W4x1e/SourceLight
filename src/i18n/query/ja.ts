import { LAST_YEAR, THIS_YEAR, type QueryLexicon } from ".";

const MONTHS = [
  "january", "february", "march", "april", "may", "june",
  "july", "august", "september", "october", "november", "december",
];
const SEASON: Record<string, string> = { 春: "spring", 夏: "summer", 秋: "autumn", 冬: "winter", クリスマス: "christmas" };
/** 去年の夏 is the summer of last year — not "last summer", the latest one
 *  over — so these name a year, which the parser resolves (see `LAST_YEAR`). */
const WHEN: Record<string, string> = { 去年: LAST_YEAR, 昨年: LAST_YEAR, 今年: THIS_YEAR };

/**
 * Japanese search words → the English ones the parser reads. Japanese has no
 * spaces, so a query is split with `Intl.Segmenter`, after the phrases below
 * are read whole: 去年の夏 is "last summer", 2024年6月 is "june 2024".
 */
export default {
  segment: true,
  phrases: [
    [/(去年|昨年|今年)の?(春|夏|秋|冬|クリスマス)/, (m) => [SEASON[m[2]!]!, WHEN[m[1]!]!]],
    [/((?:19|20)\d\d)年(1[0-2]|0?[1-9])月/, (m) => [MONTHS[Number(m[2]) - 1]!, m[1]!]],
    [/((?:19|20)\d\d)年の?(春|夏|秋|冬|クリスマス)/, (m) => [SEASON[m[2]!]!, m[1]!]],
    [/((?:19|20)\d\d)年/, (m) => [m[1]!]],
    [/(1[0-2]|0?[1-9])月/, (m) => [MONTHS[Number(m[1]) - 1]!]],
  ],
  words: {
    今日: "today",
    昨日: "yesterday",
    今週: ["this", "week"],
    先週: ["last", "week"],
    今月: ["this", "month"],
    先月: ["last", "month"],
    今年: ["this", "year"],
    去年: ["last", "year"],
    昨年: ["last", "year"],
    春: "spring",
    夏: "summer",
    秋: "autumn",
    冬: "winter",
    クリスマス: "christmas",
    の: "of",
    と: "and",
    で: "in",
    に: "in",
    から: "from",
    写真: "photos",
    画像: "photos",
    全て: "all",
    すべて: "all",
  },
} satisfies QueryLexicon;
