import type { PhotoFilter } from "../types";
import { isFilterEmpty } from "../types";
import { t } from "../i18n";

/**
 * What Cull is working on, in the words its header shows: a shoot, a
 * search, a collection or folder, any other filter, or the whole library.
 * Cull culled whatever Browse last showed — often everything — with nothing
 * saying so, and its bulk actions reached across earlier shoots.
 */
export function cullScopeLabel(
  filter: PhotoFilter,
  searchLabel: string,
  names: { shoot?: string; collection?: string; folder?: string },
): string {
  if (searchLabel) return searchLabel;
  if (filter.shootId != null && names.shoot) return names.shoot;
  if (filter.semantic) return t("cull.scope.search", { query: filter.semantic });
  if (filter.collectionId != null && names.collection) return names.collection;
  if ((filter.folderIds?.length ?? 0) === 1 && names.folder) return names.folder;
  return isFilterEmpty(filter) ? t("cull.scope.library") : t("cull.scope.filtered");
}

/**
 * The key a cull position is saved under: the filter and any transient
 * search, without the sort. A position saved on one shoot is not a position
 * on another — resuming there dropped someone at the old frame number of a
 * different set, with "Picked up where you left off".
 */
export function cullScopeKey(filter: PhotoFilter, searchLabel: string): string {
  const { sort: _sort, ...rest } = filter;
  return JSON.stringify({ ...rest, searchLabel });
}
