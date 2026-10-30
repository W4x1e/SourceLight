import { formatNumber, intlLocale, t } from "../i18n";

/**
 * A machine label with its first letter capitalised, in the interface's
 * language. CSS `capitalize` upper-cased every word: "Photo De Rue",
 * "Coucher De Soleil", and a keyword typed "iPhone" became "IPhone".
 */
export function sentenceCase(s: string): string {
  return s ? s.charAt(0).toLocaleUpperCase(intlLocale()) + s.slice(1) : s;
}

/**
 * Small display formatters shared across the UI.
 *
 * Kept pure and separate so they can be unit-tested without a component — the
 * same split the rest of the codebase uses for logic that has an easy-to-get-
 * wrong edge (here: the unit boundaries, and a zero/placeholder total).
 */

/**
 * Bytes as a short human string — "512 B", "64 KB", "608 MB", "6.0 GB".
 *
 * Downloads report raw byte counts, and a bare percentage is indistinguishable
 * from a hang on a multi-gigabyte pull; pairing the bar with "1.4 GB of 6.0 GB"
 * is what tells the user it is still moving. Binary units (1024), matching how
 * the model catalogue quotes its sizes.
 */
export function humanBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes < 1024) return t("library.bytes.b", { value: formatNumber(Math.round(bytes), { useGrouping: false }) });
  const kb = bytes / 1024;
  if (kb < 1024) return t("library.bytes.kb", { value: formatNumber(Math.round(kb), { useGrouping: false }) });
  const mb = kb / 1024;
  if (mb < 1024) return t("library.bytes.mb", { value: formatNumber(Math.round(mb), { useGrouping: false }) });
  return t("library.bytes.gb", {
    value: formatNumber(mb / 1024, { minimumFractionDigits: 1, maximumFractionDigits: 1, useGrouping: false }),
  });
}
