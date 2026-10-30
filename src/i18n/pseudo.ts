import type { Catalogue, Key, Message } from ".";

/**
 * The pseudo-locale, `en-XA`: English with every letter accented, wrapped in
 * brackets and padded a third longer. A string still hard-coded in English is
 * the only plain text on screen, a clipped bracket shows where a longer
 * translation will not fit, and the accents show a font that lacks them.
 * Placeholders are left alone so the values still land.
 */

const ACCENTED: Record<string, string> = Object.fromEntries(
  [..."abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"].map((c, i) => [
    c,
    [..."åƀçđéƒĝĥîĵķļɱñöþǫŕšţûṽŵẋýžÅƁÇĐÉƑĜĤÎĴĶĻṀÑÖÞǪŔŠŢÛṼŴẊÝŽ"][i]!,
  ]),
);

export function pseudo(text: string): string {
  const body = text
    .split(/(\{\w+\})/)
    .map((part) => (part.startsWith("{") ? part : [...part].map((c) => ACCENTED[c] ?? c).join("")))
    .join("");
  const pad = "·".repeat(Math.ceil(text.length / 3));
  return `[${body}${pad}]`;
}

export function pseudoCatalogue(source: Record<Key, Message>): Catalogue {
  const out: Partial<Record<Key, Message>> = {};
  for (const [key, msg] of Object.entries(source) as [Key, Message][]) {
    out[key] =
      typeof msg === "string"
        ? pseudo(msg)
        : (Object.fromEntries(Object.entries(msg).map(([f, m]) => [f, pseudo(m)])) as Message);
  }
  return out;
}
