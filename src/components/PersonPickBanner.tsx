import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import { toast } from "../lib/dialogs";
import { Icon } from "../lib/icons";
import { usePersonPick } from "../lib/personPick";
import { useStore } from "../store";
import { t, tx } from "../i18n";

/**
 * Browse, picking photos for someone: what is going on, and the button that
 * does it.
 *
 * The add lives here rather than in the selection panel, where it sat below
 * the capture data and the rating controls — out of sight at the very moment
 * it was the only thing to do.
 */
export default function PersonPickBanner() {
  const pick = usePersonPick((s) => s.pick);
  const stop = usePersonPick((s) => s.stop);
  const edited = usePersonPick((s) => s.edited);
  const setMode = useStore((s) => s.setMode);
  const selected = useStore((s) => s.selected);
  const clearSelection = useStore((s) => s.clearSelection);
  const filter = useStore((s) => s.filter);
  const reload = useStore((s) => s.reload);
  const [working, setWorking] = useState(false);

  // A regroup keeps people's ids, but the person being picked for
  // may have been folded into someone else. Stop rather than add photos to
  // nobody.
  useEffect(() => {
    if (!pick) return;
    const un = listen<{ phase: string }>("people-progress", (e) => {
      if (e.payload.phase !== "done") return;
      void api
        .listPeople()
        .then((people) => {
          if (people.some((p) => p.id === pick.id)) return;
          stop();
          toast(t("people.banner.regrouped"));
        })
        .catch(() => undefined);
    });
    return () => void un.then((f) => f());
  }, [pick, stop]);

  // Leaving Browse ends the pick: it went on silently, and came back
  // the next time Browse was opened.
  useEffect(() => {
    if (!pick) return;
    return useStore.subscribe((s, prev) => {
      if (prev.mode === "browse" && s.mode !== "browse") stop();
    });
  }, [pick, stop]);

  if (!pick) return null;
  const count = selected.size;

  const add = async () => {
    setWorking(true);
    try {
      const n = await api.addPhotosToPerson([...selected], pick.id);
      toast(
        n === 0
          ? t(count === 1 ? "people.banner.alreadyInThat" : "people.banner.alreadyInThose", { name: pick.name })
          : t("people.toast.added", { name: pick.name, count: n }),
        n === 0 ? "info" : "success",
      );
      clearSelection();
      edited();
      if (filter.personId != null || (filter.personIds?.length ?? 0) > 0) await reload();
    } catch (e) {
      toast(t("people.toast.couldNotAdd", { error: String(e) }), "error");
    } finally {
      setWorking(false);
    }
  };

  return (
    // Wraps rather than cutting the sentence: at the minimum window it
    // was "Addi…", and the name of whose photos these are was lost.
    <div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-line bg-panel px-4 py-2 text-caption">
      <Icon name="user" size={15} className="shrink-0 text-accent" />
      <span className="min-w-[10rem] flex-1 text-muted">
        {tx("people.banner.adding", { name: <span className="font-medium text-ink">{pick.name}</span> })}
      </span>
      <button onClick={() => void add()} disabled={working || count === 0} className="btn-accent shrink-0">
        <Icon name="plus" size={14} />
        {count === 0 ? t("people.banner.add") : t("people.banner.addCount", { count })}
      </button>
      <button
        onClick={() => {
          stop();
          setMode("people");
        }}
        className="btn-ghost shrink-0"
      >
        {t("common.done")}
      </button>
      <button
        onClick={stop}
        className="icon-chip shrink-0"
        title={t("people.banner.cancel")}
        aria-label={t("people.banner.cancel")}
      >
        <Icon name="close" size={14} />
      </button>
    </div>
  );
}
