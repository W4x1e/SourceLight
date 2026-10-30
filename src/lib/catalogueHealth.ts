import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import { confirm, toast } from "./dialogs";
import { useStore } from "../store";
import { formatDate, t } from "../i18n";

/**
 * What the user is told when the catalogue is found damaged.
 *
 * Startup restores the newest good backup by itself (`db::open_or_recover`):
 * this says that it happened, what it cost, and where the damaged file went.
 * Deeper damage, found by the background check after startup, asks first,
 * because restoring then throws away the session in progress.
 */

/** Says what startup did, if it had to recover. Once per launch. */
export async function reportRecovery(): Promise<void> {
  const recovery = await api.catalogueRecovery().catch(() => null);
  if (!recovery) return;
  const restored = recovery.restored;
  const message = restored
    ? t("shell.catalogue.restoredMessage", {
        when: formatDate((restored.modified ?? 0) * 1000, { dateStyle: "medium", timeStyle: "short" }),
        path: recovery.damagedCopy,
      })
    : t("shell.catalogue.newMessage", { path: recovery.damagedCopy });
  const ok = await confirm({
    title: t("shell.catalogue.damagedTitle"),
    message,
    confirmLabel: t("common.ok"),
    cancelLabel: restored ? t("shell.catalogue.seeBackups") : t("shell.catalogue.addFolders"),
  });
  if (!ok) useStore.getState().openSettings("library");
}

/** Offers the newest good backup when the background check finds damage. */
export function watchForDamage(): Promise<() => void> {
  return listen<string>("catalogue-damaged", async () => {
    const restore = await confirm({
      title: t("shell.catalogue.foundDamageTitle"),
      message: t("shell.catalogue.foundDamageMessage"),
      confirmLabel: t("shell.catalogue.restoreAndRestart"),
      cancelLabel: t("shell.catalogue.notNow"),
      danger: true,
    });
    if (!restore) return;
    try {
      await api.restoreNewestBackup();
      await api.restartApp();
    } catch (e) {
      toast(t("shell.catalogue.restoreFailed", { error: String(e) }), "error");
    }
  });
}
