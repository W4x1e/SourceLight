/**
 * Which interface strings each language has not translated yet.
 *
 *   npm run i18n:missing          # a count per language and area
 *   npm run i18n:missing -- de    # ...and every missing German key, with its English
 *
 * A missing key is not a bug — the interface shows the English until it is
 * translated (docs/I18N.md) — so this is a to-do list, not a check, and it
 * never fails.
 */
import { AREAS } from "../src/i18n/en";
import type { Catalogue, Message } from "../src/i18n";
import de from "../src/i18n/de";
import es from "../src/i18n/es";
import fr from "../src/i18n/fr";
import ja from "../src/i18n/ja";

const LANGUAGES: Record<string, Catalogue> = { de, es, fr, ja };
const detail = process.argv[2];

const show = (m: Message) => (typeof m === "string" ? m : m.other);

for (const [code, catalogue] of Object.entries(LANGUAGES)) {
  if (detail && detail !== code) continue;
  const rows: string[] = [];
  let missing = 0;
  let total = 0;
  for (const [area, strings] of Object.entries(AREAS)) {
    const keys = Object.keys(strings) as (keyof Catalogue)[];
    const absent = keys.filter((k) => !(k in catalogue));
    total += keys.length;
    missing += absent.length;
    if (absent.length) rows.push(`  ${area.padEnd(10)} ${absent.length} of ${keys.length}`);
    if (detail) for (const k of absent) rows.push(`    ${k}  ${JSON.stringify(show((strings as Record<string, Message>)[k]!))}`);
  }
  console.log(`${code}: ${missing ? `${missing} of ${total} missing` : `all ${total} translated`}`);
  for (const r of rows) console.log(r);
}
