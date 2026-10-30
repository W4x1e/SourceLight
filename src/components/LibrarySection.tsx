import { useEffect } from "react";
import { t } from "../i18n";
import { useStore } from "../store";
import { FolderList } from "./LibraryMenu";
import CatalogueBackups from "./CatalogueBackups";
import OnOff from "./OnOff";
import { trashName } from "../lib/platform";

/**
 * Settings → Library: where the library is, and the catalogue that holds all
 * the work done on it.
 *
 * Backups used to sit at the top of an "Advanced" tab under an "AI & local
 * models" heading — the single control that decides whether someone still has
 * their ratings and edits after something goes wrong, filed under AI.
 * The folder list is the same one as the top bar's folder menu.
 */
export default function LibrarySection() {
  const refreshFolders = useStore((s) => s.refreshFolders);
  const autoBackup = useStore((s) => s.autoBackup);
  const setAutoBackup = useStore((s) => s.setAutoBackup);
  useEffect(() => void refreshFolders(), [refreshFolders]);

  return (
    <div>
      <div className="section-label mb-1.5 px-0">{t("settings.library.folders")}</div>
      <p className="mb-2 text-label leading-relaxed text-faint">
        {t("settings.library.foldersNote", { trash: trashName() })}
      </p>
      <div className="rounded-md bg-raised py-1">
        <FolderList />
      </div>

      <div className="section-label mb-1.5 mt-5 px-0">{t("settings.library.catalogue")}</div>
      <div className="mb-2 flex items-start gap-3 rounded-md bg-raised p-3">
        <div className="min-w-0 flex-1">
          <div className="text-body text-ink">{t("settings.library.dailyBackup.title")}</div>
          <p className="text-label leading-relaxed text-faint">
            {t("settings.library.dailyBackup.blurb")}
          </p>
        </div>
        <OnOff label={t("settings.library.dailyBackup.title")} on={autoBackup} onChange={setAutoBackup} />
      </div>
      <CatalogueBackups />
    </div>
  );
}
