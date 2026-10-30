import { api } from "./api";
import { confirm } from "./dialogs";
import { t } from "../i18n";

type Options = Parameters<typeof api.exportSpaceCheck>[2];

/**
 * Asks before a batch export that looks likely to fill its drive.
 *
 * Resolves `true` to go ahead. The size is an estimate that leans high, so this
 * warns rather than refuses — the user may be about to free space, or know
 * better. And it never stands in the way of an export on its own account: if
 * the check itself fails, the export proceeds exactly as it did before.
 */
export async function roomToExport(
  photoIds: number[],
  directory: string,
  options: Options,
): Promise<boolean> {
  let warning;
  try {
    warning = await api.exportSpaceCheck(photoIds, directory, options);
  } catch (e) {
    console.warn("free-space check failed; exporting anyway:", e);
    return true;
  }
  if (!warning) return true;
  return confirm({
    title: t("transfer.space.title"),
    message: t("transfer.space.message", { needed: warning.needed, free: warning.free }),
    confirmLabel: t("transfer.space.confirm"),
    cancelLabel: t("common.cancel"),
  });
}
