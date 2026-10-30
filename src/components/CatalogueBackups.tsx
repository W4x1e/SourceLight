import { useCallback, useEffect, useState } from "react";
import { formatDate, t } from "../i18n";
import { api } from "../lib/api";
import type { BackupInfo } from "../bindings";
import { confirm, toast } from "../lib/dialogs";
import { humanBytes } from "../lib/format";
import { Icon } from "../lib/icons";

/**
 * Backing up and restoring the catalogue.
 *
 * The library database holds every rating, flag, tag, place, trip, collection,
 * person and develop edit — and holds them nowhere else. Photos are safe
 * because the app never writes to them; the *work* is what needs a copy, and
 * until now there was none.
 *
 * A restore does not happen when it is clicked. The app has the database open,
 * and swapping the file out from under a live SQLite handle is how a
 * recoverable mistake becomes a corrupt one, so it is staged and applied at the
 * next launch. That has an interface consequence worth being explicit about:
 * the user has to be told nothing has changed yet, and given a way to take it
 * back.
 */
/**
 * When a backup was taken, as a person would read it.
 *
 * `modified` is `number | null` because specta renders Rust's f64 that way —
 * the type admits NaN and infinities that JSON cannot carry. A filesystem that
 * declines to report a time comes through as 0 rather than a crash.
 */
function whenOf(b: BackupInfo): string {
  const secs = b.modified ?? 0;
  // "1 Sep 2025, 11:00" rather than "9/1/2025, 11:00:00 AM": the seconds are
  // noise, and a numeric date reads differently either side of the Atlantic.
  return secs > 0
    ? formatDate(secs * 1000, { dateStyle: "medium", timeStyle: "short" })
    : t("settings.backups.dateUnknown");
}

/**
 * Why a backup exists, from its filename (`backup::create` names it after the
 * reason): the daily copy, one before an upgrade or another risky change, or
 * one the user asked for.
 */
export function takenWhy(b: BackupInfo): string {
  if (!b.automatic) return t("settings.backups.by.you");
  if (b.filename.startsWith("library-daily")) return t("settings.backups.by.daily");
  if (/^library-auto-v\d/.test(b.filename)) return t("settings.backups.by.upgrade");
  if (b.filename.startsWith("library-auto-before-retag")) return t("settings.backups.by.retag");
  return t("settings.backups.by.change");
}

export default function CatalogueBackups() {
  const [backups, setBackups] = useState<BackupInfo[] | null>(null);
  const [pending, setPending] = useState(false);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [list, staged] = await Promise.all([
        api.listCatalogueBackups(),
        api.catalogueRestorePending(),
      ]);
      setBackups(list);
      setPending(staged);
      setErr(null);
    } catch (e) {
      setErr(String(e));
      setBackups([]);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const backUpNow = async () => {
    setBusy(true);
    setErr(null);
    try {
      await api.backupCatalogue();
      toast(t("settings.backups.done"), "success");
      await refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const restore = async (b: BackupInfo) => {
    const when = whenOf(b);
    const agreed = await confirm({
      title: t("settings.backups.restoreTitle"),
      message: t("settings.backups.restoreMessage", { when }),
      confirmLabel: t("settings.backups.restoreConfirm"),
      danger: true,
    });
    if (!agreed) return;

    setBusy(true);
    setErr(null);
    try {
      await api.restoreCatalogueBackup(b.filename);
      await refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const cancelRestore = async () => {
    setBusy(true);
    try {
      await api.cancelCatalogueRestore();
      await refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div>
      <p className="text-caption leading-relaxed text-muted">
        {t("settings.backups.intro")}
      </p>

      {pending && (
        <div className="mt-2 flex items-center gap-2 rounded-md bg-raised px-3 py-2">
          <Icon name="warn" size={14} className="shrink-0 text-accent" />
          <span className="min-w-0 flex-1 text-caption text-ink">
            {t("settings.backups.pending")}
          </span>
          <button onClick={() => void cancelRestore()} disabled={busy} className="btn-ghost shrink-0">
            {t("common.cancel")}
          </button>
        </div>
      )}

      <div className="mt-2 flex items-center gap-2">
        <button onClick={() => void backUpNow()} disabled={busy} className="btn-solid">
          {t("settings.backups.backUpNow")}
        </button>
        {backups !== null && (
          <span className="text-label text-faint">
            {backups.length === 0
              ? t("settings.backups.none")
              : t("settings.backups.count", { count: backups.length })}
          </span>
        )}
      </div>

      {err && <div className="mt-1.5 text-label text-danger">{err}</div>}

      {backups && backups.length > 0 && (
        <ul className="mt-2 divide-y divide-line rounded-md bg-raised">
          {backups.map((b) => (
            <li key={b.path} className="flex items-center gap-2 px-3 py-2">
              <div className="min-w-0 flex-1">
                <div className="truncate text-caption text-ink" title={b.path}>
                  {whenOf(b)}
                </div>
                <div className="text-label text-faint">
                  {t("settings.backups.detail", { size: humanBytes(b.bytes ?? 0), why: takenWhy(b) })}
                </div>
              </div>
              <button onClick={() => void restore(b)} disabled={busy} className="btn-ghost shrink-0">
                {t("settings.backups.restore")}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
