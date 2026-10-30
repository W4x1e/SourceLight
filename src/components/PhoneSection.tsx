import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open as openDirectoryDialog } from "@tauri-apps/plugin-dialog";
import { formatDate, t, type Key } from "../i18n";
import { api } from "../lib/api";
import { confirm } from "../lib/dialogs";
import type { PhoneImportFolder, PhoneLinkStatus, PhonePairing, PhonePairRequest } from "../bindings";
import OnOff from "./OnOff";

/**
 * Settings → Phone (docs/MOBILE.md): pair phones with this library, and
 * choose whether they can reach it away from home.
 *
 * The tab only exists in a build with the phone link (the `phone-link` Cargo
 * feature); `status.available` says which. A pairing code is shown inline
 * rather than in a dialog of its own: Settings is already a modal, and the
 * "allow this phone?" question that follows a scan is a confirm on top.
 */

const PLATFORM: Record<string, Key> = {
  ios: "settings.phone.platform.ios",
  android: "settings.phone.platform.android",
};

/** SQLite's `datetime('now')`, which is UTC without saying so. */
function sqliteDate(s: string): string {
  const d = new Date(`${s.replace(" ", "T")}Z`);
  return Number.isNaN(d.getTime()) ? s : formatDate(d, { dateStyle: "medium" });
}

const inTauri = () => "__TAURI_INTERNALS__" in window;

export default function PhoneSection() {
  const [status, setStatus] = useState<PhoneLinkStatus | null>(null);
  const [pairing, setPairing] = useState<PhonePairing | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [importTo, setImportTo] = useState<PhoneImportFolder | null>(null);

  const refresh = useCallback(() => {
    api
      .phoneLinkStatus()
      .then(setStatus)
      .catch((e) => setError(String(e)));
    api
      .phoneImportFolder()
      .then(setImportTo)
      .catch(() => setImportTo(null));
  }, []);

  /** Where photos added from a phone are saved (Add to library). */
  const chooseImportFolder = async () => {
    const picked = await openDirectoryDialog({ directory: true, title: t("settings.phone.importTo.pickTitle") });
    if (typeof picked !== "string") return;
    try {
      await api.setPhoneImportFolder(picked);
      refresh();
    } catch (e) {
      setError(String(e));
    }
  };
  const defaultImportFolder = async () => {
    try {
      await api.setPhoneImportFolder(null);
      refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    refresh();
    if (!inTauri()) return;
    const un = listen("phone-link-changed", refresh);
    return () => void un.then((f) => f());
  }, [refresh]);

  // A phone scanned the code and waits for an answer. Asked here because the
  // code is only ever on screen in this section.
  const pairingRef = useRef(pairing);
  pairingRef.current = pairing;
  useEffect(() => {
    if (!inTauri()) return;
    const un = listen<PhonePairRequest>("phone-pair-request", async (e) => {
      const phone = e.payload;
      const allow = await confirm({
        title: t("settings.phone.allowTitle", { name: phone.deviceName }),
        message: t("settings.phone.allowMessage"),
        confirmLabel: t("settings.phone.allow"),
        cancelLabel: t("settings.phone.decline"),
      });
      await api.answerPhonePairing(phone.id, allow);
      // The code is spent either way.
      setPairing(null);
      refresh();
    });
    return () => void un.then((f) => f());
  }, [refresh]);

  // Leaving the tab (or closing Settings) retires a code still on screen, so
  // nobody can pair with a code no one is watching.
  useEffect(
    () => () => {
      if (pairingRef.current) void api.cancelPhonePairing();
    },
    [],
  );

  // The code expires on the desktop; take it off screen at the same moment.
  useEffect(() => {
    if (!pairing) return;
    const timer = window.setTimeout(() => setPairing(null), pairing.expiresInSecs * 1000);
    return () => window.clearTimeout(timer);
  }, [pairing]);

  const setFlags = async (enabled: boolean, awayFromHome: boolean) => {
    setBusy(true);
    try {
      setStatus(await api.setPhoneLink(enabled, awayFromHome));
      setError(null);
      if (!enabled) setPairing(null);
    } catch (e) {
      setError(String(e));
      refresh();
    } finally {
      setBusy(false);
    }
  };

  const pair = async () => {
    setBusy(true);
    setCopied(false);
    try {
      setPairing(await api.startPhonePairing());
      setError(null);
      refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const cancel = () => {
    setPairing(null);
    void api.cancelPhonePairing();
  };

  const remove = async (id: string, name: string) => {
    const ok = await confirm({
      title: t("settings.phone.removeTitle", { name }),
      message: t("settings.phone.removeMessage"),
      confirmLabel: t("settings.phone.remove"),
      danger: true,
    });
    if (!ok) return;
    try {
      setStatus(await api.removePhone(id));
    } catch (e) {
      setError(String(e));
    }
  };

  const copy = async () => {
    if (!pairing) return;
    await navigator.clipboard.writeText(pairing.uri);
    setCopied(true);
  };

  if (!status) return error ? <p className="text-label text-danger">{error}</p> : null;

  const minutes = Math.round((pairing?.expiresInSecs ?? 300) / 60);

  return (
    <div>
      <div className="flex items-start gap-3 rounded-md bg-raised p-3">
        <div className="min-w-0 flex-1">
          <div className="text-caption font-medium text-ink">{t("settings.phone.link.title")}</div>
          <p className="mt-0.5 text-label leading-relaxed text-faint">{t("settings.phone.link.blurb")}</p>
        </div>
        <OnOff
          label={t("settings.phone.link.title")}
          on={status.enabled}
          disabled={busy}
          onChange={(v) => void setFlags(v, status.awayFromHome)}
        />
      </div>

      {/* A card like the switch above it, its name the same size: it was a
          bare row a step larger. */}
      <div className={`mt-2 flex items-start gap-3 rounded-md bg-raised p-3 ${status.enabled ? "" : "opacity-60"}`}>
        <div className="min-w-0 flex-1">
          <div className="text-caption font-medium text-ink">{t("settings.phone.away.title")}</div>
          <p className="mt-0.5 text-label leading-relaxed text-faint">{t("settings.phone.away.blurb")}</p>
        </div>
        <OnOff
          label={t("settings.phone.away.title")}
          on={status.awayFromHome}
          disabled={busy}
          onChange={(v) => void setFlags(status.enabled, v)}
        />
      </div>

      {status.enabled && !status.running && status.error && (
        <p className="mt-2 text-label text-danger">{t("settings.phone.notRunning", { error: status.error })}</p>
      )}

      <div className="mt-5">
        {pairing ? (
          <div className="flex items-start gap-4 rounded-md bg-raised p-3">
            {/* Dark on white whatever the theme: that is what a camera reads. */}
            <img
              className="h-40 w-40 shrink-0 rounded bg-white p-1.5"
              src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(pairing.qrSvg)}`}
              alt={t("settings.phone.qrAlt")}
            />
            <div className="min-w-0 flex-1">
              <p className="text-body text-ink">{t("settings.phone.scan")}</p>
              <p className="mt-1 text-label leading-relaxed text-faint">
                {t("settings.phone.expires", { count: minutes })}
              </p>
              {!pairing.reachableAway && (
                <p className="mt-1 text-label leading-relaxed text-faint">{t("settings.phone.homeOnly")}</p>
              )}
              <div className="mt-3 flex gap-2">
                <button className="btn-solid" onClick={cancel}>
                  {t("common.cancel")}
                </button>
                <button className="btn-ghost text-muted" onClick={() => void copy()}>
                  {copied ? t("settings.phone.copied") : t("settings.phone.copy")}
                </button>
              </div>
            </div>
          </div>
        ) : (
          // Solid, not the key: Settings' Done is the one key in the dialog.
          <button className="btn-solid" disabled={busy} onClick={() => void pair()}>
            {t("settings.phone.pair")}
          </button>
        )}
      </div>

      <div className="mt-5">
        <div className="section-label mb-1.5 px-0">{t("settings.phone.phones.title")}</div>
        {status.phones.length === 0 ? (
          <p className="text-label text-faint">{t("settings.phone.phones.none")}</p>
        ) : (
          <ul className="space-y-1">
            {status.phones.map((p) => (
              <li key={p.id} className="flex items-center gap-3 rounded-md px-1 py-1.5">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-caption font-medium text-ink">{p.name}</div>
                  <div className="text-label text-faint">
                    {t(PLATFORM[p.platform] ?? "settings.phone.platform.other")}
                    {" · "}
                    {p.lastSeen
                      ? t("settings.phone.lastSeen", { date: sqliteDate(p.lastSeen) })
                      : t("settings.phone.notSeen")}
                  </div>
                </div>
                <button className="btn-ghost shrink-0 text-muted" onClick={() => void remove(p.id, p.name)}>
                  {t("settings.phone.remove")}
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>

      {importTo && (
        <div className="mt-5">
          <div className="section-label mb-1.5 px-0">{t("settings.phone.importTo.title")}</div>
          <p className="text-label leading-relaxed text-faint">{t("settings.phone.importTo.blurb")}</p>
          <div className="mt-1.5 flex items-center gap-2">
            <div className="min-w-0 flex-1 truncate text-label text-ink" title={importTo.path}>
              {importTo.path}
            </div>
            <button className="btn-ghost shrink-0 text-muted" onClick={() => void chooseImportFolder()}>
              {t("settings.phone.importTo.change")}
            </button>
            {importTo.chosen && (
              <button className="btn-ghost shrink-0 text-muted" onClick={() => void defaultImportFolder()}>
                {t("settings.phone.importTo.default")}
              </button>
            )}
          </div>
        </div>
      )}

      {error && <p className="mt-2 text-label text-danger">{error}</p>}
    </div>
  );
}
