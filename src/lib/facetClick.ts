/**
 * A tag facet's two lists after a click: a plain click chooses or
 * unchooses a value; Alt-click leaves it out, or lets it back in. A value is
 * never both, and a plain click on one that is left out lets it back in —
 * the struck-through row reads as "off", so clicking it should not also
 * choose it.
 */
export function facetClick(
  chosen: string[],
  excluded: string[],
  value: string,
  alt: boolean,
): { chosen: string[]; excluded: string[] } {
  if (excluded.includes(value)) {
    return { chosen, excluded: excluded.filter((v) => v !== value) };
  }
  if (alt) {
    return { chosen: chosen.filter((v) => v !== value), excluded: [...excluded, value] };
  }
  return {
    chosen: chosen.includes(value) ? chosen.filter((v) => v !== value) : [...chosen, value],
    excluded,
  };
}
