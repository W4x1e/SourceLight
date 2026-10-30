import { api } from "./api";
import { confirm } from "./dialogs";
import { t } from "../i18n";

/**
 * Says so when the last session never exited: the computer froze or lost
 * power, or the app was ended. Its log was kept (`session_log.rs`), and this
 * offers the folder it is in. Once per launch.
 */
export async function reportUncleanExit(): Promise<void> {
  const log = await api.uncleanExit().catch(() => null);
  if (!log) return;
  const open = await confirm({
    title: t("shell.uncleanExit.title"),
    message: t("shell.uncleanExit.message", { file: log }),
    confirmLabel: t("settings.diagnostics.openLogs"),
    cancelLabel: t("common.close"),
  });
  if (open) await api.openLogFolder().catch(() => undefined);
}
