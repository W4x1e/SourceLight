import { useEffect, useState, type CSSProperties } from "react";
import { t } from "../../src/i18n";
import { Icon, type IconName } from "../../src/lib/icons";
import { api, type LinkStatus } from "./api";
import DeviceLibrary from "./DeviceLibrary";
import GpuTest from "./GpuTest";
import Licences from "./Licences";
import Library from "./Library";
import { LinkProvider, type Test } from "./link";
import Tracks from "./TrackPanel";

type Tab = "library" | "device" | "tracks";

const TABS: { id: Tab; icon: IconName; label: () => string }[] = [
  { id: "library", icon: "grid", label: () => t("mobile.tabs.library") },
  { id: "device", icon: "phone", label: () => t("mobile.tabs.device") },
  { id: "tracks", icon: "trips", label: () => t("mobile.tabs.tracks") },
];

/**
 * The app's three places behind a tab bar (docs/MOBILE.md section 4): the
 * library synced with the computer, the phone's own photos, and GPS tracks.
 * Each stays mounted once opened, so switching keeps its place and the
 * library keeps syncing. Used without a computer, it opens on the phone's
 * own photos. Tapping the tab already showing scrolls it back to the top.
 *
 * The link to the computer (`LinkProvider`) is shared: each tab has the
 * computer button at its top right, which opens the sync sheet with
 * pairing, Forget computer and, held on its version line, the tests.
 */
export default function Home({
  desktop,
  platform,
  onPair,
}: {
  desktop: string | null;
  platform: LinkStatus["platform"];
  onPair: () => void;
}) {
  const paired = desktop !== null;
  const [tab, setTab] = useState<Tab>(paired ? "library" : "device");
  // A tab's screen is only made once someone opens it: the phone's photos
  // are only asked for then.
  const [opened, setOpened] = useState<Set<Tab>>(() => new Set([paired ? "library" : "device"]));
  // Bumped to send a tab's screen back to its top.
  const [toTop, setToTop] = useState<Record<Tab, number>>({ library: 0, device: 0, tracks: 0 });
  const [panel, setPanel] = useState<Test | null>(null);
  // The GPU test's photo: the newest in the library.
  const [gpuPhoto, setGpuPhoto] = useState<number | null>(null);

  const choose = (next: Tab) => {
    if (next === tab) return setToTop((all) => ({ ...all, [next]: all[next] + 1 }));
    setOpened((all) => new Set(all).add(next));
    setTab(next);
  };

  // A test opened from the sync sheet takes the sheet's Back step.
  const openTest = async (test: Test) => {
    if (test === "gpuTest") {
      const tiles = paired ? await api.photos().catch(() => []) : [];
      setGpuPhoto(tiles[0]?.[0] ?? null);
    } else if (test === "scrollTest") {
      // The scroll test is the library's grid.
      setOpened((all) => new Set(all).add("library"));
      setTab("library");
    }
    setPanel(test);
  };
  useEffect(() => {
    const onPop = () => {
      const at = history.state as Partial<Record<Test, boolean>> | null;
      setPanel(at?.gpuTest ? "gpuTest" : at?.scrollTest ? "scrollTest" : at?.licences ? "licences" : null);
    };
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  const shows = (id: Tab) => (tab === id ? "h-full" : "hidden");

  return (
    <LinkProvider desktop={desktop} onPair={onPair} onForgot={onPair} onTest={(test) => void openTest(test)}>
      <div className="flex h-full flex-col bg-base">
        {/* The tab bar takes the bottom safe area, so the grids need none. */}
        <div className="min-h-0 flex-1" style={{ "--grid-bottom": "0px" } as CSSProperties}>
          {opened.has("library") && (
            <div className={shows("library")}>
              <Library desktop={desktop} onPair={onPair} scrollTest={panel === "scrollTest"} toTop={toTop.library} />
            </div>
          )}
          {opened.has("device") && (
            <div className={shows("device")}>
              <DeviceLibrary active={tab === "device"} paired={paired} platform={platform} toTop={toTop.device} />
            </div>
          )}
          {opened.has("tracks") && (
            <div className={shows("tracks")}>
              <Tracks active={tab === "tracks"} toTop={toTop.tracks} />
            </div>
          )}
        </div>
        <nav className="flex shrink-0 border-t border-line bg-base pb-[var(--safe-bottom)]">
          {TABS.map(({ id, icon, label }) => (
            <button
              key={id}
              // On a phone on its side, the icon beside the label keeps the bar low.
              className={`relative flex min-h-14 flex-1 flex-col items-center justify-center gap-0.5 text-label short:min-h-11 short:flex-row short:gap-2 ${tab === id ? "font-medium text-ink" : "text-muted"}`}
              aria-current={tab === id ? "page" : undefined}
              onClick={() => choose(id)}
            >
              {/* The active tab as the desktop marks its modes: a white bar. */}
              {tab === id && <span aria-hidden className="absolute top-0 left-1/2 h-0.5 w-14 -translate-x-1/2 bg-accent" />}
              <Icon name={icon} size={22} />
              {label()}
            </button>
          ))}
        </nav>

        {panel === "licences" && (
          <div className="fixed inset-0 z-50">
            <Licences onClose={() => history.back()} />
          </div>
        )}
        {panel === "gpuTest" && (
          <div className="fixed inset-0 z-50">
            <GpuTest photoId={gpuPhoto} onClose={() => history.back()} />
          </div>
        )}
      </div>
    </LinkProvider>
  );
}
