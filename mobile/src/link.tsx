import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { formatDate, t } from "../../src/i18n";
import { Icon } from "../../src/lib/icons";
import { api, type SyncSummary, type Waiting } from "./api";
import { pushStep, replaceStep } from "./back";
import { ConfirmSheet, MenuHeading, MenuItem, Sheet, StatusDot } from "./ui";

/** Where the link to the computer stands. */
export type Conn =
  | { state: "connecting" }
  | { state: "syncing" }
  | { state: "up"; relayed: boolean; millis: number }
  | { state: "down"; error: string };

/** The tests, opened from the version line in the sync sheet. */
/** What the sync sheet opens over everything: a test, or the licences. */
export type Test = "gpuTest" | "scrollTest" | "licences";

type LinkValue = {
  /** The paired computer's name; null when none is. */
  desktop: string | null;
  conn: Conn;
  /** When the phone last synced, ms since the epoch. */
  lastSynced: number | null;
  /** Syncs now, connecting first if the computer couldn't be reached. */
  syncNow: () => Promise<void>;
  /** Told of each sync's summary; returns the way to stop. */
  onSynced: (fn: (s: SyncSummary) => void) => () => void;
  /** Opens the sync sheet. */
  openSheet: () => void;
};

const LinkContext = createContext<LinkValue | null>(null);

export function useLink(): LinkValue {
  const value = useContext(LinkContext);
  if (!value) throw new Error("useLink outside LinkProvider");
  return value;
}

/** How often a connected phone syncs, and an unconnected one tries again. */
const TICK_MS = 10_000;

/**
 * The link to the computer, for every tab: it connects once the app opens,
 * then every ten seconds syncs (picking up changes made on the computer and
 * sending anything waiting here) or, not connected, quietly tries again, so a
 * computer that was asleep is found without a tap. Sync now does the same at
 * once. Each sync's summary goes to whoever listens (the library loads its
 * tiles and says what landed).
 *
 * The sync sheet, from the computer button at the top of each tab, says how
 * it stands and when it last synced, what is waiting to go, Sync now, and
 * Forget computer (or Pair); its version line, held, opens the tests.
 */
export function LinkProvider({
  desktop,
  onPair,
  onForgot,
  onTest,
  children,
}: {
  desktop: string | null;
  /** Pair with a computer, from the sheet when none is paired. */
  onPair: () => void;
  /** The computer was forgotten. */
  onForgot: () => void;
  onTest: (test: Test) => void;
  children: ReactNode;
}) {
  const paired = desktop !== null;
  const [conn, setConn] = useState<Conn>({ state: "connecting" });
  const [lastSynced, setLastSynced] = useState<number | null>(null);
  const listeners = useRef(new Set<(s: SyncSummary) => void>());
  const tell = (s: SyncSummary) => listeners.current.forEach((fn) => fn(s));
  const onSynced = useCallback((fn: (s: SyncSummary) => void) => {
    listeners.current.add(fn);
    return () => void listeners.current.delete(fn);
  }, []);

  useEffect(() => {
    if (paired) void api.linkStatus().then((s) => setLastSynced(s.lastSyncedMs)).catch(() => {});
  }, [paired]);

  const connRef = useRef(conn);
  connRef.current = conn;
  // One at a time: the ticks, Sync now and connecting don't overlap.
  const busy = useRef(false);
  const connect = useCallback(async () => {
    setConn({ state: "connecting" });
    try {
      const c = await api.connect();
      setConn({ state: "syncing" });
      tell(await api.sync());
      setLastSynced(Date.now());
      setConn({ state: "up", relayed: c.relayed, millis: c.millis });
    } catch (e) {
      setConn({ state: "down", error: String(e) });
    }
  }, []);
  const sync = useCallback(
    async (shown: boolean) => {
      if (busy.current) return;
      busy.current = true;
      try {
        if (connRef.current.state === "down" || connRef.current.state === "connecting") {
          await connect();
          return;
        }
        if (shown) setConn({ state: "syncing" });
        const p = await api.ping();
        tell(await api.sync());
        setLastSynced(Date.now());
        setConn({ state: "up", relayed: p.relayed, millis: Math.round(p.millis) });
      } catch (e) {
        setConn({ state: "down", error: String(e) });
      } finally {
        busy.current = false;
      }
    },
    [connect],
  );

  useEffect(() => {
    if (!paired) return;
    busy.current = true;
    void connect().finally(() => (busy.current = false));
    const timer = window.setInterval(() => void sync(false), TICK_MS);
    return () => window.clearInterval(timer);
  }, [paired, connect, sync]);

  const syncNow = useCallback(() => sync(true), [sync]);

  const [sheet, setSheet] = useState(false);
  // The sheet takes a Back step, as the menus do.
  const openSheet = useCallback(() => {
    pushStep("syncSheet");
    setSheet(true);
  }, []);
  useEffect(() => {
    const onPop = () => setSheet(Boolean((history.state as { syncSheet?: boolean } | null)?.syncSheet));
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  return (
    <LinkContext.Provider value={{ desktop, conn, lastSynced, syncNow, onSynced, openSheet }}>
      {children}
      {sheet && (
        <SyncSheet
          desktop={desktop}
          conn={paired ? conn : null}
          lastSynced={lastSynced}
          onSyncNow={syncNow}
          onClose={() => history.back()}
          onPair={() => {
            history.back();
            onPair();
          }}
          onForget={async () => {
            await api.forget();
            history.back();
            onForgot();
          }}
          onTest={(test) => {
            replaceStep(test);
            setSheet(false);
            onTest(test);
          }}
        />
      )}
    </LinkContext.Provider>
  );
}

/** The computer button at the top right of each tab: how the link stands at
 *  a glance (a light in its corner; the sync arrows turning while it
 *  syncs), and the sync sheet when tapped. */
export function SyncButton() {
  const { desktop, conn, openSheet } = useLink();
  const syncing = desktop !== null && (conn.state === "syncing" || conn.state === "connecting");
  const tone = desktop === null ? null : conn.state === "up" ? "positive" : conn.state === "down" ? "danger" : "warning";
  return (
    <button
      type="button"
      className="relative flex h-12 w-12 shrink-0 items-center justify-center rounded-md text-ink active:bg-raised"
      aria-label={t("mobile.sync.open")}
      title={t("mobile.sync.open")}
      onClick={openSheet}
    >
      <Icon name={syncing ? "sync" : "desktop"} size={22} className={syncing ? "spin" : ""} />
      {tone && !syncing && (
        <span className="absolute top-3 right-2.5 rounded-full bg-base p-0.5">
          <StatusDot tone={tone} />
        </span>
      )}
    </button>
  );
}

/** "Synced just now", "Synced 4 minutes ago", "Last synced at 14:32" or
 *  with the date when it wasn't today. */
export function syncedText(ms: number, now = Date.now()): string {
  const minutes = Math.floor((now - ms) / 60_000);
  if (minutes < 1) return t("mobile.sync.justNow");
  if (minutes < 60) return t("mobile.sync.minutesAgo", { count: minutes });
  const when = new Date(ms);
  return new Date(now).toDateString() === when.toDateString()
    ? t("mobile.link.lastSyncedToday", { time: formatDate(when, { hour: "numeric", minute: "2-digit" }) })
    : t("mobile.link.lastSynced", {
        when: formatDate(when, { day: "numeric", month: "short", hour: "numeric", minute: "2-digit" }),
      });
}

/** What is waiting to go, as "3 ratings, 1 edit and 2 photos", or null. */
function waitingText(w: Waiting): string | null {
  const parts = [
    w.decisions > 0 ? t("mobile.sync.waitingDecisions", { count: w.decisions }) : null,
    w.edits > 0 ? t("mobile.sync.waitingEdits", { count: w.edits }) : null,
    w.adds > 0 ? t("mobile.sync.waitingAdds", { count: w.adds }) : null,
    w.tracks > 0 ? t("mobile.sync.waitingTracks", { count: w.tracks }) : null,
  ].filter((p): p is string => p !== null);
  if (parts.length === 0) return null;
  return new Intl.ListFormat(undefined, { type: "conjunction" }).format(parts);
}

/** How long a press on the version line opens the tests. */
const HOLD_MS = 600;

function SyncSheet({
  desktop,
  conn,
  lastSynced,
  onSyncNow,
  onClose,
  onPair,
  onForget,
  onTest,
}: {
  desktop: string | null;
  /** Null when no computer is paired. */
  conn: Conn | null;
  lastSynced: number | null;
  onSyncNow: () => Promise<void>;
  onClose: () => void;
  onPair: () => void;
  onForget: () => Promise<void>;
  onTest: (test: Test) => void;
}) {
  const [waiting, setWaiting] = useState<Waiting | null>(null);
  const [confirmForget, setConfirmForget] = useState(false);
  const [tests, setTests] = useState(false);
  const [version, setVersion] = useState<string | null>(null);
  const hold = useRef(0);
  useEffect(() => {
    void getVersion()
      .then(setVersion)
      .catch(() => {});
  }, []);
  // What is waiting, now and after each sync while the sheet is up.
  useEffect(() => {
    if (!conn) return;
    void api
      .linkWaiting()
      .then(setWaiting)
      .catch(() => {});
  }, [conn, lastSynced]);
  // "Synced 2 minutes ago" moves on while the sheet is open.
  const [, setNow] = useState(0);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, []);

  const syncing = conn?.state === "syncing" || conn?.state === "connecting";
  const [tone, state] = !conn
    ? (["warning", t("mobile.link.notPaired")] as const)
    : conn.state === "up"
      ? ([
          "positive",
          conn.relayed ? t("mobile.sync.connectedRelay") : t("mobile.sync.connected", { millis: conn.millis }),
        ] as const)
      : conn.state === "down"
        ? (["danger", t("mobile.sync.unreachable")] as const)
        : (["warning", conn.state === "syncing" ? t("mobile.sync.syncing") : t("mobile.link.connecting")] as const);
  const pending = waiting ? waitingText(waiting) : null;

  return (
    <>
    <Sheet label={t("mobile.sync.title")} onClose={onClose}>
      <div className="px-5 pb-2">
        <div className="flex items-center gap-3">
          <Icon name="desktop" size={28} strokeWidth={1.5} className="shrink-0 text-muted" />
          <div className="min-w-0 flex-1">
            <h2 className="truncate font-display text-title text-ink">{desktop ?? t("mobile.library.unpairedTitle")}</h2>
            <div className="mt-0.5 flex items-center gap-2 text-caption text-muted">
              <StatusDot tone={tone} />
              <span className="truncate">{state}</span>
            </div>
            {conn && (
              <div className="mt-0.5 text-caption text-faint">
                {lastSynced !== null ? syncedText(lastSynced) : t("mobile.sync.never")}
              </div>
            )}
          </div>
        </div>

        {conn ? (
          <>
            <dl className="mt-5 text-caption">
              <div className="flex justify-between gap-4">
                <dt className="text-muted">{t("mobile.sync.waiting")}</dt>
                <dd className="text-right text-ink">{pending ?? t("mobile.sync.nothingWaiting")}</dd>
              </div>
            </dl>
            {conn.state === "down" && <p className="mt-3 text-caption leading-relaxed text-muted">{t("mobile.link.offlineHint")}</p>}
            <button
              className="btn-accent mt-5 min-h-12 w-full gap-2 text-body"
              disabled={syncing}
              onClick={() => void onSyncNow()}
            >
              <Icon name="sync" size={20} className={syncing ? "spin" : ""} />
              {syncing ? t("mobile.sync.syncing") : t("mobile.sync.now")}
            </button>
          </>
        ) : (
          <>
            <p className="mt-4 text-caption leading-relaxed text-muted">{t("mobile.library.unpairedHint")}</p>
            <button className="btn-accent mt-5 min-h-12 w-full text-body" onClick={onPair}>
              {t("mobile.menu.pair")}
            </button>
          </>
        )}
      </div>

      {conn && (
        <div className="mt-3 border-t border-line py-1">
          <MenuItem icon="close" danger label={t("mobile.link.forget")} onClick={() => setConfirmForget(true)} />
        </div>
      )}
      <MenuItem icon="info" label={t("mobile.menu.licences")} onClick={() => onTest("licences")} />
      {tests && (
        <>
          <MenuHeading>{t("mobile.menu.tests")}</MenuHeading>
          <MenuItem icon="activity" label={t("mobile.gpuTest.openButton")} onClick={() => onTest("gpuTest")} />
          {!conn && <MenuItem icon="grid" label={t("mobile.scrollTest.title")} onClick={() => onTest("scrollTest")} />}
        </>
      )}
      {version && (
        <p
          className="px-5 pt-3 pb-4 font-mono text-label text-faint select-none"
          onPointerDown={() => (hold.current = window.setTimeout(() => setTests(true), HOLD_MS))}
          onPointerUp={() => window.clearTimeout(hold.current)}
          onPointerLeave={() => window.clearTimeout(hold.current)}
          onContextMenu={(e) => e.preventDefault()}
        >
          {t("mobile.menu.version", { version })}
        </p>
      )}
    </Sheet>
      {confirmForget && desktop && (
        <ConfirmSheet
          title={t("mobile.sync.forgetTitle", { name: desktop })}
          message={t("mobile.sync.forgetMessage")}
          confirm={t("mobile.link.forget")}
          cancel={t("common.cancel")}
          onConfirm={() => void onForget()}
          onCancel={() => setConfirmForget(false)}
        />
      )}
    </>
  );
}
