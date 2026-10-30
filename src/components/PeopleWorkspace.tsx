import { Fragment, useCallback, useEffect, useRef, useState } from "react";
import { IS_MAC } from "../lib/platform";
import { ensureAi } from "../lib/aiReady";
import { listen } from "@tauri-apps/api/event";
import { api, fileUrl } from "../lib/api";
import { describePeopleRun, findPeople, peopleBacklog, type PeopleStage, arrangePeople, type PeopleOrder } from "../lib/people";
import { scrimOpen, useEscape } from "../lib/useEscape";
import { usePresence } from "../lib/usePresence";
import { Icon } from "../lib/icons";
import { confirm, toast } from "../lib/dialogs";
import { comboToLabel, resolveAction } from "../lib/keymap";
import { isCancelled } from "../lib/jobs";
import { usePersonPick } from "../lib/personPick";
import { useStore } from "../store";
import { formatNumber, t, tx } from "../i18n";
import { EMPTY_FILTER, type PhotoFilter } from "../types";
import FaceCrop from "./FaceCrop";
import PhotoSet from "./PhotoSet";
import UnplacedFaces from "./UnplacedFaces";
import type { FacesStatus, PeopleSuggestion, Person, PersonFace } from "../bindings";

/**
 * People: the identities the face pass found, grouped by how they look.
 *
 * Three things this workspace has to do that it previously didn't.
 *
 * It has to *get* the faces. Analysing them was a button in Cull — a different
 * workspace, for a different job — so arriving here on a fresh library showed an
 * empty page and instructions to go somewhere else. It now runs the pass itself
 * on arrival (the face and People models are a one-off download of about
 * 262 MB; culling alone needs only the 1 MB face models).
 *
 * It has to be correctable. Grouping deliberately leans toward splitting one
 * person into two rather than merging two strangers, on the grounds that the
 * user can fix a split — which was only true in the sense that nothing stopped
 * them wishing for it. Merge and "not this person" are that repair, and both
 * are recorded so a later regroup does not undo them. The near misses the
 * grouping would not merge on its own are offered as "Same person?", so the
 * user does not have to hunt for them.
 *
 * And the name box has to fit in the card, which it did not: `.input` is
 * `w-full`, so a `w-full` input with `m-1.5` inside a 132px card overflowed it
 * by exactly the 12px of its own margins.
 */

/** The labels the busy status can show. */
type BusyKey =
  | "people.busy.grouping"
  | "people.busy.clearing"
  | "people.busy.merging"
  | "people.busy.hiding"
  | "people.busy.updating"
  | "people.busy.erasing";

/** The More menu's entries: keys, translated where the menu is drawn. */
type MenuKey =
  | "people.menu.regroup"
  | "people.menu.regroupHint"
  | "people.menu.hideSeenOnce"
  | "people.menu.hideSeenOnceHint"
  | "people.menu.unplaced"
  | "people.menu.unplacedHint"
  | "people.menu.redetect"
  | "people.menu.redetectHint"
  | "people.menu.erase"
  | "people.menu.eraseHint";

/** The status line for a run in progress. */
function stageLabel(stage: PeopleStage, progress: { current: number; total: number } | null): string {
  if (stage === "setup") return t("people.stage.setup");
  if (stage === "grouping") return t("people.busy.grouping");
  return progress
    ? t("people.stage.facesProgress", { current: progress.current, total: progress.total })
    : t("people.stage.faces");
}

/** A run that found faces and could not recognise them: said on the page, with a way to try again. */
function FailureCard({
  message,
  onRetry,
  disabled,
}: {
  message: string;
  onRetry: () => void;
  disabled: boolean;
}) {
  return (
    <div
      role="alert"
      className="mx-auto mb-4 flex max-w-xl items-start gap-3 rounded-lg border border-line bg-panel p-3"
    >
      <Icon name="warn" size={16} className="mt-0.5 shrink-0 text-danger" />
      <div className="min-w-0 flex-1">
        <div className="text-body font-medium text-ink">{t("people.failure.title")}</div>
        <div className="mt-0.5 break-words text-caption leading-relaxed text-faint">{message}</div>
      </div>
      <button onClick={onRetry} disabled={disabled} className="btn-solid shrink-0">
        {t("common.retry")}
      </button>
    </div>
  );
}

/**
 * The page's maintenance actions. Regroup and Re-detect sat next to the main
 * button as equals, which made finding people look like a choice between three
 * steps rather than one action with two repairs.
 */
function MoreMenu({
  items,
  disabled,
}: {
  items: { label: MenuKey; hint: MenuKey; onClick: () => void; danger?: boolean }[];
  disabled: boolean;
}) {
  const [open, setOpen] = useState(false);
  const presence = usePresence(open);
  const ref = useRef<HTMLDivElement>(null);
  const close = useCallback(() => setOpen(false), []);
  useEscape(open, close);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", onDown);
    return () => window.removeEventListener("pointerdown", onDown);
  }, [open]);
  // A menu, as the photo menu is: focus to its first item on open, and the
  // arrows walk the items.
  useEffect(() => {
    if (open) ref.current?.querySelector<HTMLElement>("[role=menuitem]")?.focus();
  }, [open]);
  const walk = (e: React.KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const list = [...(ref.current?.querySelectorAll<HTMLElement>("[role=menuitem]") ?? [])];
    const i = list.indexOf(document.activeElement as HTMLElement);
    const next = e.key === "ArrowDown" ? (i + 1) % list.length : (i - 1 + list.length) % list.length;
    list[next]?.focus();
  };
  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        disabled={disabled}
        className="btn-ghost"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={t("people.menu.aria")}
        title={t("common.more")}
      >
        <Icon name="more" size={16} />
      </button>
      {presence.mounted && (
        <div
          role="menu"
          onKeyDown={walk}
          className="anim-pop absolute right-0 top-full z-30 mt-1 min-w-56 rounded-md bg-raised py-1 shadow-2xl"
          data-tauri-drag-region="false"
          // Opens from the button at its right edge, not the default top left.
          style={{ transformOrigin: "top right" }}
          data-closing={presence.closing || undefined}
        >
          {items.map((it, i) => (
            <Fragment key={it.label}>
              {/* The ones that throw work away sit apart from the rest, as
                  the photo menu's Move to Recycle Bin does. */}
              {it.danger && i > 0 && !items[i - 1]?.danger && (
                <div className="my-1 h-px bg-line" role="separator" />
              )}
              <button
                role="menuitem"
                title={t(it.hint)}
                onClick={() => {
                  setOpen(false);
                  it.onClick();
                }}
                className={`flex w-full px-3 py-1.5 text-left text-caption hover:bg-hover focus:bg-hover focus:outline-none ${
                  it.danger ? "text-danger-ink" : "text-ink"
                }`}
              >
                {t(it.label)}
              </button>
            </Fragment>
          ))}
        </div>
      )}
    </div>
  );
}

/** How many of each person's other faces the match review offers to flip through. */
const REVIEW_STRIP = 7;
/** Faces the review sheet shows at a time. */
const FACE_PAGE = 200;

const personLabel = (p: Person) => p.name ?? t("people.unnamed");

/**
 * "Same person?", asked at a size where it can be answered.
 *
 * The question used to be a 64px crop of each person's cover face in a banner
 * over the grid: too small to tell two people in sunglasses apart, and only one
 * face each, when the reason the grouping split them is usually that one person
 * looks different from photo to photo. Here each side is a full portrait with a
 * strip of that person's other faces to flip through, and the answer is a key,
 * so a run of these goes quickly.
 */
function MatchReview({
  a,
  b,
  closing,
  working,
  onAnswer,
  onSkip,
  onClose,
}: {
  a: Person;
  b: Person;
  closing: boolean;
  working: boolean;
  onAnswer: (same: boolean) => void;
  onSkip: () => void;
  onClose: () => void;
}) {
  const [faces, setFaces] = useState<Record<number, PersonFace[]>>({});
  const keymap = useStore((s) => s.keymap);
  const key = (id: "people-same" | "people-different" | "people-skip") => comboToLabel(keymap[id]);
  // The face each side is showing large; absent means the person's cover.
  const [shown, setShown] = useState<Record<number, PersonFace>>({});

  useEffect(() => {
    let live = true;
    setShown({});
    for (const id of [a.id, b.id]) {
      void api
        .personFaces(id)
        .catch(() => [])
        .then((fs) => {
          if (!live) return;
          // Faces too weak to vote on who anyone is are no help deciding it.
          const grouped = fs.filter((f) => !f.ungrouped);
          setFaces((prev) => ({ ...prev, [id]: (grouped.length ? grouped : fs).slice(0, REVIEW_STRIP) }));
        });
    }
    return () => {
      live = false;
    };
  }, [a.id, b.id]);

  useEscape(!closing, onClose);
  useEffect(() => {
    if (closing) return;
    const onKey = (e: KeyboardEvent) => {
      if (working) return;
      if (scrimOpen()) return;
      const action = resolveAction(useStore.getState().keymap, "people", e);
      if (action === "people-same") onAnswer(true);
      else if (action === "people-different") onAnswer(false);
      // → skips too, whatever skip is bound to: it reads as "next".
      else if (action === "people-skip" || (e.key === "ArrowRight" && !e.ctrlKey && !e.metaKey)) onSkip();
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [closing, working, onAnswer, onSkip]);

  return (
    <div
      className="anim-scrim fixed inset-0 z-50 grid place-items-center bg-scrim p-8"
      data-closing={closing || undefined}
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-labelledby="match-review-title"
        className="anim-card flex max-h-[calc(100vh-4rem)] w-full max-w-[640px] flex-col overflow-hidden rounded-lg border border-line bg-panel shadow-2xl"
      >
        <div className="flex items-center gap-3 border-b border-line px-4 py-3">
          <div className="min-w-0">
            <h2 id="match-review-title" className="font-display text-display text-ink">
              {t("people.match.title")}
            </h2>
            <p className="text-caption text-faint">{t("people.match.subtitle")}</p>
          </div>
          <button onClick={onClose} className="icon-chip ml-auto shrink-0" aria-label={t("common.close")}>
            <Icon name="close" size={15} />
          </button>
        </div>

        <div className="grid min-h-0 grid-cols-2 gap-4 overflow-y-auto p-4">
          {[a, b].map((p) => {
            const big = shown[p.id];
            const strip = faces[p.id];
            return (
              <div key={p.id} className="flex min-w-0 flex-col items-center">
                {/* Capped by height too, so both portraits and the buttons fit
                    at the smallest window. */}
                <div
                  className="w-full overflow-hidden rounded-md"
                  style={{ maxWidth: "min(100%, calc(100vh - 380px))" }}
                >
                  {big ? (
                    <FaceCrop
                      src={big.thumbPath ? fileUrl(big.thumbPath) : null}
                      box={big.boxFrac}
                      portrait={big.portraitPath}
                    />
                  ) : (
                    <FaceCrop
                      src={p.coverPath ? fileUrl(p.coverPath) : null}
                      box={p.coverBox}
                      portrait={p.portraitPath}
                    />
                  )}
                </div>
                <div className="mt-2 w-full truncate text-center text-body font-medium text-ink">
                  {personLabel(p)}
                </div>
                <div className="text-caption text-faint">
                  {tx("common.photos", { count: <span className="tabular-nums">{formatNumber(p.count)}</span> }, p.count)}
                </div>
                {/* Hover or focus to see a face large: scrubbing the strip is
                    how you check the same person holds up across photos. */}
                <div className="mt-2 flex h-10 flex-wrap justify-center gap-1">
                  {strip?.map((f) => (
                    <button
                      key={f.faceId}
                      onPointerEnter={() => setShown((s) => ({ ...s, [p.id]: f }))}
                      onFocus={() => setShown((s) => ({ ...s, [p.id]: f }))}
                      onClick={() => setShown((s) => ({ ...s, [p.id]: f }))}
                      className={`overflow-hidden rounded-sm ring-offset-1 ring-offset-panel ${
                        big?.faceId === f.faceId ? "ring-2 ring-accent" : "opacity-80 hover:opacity-100"
                      }`}
                      aria-label={t("people.match.showAnother", { name: personLabel(p) })}
                    >
                      <FaceCrop
                        src={f.thumbPath ? fileUrl(f.thumbPath) : null}
                        box={f.boxFrac}
                        portrait={f.portraitPath}
                        size={40}
                      />
                    </button>
                  ))}
                </div>
              </div>
            );
          })}
        </div>

        <div className="flex items-center gap-2 border-t border-line px-4 py-3">
          <button onClick={onSkip} disabled={working} className="btn-ghost">
            {t("people.match.skip")} <span className="font-mono text-label opacity-60" aria-hidden>{key("people-skip")}</span>
          </button>
          <div className="ml-auto flex items-center gap-2">
            <button onClick={() => onAnswer(false)} disabled={working} className="btn-solid">
              {t("people.match.notSame")} <span className="font-mono text-label opacity-60" aria-hidden>{key("people-different")}</span>
            </button>
            <button onClick={() => onAnswer(true)} disabled={working} className="btn-accent">
              {t("people.match.same")} <span className="font-mono text-label opacity-60" aria-hidden>{key("people-same")}</span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

/** Names compared the way a person would: case and stray spaces don't count. */
const sameName = (a: string | null, b: string | null) =>
  !!a && !!b && a.trim().toLocaleLowerCase() === b.trim().toLocaleLowerCase();

/**
 * Every face grouped as one person, and where to send them if they're
 * someone else.
 *
 * It only offered "Not them", which is no use on the commonest repair: a
 * person split off into a card of their own, often of one face, and
 * recognised at a glance. The way to fix that was to ctrl-click two cards and
 * find a Merge button in the header, which nothing on this dialog said. Now
 * the dialog asks who they are, with anyone of the same name first.
 */
function FaceReview({
  person,
  faces,
  others,
  working,
  onDetach,
  onMerge,
  onCover,
  onClose,
}: {
  person: Person;
  faces: PersonFace[];
  /** Everyone they could be merged into. */
  others: Person[];
  working: boolean;
  onDetach: (faceId: number) => void;
  onMerge: (into: Person) => void;
  /** Makes a face the person's cover. */
  onCover: (faceId: number) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  // A page at a time: thousands of faces mounted at once froze the
  // sheet for someone in most of the library.
  const [shownCount, setShownCount] = useState(FACE_PAGE);
  useEscape(true, onClose);
  const single = faces.length <= 1;
  const q = query.trim().toLocaleLowerCase();
  // Same name first — the likeliest answer — then named people, then by size.
  const rank = (p: Person) => (sameName(p.name, person.name) ? 0 : p.name ? 1 : 2);
  const candidates = others
    .filter((p) => !q || (p.name ?? "").toLocaleLowerCase().includes(q))
    .sort((a, b) => rank(a) - rank(b) || b.count - a.count);

  return (
    <div
      className="anim-scrim fixed inset-0 z-50 grid place-items-center bg-scrim p-8"
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-labelledby="face-review-title"
        className="anim-card flex max-h-[min(720px,calc(100vh-4rem))] w-full max-w-xl flex-col overflow-hidden rounded-lg border border-line bg-panel shadow-2xl"
      >
        <div className="flex items-center gap-3 border-b border-line px-4 py-3">
          <div className="min-w-0">
            <h2 id="face-review-title" className="font-display text-display text-ink">
              {person.name ?? t("people.unnamedPerson")}
            </h2>
            <p className="text-caption text-faint">
              {single ? t("people.faces.single") : t("people.faces.many", { count: faces.length })}
            </p>
          </div>
          <button onClick={onClose} className="icon-chip ml-auto shrink-0" aria-label={t("common.close")}>
            <Icon name="close" size={15} />
          </button>
        </div>

        <div className="flex min-h-0 shrink flex-wrap gap-2 overflow-y-auto p-4">
          {faces.slice(0, shownCount).map((f) => (
            <div key={f.faceId} className="group relative overflow-hidden rounded-md border border-line">
              <FaceCrop
                src={f.thumbPath ? fileUrl(f.thumbPath) : null}
                box={f.boxFrac}
                portrait={f.portraitPath}
                size={single ? 160 : 92}
              />
              {f.manual && (
                <span
                  className="absolute left-1 top-1 rounded-sm bg-chip px-1 text-label text-muted backdrop-blur"
                  title={t("people.faces.addedByYouHint")}
                >
                  {t("people.faces.addedByYou")}
                </span>
              )}
              {f.ungrouped && (
                <span
                  className="absolute left-1 top-1 rounded-sm bg-chip px-1 text-label text-muted backdrop-blur"
                  title={t("people.faces.notGroupedHint")}
                >
                  {t("people.faces.notGrouped")}
                </span>
              )}
              {/* The cover: marked, or offered on hover. */}
              {person.coverFaceId === f.faceId ? (
                <span
                  className="absolute right-1 top-1 rounded-sm bg-accent px-1 text-label text-on-accent"
                  title={t("people.faces.coverHint")}
                >
                  {t("people.faces.cover")}
                </span>
              ) : (
                !single &&
                !f.manual && (
                  <button
                    onClick={() => onCover(f.faceId)}
                    disabled={working}
                    className="absolute right-1 top-1 rounded-sm bg-chip px-1 text-label text-ink opacity-0 backdrop-blur transition-opacity focus-visible:opacity-100 group-hover:opacity-100"
                    title={t("people.faces.useAsCoverHint")}
                  >
                    {t("people.faces.useAsCover")}
                  </button>
                )
              )}
              {/* Taking the only face out of a person just makes the same
                  person again; a hand-added one is still worth removing. */}
              {(!single || f.manual) && (
                <button
                  onClick={() => onDetach(f.faceId)}
                  disabled={working}
                  className="absolute inset-x-0 bottom-0 bg-chip py-1 text-label text-ink opacity-0 backdrop-blur transition-opacity focus-visible:opacity-100 group-hover:opacity-100"
                >
                  {t("people.faces.notThem")}
                </button>
              )}
            </div>
          ))}
          {faces.length > shownCount && (
            <button onClick={() => setShownCount((n) => n + FACE_PAGE)} className="btn-ghost w-full">
              {t("people.faces.showMore", { count: Math.min(FACE_PAGE, faces.length - shownCount) })}
            </button>
          )}
        </div>

        {others.length > 0 && (
          <div className="flex min-h-0 flex-col border-t border-line">
            <div className="flex items-center gap-3 px-4 pb-2 pt-3">
              <div className="text-body font-medium text-ink">{t("people.faces.samePersonAs")}</div>
              <div className="relative ml-auto w-48">
                <Icon
                  name="search"
                  size={13}
                  className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-faint"
                />
                <input
                  className="input h-7 pl-7 text-caption"
                  placeholder={t("people.faces.findPlaceholder")}
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  aria-label={t("people.faces.findAria")}
                />
              </div>
            </div>
            <div className="flex max-h-[min(232px,32vh)] flex-wrap content-start gap-2 overflow-y-auto px-4 pb-4">
              {candidates.length === 0 && (
                <div className="py-3 text-caption text-faint">{t("people.faces.nobody")}</div>
              )}
              {candidates.map((p) => {
                const match = sameName(p.name, person.name);
                return (
                  <button
                    key={p.id}
                    onClick={() => onMerge(p)}
                    disabled={working}
                    className={`w-[76px] overflow-hidden rounded-md border bg-panel text-left transition-colors hover:border-accent ${
                      match ? "border-accent/60" : "border-line"
                    }`}
                    title={t("people.faces.mergeInto", { name: personLabel(p), count: p.count })}
                  >
                    <FaceCrop
                      src={p.coverPath ? fileUrl(p.coverPath) : null}
                      box={p.coverBox}
                      portrait={p.portraitPath}
                      size={74}
                    />
                    <div className="px-1 py-0.5">
                      <div className={`truncate text-label ${p.name ? "text-ink" : "text-faint"}`}>
                        {personLabel(p)}
                      </div>
                      <div className="text-label text-faint">
                        {tx("common.photos", { count: <span className="tabular-nums">{formatNumber(p.count)}</span> }, p.count)}
                      </div>
                    </div>
                  </button>
                );
              })}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

/** The key held to choose people for a merge: ⌘ on a Mac. */
const mergeKey = () => (IS_MAC ? "⌘" : t("shell.keymap.key.ctrl"));

export default function PeopleWorkspace() {
  // Whether recognising faces is on: a yes asked for here, once.
  const [peopleOn, setPeopleOn] = useState<boolean | null>(null);
  useEffect(() => {
    void api
      .peopleEnabled()
      .then(setPeopleOn)
      .catch(() => setPeopleOn(true));
  }, []);
  const [people, setPeople] = useState<Person[] | null>(null);
  const [busy, setBusy] = useState<BusyKey | null>(null);
  const [names, setNames] = useState<Record<number, string>>({});
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [open, setOpen] = useState<{ person: Person; faces: PersonFace[] } | null>(null);
  const openRef = useRef(open);
  openRef.current = open;
  const [status, setStatus] = useState<FacesStatus | null>(null);
  // The run in progress, by stage; the face pass also carries its count.
  const [stage, setStage] = useState<PeopleStage | null>(null);
  const [progress, setProgress] = useState<{ current: number; total: number } | null>(null);
  // The last run's outcome when it needs saying on the page, not only in a
  // toast: faces found and none recognised left an empty page with no reason.
  const [failure, setFailure] = useState<string | null>(null);
  const [suggestions, setSuggestions] = useState<PeopleSuggestion[]>([]);
  const [reviewing, setReviewing] = useState(false);
  const [skipped, setSkipped] = useState<Set<string>>(new Set());
  const [showHidden, setShowHidden] = useState(false);
  // Finding someone among many, and the order they're shown in.
  const [findPerson, setFindPerson] = useState("");
  const [peopleOrder, setPeopleOrder] = useState<PeopleOrder>("photos");
  const startPick = usePersonPick((s) => s.start);
  const setMode = useStore((s) => s.setMode);
  const setFilter = useStore((s) => s.setFilter);
  const browseFilter = useStore((s) => s.filter);
  const clearBrowseSelection = useStore((s) => s.clearSelection);
  // Back on this page, a pick started from it is over.
  useEffect(() => usePersonPick.getState().stop(), []);
  // The person whose photos are showing, in place of the page of people.
  const [viewing, setViewing] = useState<{ title: string; filter: PhotoFilter } | null>(null);

  // Suggestions are worked out off the main thread, so after two quick
  // changes the older answer can arrive last; only the latest is kept.
  const loads = useRef(0);
  const load = useCallback(async () => {
    const mine = ++loads.current;
    setPeople(await api.listPeople().catch(() => []));
    setStatus(await api.facesStatus().catch(() => null));
    const suggestions = await api.peopleSuggestions().catch(() => []);
    if (mine === loads.current) setSuggestions(suggestions);
  }, []);
  useEffect(() => void load(), [load]);
  const backlog = peopleBacklog(status);

  const group = useCallback(
    async (announce: boolean) => {
      setBusy("people.busy.grouping");
      try {
        // Asked for: every face again, not only the new ones.
        const n = await api.groupPeople(true);
        await load();
        if (announce) toast(t("people.toast.grouped", { count: n }), "success");
      } catch (e) {
        // Stopped from the Activity panel: nothing was changed, and the
        // user chose it, so it is not an error.
        if (isCancelled(e)) toast(t("people.toast.groupingStopped"));
        // Otherwise the models are present — `ensureAi` ran above — so this
        // is a real failure, not a setup gap.
        else toast(t("people.toast.couldNotGroup", { error: String(e) }), "error");
      } finally {
        setBusy(null);
      }
    },
    [load],
  );

  /** The one action: set up, find and recognise faces, group — see `findPeople`. */
  const run = useCallback(async () => {
    setFailure(null);
    setProgress(null);
    try {
      const result = await findPeople(setStage);
      if (!result) return;
      const { text, tone } = describePeopleRun(result);
      if (tone === "error") setFailure(text);
      else toast(text, tone);
    } catch (e) {
      if (isCancelled(e)) toast(t("people.run.stopped"));
      else setFailure(t("people.failure.message", { error: String(e) }));
    } finally {
      setStage(null);
      setProgress(null);
      await load();
    }
  }, [load]);

  // Progress for the pass, whoever started it; and a reload when grouping
  // finishes, which is how automatic analysis's results arrive here.
  useEffect(() => {
    const unFaces = listen<{ current: number; total: number; phase: string }>("faces-progress", (e) => {
      const p = e.payload;
      if (p.phase !== "done") setProgress({ current: p.current, total: p.total });
    });
    const unPeople = listen<{ phase: string }>("people-progress", (e) => {
      if (e.payload.phase === "done") void load();
    });
    // Portraits are cut in the background: the tiles crop thumbnails
    // until they are ready, then show them.
    const unPortraits = listen("people-portraits", () => {
      void api
        .listPeople()
        .then(setPeople)
        .catch(() => undefined);
      const shown = openRef.current?.person.id;
      if (shown == null) return;
      void api
        .personFaces(shown)
        .then((faces) => setOpen((cur) => (cur && cur.person.id === shown ? { ...cur, faces } : cur)))
        .catch(() => undefined);
    });
    return () => {
      void unFaces.then((f) => f());
      void unPeople.then((f) => f());
      void unPortraits.then((f) => f());
    };
  }, [load]);

  // Arriving with work to do starts the pass, once per visit; the ref keeps a
  // re-render from starting a second one on top of it. On a library that
  // already has people it waits for the button — a top-up is the user's call.
  const autoStarted = useRef(false);
  useEffect(() => {
    if (!peopleOn || autoStarted.current || status === null || backlog === 0) return;
    if (people !== null && people.length > 0) return;
    autoStarted.current = true;
    void run();
  }, [peopleOn, status, backlog, people, run]);

  /** The yes: recognising starts, fetching the recogniser first. */
  const turnOn = async () => {
    await api.setPeopleEnabled(true);
    setPeopleOn(true);
    autoStarted.current = true;
    await run();
  };

  /** Turns People off and erases what it built, here and in backups. */
  const erase = async () => {
    const yes = await confirm({
      title: t("people.erase.title"),
      message: t("people.erase.message"),
      confirmLabel: t("people.erase.confirm"),
      danger: true,
    });
    if (!yes) return;
    setBusy("people.busy.erasing");
    try {
      const done = await api.eraseFaceData();
      setPeopleOn(false);
      setOpen(null);
      setSelected(new Set());
      await load();
      toast(
        done.kept > 0
          ? `${t("people.erase.done", { count: done.scrubbed })} ${t("people.erase.kept", { count: done.kept })}`
          : t("people.erase.done", { count: done.scrubbed }),
        "success",
      );
    } catch (e) {
      toast(t("people.erase.failed", { error: String(e) }), "error");
    } finally {
      setBusy(null);
    }
  };

  /** Throws away every detected face and runs the pass again. */
  const reanalyse = async () => {
    const yes = await confirm({
      title: t("people.workspace.redetectTitle"),
      message: t("people.workspace.redetectMessage"),
      confirmLabel: t("people.workspace.redetectConfirm"),
      danger: true,
    });
    if (!yes) return;
    // AI first: clearing first left every photo queued for faces when
    // turning AI on was then declined.
    if (!(await ensureAi("faces"))) return;
    setBusy("people.busy.clearing");
    try {
      await api.resetFaces();
      await load();
    } finally {
      setBusy(null);
    }
    autoStarted.current = true;
    await run();
  };

  const rename = async (p: Person) => {
    const name = (names[p.id] ?? p.name ?? "").trim();
    if (name === (p.name ?? "")) return;
    await api.setPersonName(p.id, name);
    // A name someone else already has is usually the same person split in
    // two — but not always, so it is a question, never a merge by name.
    const namesake = (people ?? [])
      .filter((o) => o.id !== p.id && sameName(o.name, name))
      .sort((a, b) => b.count - a.count)[0];
    if (
      namesake &&
      (await confirm({
        title: t("people.workspace.namesakeTitle", { name: namesake.name ?? "" }),
        message: t("people.workspace.namesakeMessage", { name: namesake.name ?? "", count: namesake.count }),
        confirmLabel: t("people.workspace.namesakeMerge"),
        cancelLabel: t("people.workspace.namesakeApart"),
      }))
    ) {
      await mergeInto(p, namesake);
      return;
    }
    await load();
  };

  // Their photos, here — not a Browse filter and a jump to another workspace.
  const viewPerson = (p: Person) =>
    setViewing({
      title: p.name ?? t("people.unnamedPerson"),
      filter: { ...EMPTY_FILTER, personIds: [p.id] },
    });

  const toggleSelected = (id: number) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  /** Asks before merging people with different names: one key made
   *  two sisters one person, and one name was dropped. */
  const namesAgree = async (ids: number[]): Promise<boolean> => {
    const names = [
      ...new Set((people ?? []).filter((p) => ids.includes(p.id) && p.name).map((p) => p.name!.trim())),
    ];
    if (names.length < 2) return true;
    return confirm({
      title: t("people.merge.twoNamesTitle"),
      message: t("people.merge.twoNamesMessage", { names: names.join(", ") }),
      confirmLabel: t("people.merge.confirm"),
    });
  };
  /** Every merge's toast can take it back. */
  const undoMerge = {
    label: t("common.undo"),
    run: () =>
      void api
        .undoMerge()
        .then(() => load())
        .then(() => toast(t("people.merge.undone"), "success"))
        .catch((e) => toast(String(e), "error")),
  };

  const mergeSelected = async () => {
    if (selected.size < 2) return;
    if (!(await namesAgree([...selected]))) return;
    setBusy("people.busy.merging");
    try {
      await api.mergePeople([...selected]);
      setSelected(new Set());
      await load();
      toast(t("people.toast.merged"), "success", undoMerge);
    } catch (e) {
      toast(t("people.toast.couldNotMerge", { error: String(e) }), "error");
    } finally {
      setBusy(null);
    }
  };

  /** Puts people out of the way, or brings them back. */
  const hide = async (ids: number[], hidden: boolean) => {
    setBusy(hidden ? "people.busy.hiding" : "people.busy.updating");
    try {
      await api.setPeopleHidden(ids, hidden);
      setSelected(new Set());
      await load();
      if (hidden)
        toast(
          ids.length === 1
            ? t("people.toast.hidden")
            : t("people.toast.hidMany", { count: ids.length }),
        );
    } catch (e) {
      toast(t("people.toast.couldNotUpdate", { error: String(e) }), "error");
    } finally {
      setBusy(null);
    }
  };

  /** The usual bulk of a page of people: strangers in one photo's background. */
  const hideSeenOnce = async () => {
    const once = (people ?? []).filter((p) => !p.hidden && !p.name && p.count === 1);
    if (once.length === 0) {
      toast(t("people.workspace.nobodyOnce"));
      return;
    }
    const yes = await confirm({
      title: t("people.workspace.hideOnceTitle", { count: once.length }),
      message: t("people.workspace.hideOnceMessage"),
      confirmLabel: t("people.workspace.hideOnceConfirm"),
    });
    if (yes) await hide(once.map((p) => p.id), true);
  };

  /** Browse, picking the photos face detection missed them in. */
  const addPhotos = (p: Person) => {
    startPick(p.id, p.name ?? t("people.thisPerson"));
    clearBrowseSelection();
    setFilter({ ...browseFilter, personId: null, personIds: [] });
    setMode("browse");
  };

  /** One person into another: the "same person as…" of the face review. */
  const mergeInto = async (from: Person, into: Person) => {
    if (!(await namesAgree([into.id, from.id]))) return;
    setBusy("people.busy.merging");
    try {
      await api.mergePeople([into.id, from.id]);
      setOpen(null);
      setSelected(new Set());
      await load();
      toast(t("people.toast.mergedInto", { name: into.name ?? t("people.thatPerson") }), "success", undoMerge);
    } catch (e) {
      toast(t("people.toast.couldNotMerge", { error: String(e) }), "error");
    } finally {
      setBusy(null);
    }
  };

  const closeFaces = useCallback(() => setOpen(null), []);
  const [unplacedOpen, setUnplacedOpen] = useState(false);

  const chooseCover = async (faceId: number) => {
    if (!open) return;
    try {
      await api.setCoverFace(faceId);
      await load();
      setOpen({ ...open, person: { ...open.person, coverFaceId: faceId } });
    } catch (e) {
      toast(t("people.toast.couldNotCover", { error: String(e) }), "error");
    }
  };

  const openPerson = async (person: Person) => {
    setOpen({ person, faces: await api.personFaces(person.id).catch(() => []) });
  };

  const detach = async (faceId: number) => {
    setBusy("people.busy.updating");
    try {
      await api.detachFace(faceId);
      await load();
      const manual = open?.faces.find((f) => f.faceId === faceId)?.manual;
      // The face leaves the sheet; the rest are as they were.
      setOpen((cur) => cur && { ...cur, faces: cur.faces.filter((f) => f.faceId !== faceId) });
      toast(manual ? t("people.toast.takenOut") : t("people.toast.movedOut"), "success");
    } catch (e) {
      toast(t("people.toast.couldNotMove", { error: String(e) }), "error");
    } finally {
      setBusy(null);
    }
  };

  const working = busy !== null || stage !== null;

  // Two people who may be one, asked a pair at a time in `MatchReview`. A
  // skipped pair stays skipped for this visit; the backend only sends a few at
  // once, so answering one brings in the next.
  const byId = new Map((people ?? []).map((p) => [p.id, p]));
  const visible = (people ?? []).filter((p) => !p.hidden);
  const hiddenPeople = (people ?? []).filter((p) => p.hidden);
  const pairKey = (s: PeopleSuggestion) => `${s.a}:${s.b}`;
  const queue = suggestions.filter((s) => byId.has(s.a) && byId.has(s.b) && !skipped.has(pairKey(s)));
  const question = queue[0];
  const pair = question ? { a: byId.get(question.a)!, b: byId.get(question.b)! } : null;
  // What the dialog shows while it animates out, after the last pair is gone.
  const lastPair = useRef(pair);
  if (pair) lastPair.current = pair;
  const reviewOpen = reviewing && pair !== null;
  const review = usePresence(reviewOpen);
  const closeReview = useCallback(() => setReviewing(false), []);

  const answer = useCallback(
    async (same: boolean) => {
      if (!question) return;
      if (same && !(await namesAgree([question.a, question.b]))) return;
      setBusy(same ? "people.busy.merging" : "people.busy.updating");
      try {
        if (same) await api.mergePeople([question.a, question.b]);
        else await api.keepPeopleApart(question.a, question.b);
        await load();
        // A merge is said, and can be taken back: it was silent.
        if (same) toast(t("people.toast.merged"), "success", undoMerge);
      } catch (e) {
        toast(String(e), "error");
      } finally {
        setBusy(null);
      }
    },
    [question, load],
  );
  const skip = useCallback(() => {
    if (question) setSkipped((s) => new Set(s).add(pairKey(question)));
  }, [question]);

  // Out of pairs mid-review: say so rather than just vanishing.
  useEffect(() => {
    if (reviewing && people !== null && !busy && !question) {
      setReviewing(false);
      toast(t("people.toast.lookAlikesDone"), "success");
    }
  }, [reviewing, people, busy, question]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {viewing && (
        <PhotoSet
          title={viewing.title}
          backLabel={t("people.workspace.title")}
          filter={viewing.filter}
          onBack={() => setViewing(null)}
        />
      )}
      {/* The page is hidden rather than unmounted while a person is open, so
          Back returns to the same scroll position on a long page of people. */}
      <div
        className={`h-[52px] shrink-0 items-center gap-2.5 border-b border-line bg-panel px-4 ${
          viewing ? "hidden" : "flex"
        }`}
        data-tauri-drag-region="deep"
      >
        <Icon name="user" size={16} className="text-accent" />
        <div className="min-w-0">
          <div className="text-body font-semibold text-ink">{t("people.workspace.title")}</div>
          <div className="truncate text-caption text-faint">
            {selected.size > 0
              ? t("people.workspace.subtitleSelected", { count: selected.size })
              : t("people.workspace.subtitle")}
          </div>
        </div>

        <div className="ml-auto flex shrink-0 items-center gap-2">
          {selected.size >= 2 && (
            <button onClick={() => void mergeSelected()} disabled={working} className="btn-accent">
              {t("people.workspace.merge", { count: selected.size })}
            </button>
          )}
          {selected.size > 0 && (
            <button onClick={() => void hide([...selected], true)} disabled={working} className="btn-solid">
              <Icon name="eye-off" size={14} />
              {t("people.workspace.hideCount", { count: selected.size })}
            </button>
          )}
          {selected.size > 0 && (
            <button onClick={() => setSelected(new Set())} className="btn-ghost">
              {t("common.clear")}
            </button>
          )}
          {stage ? (
            <span className="text-caption tabular-nums text-muted" role="status">
              {stageLabel(stage, progress)}
            </span>
          ) : (
            peopleOn !== false &&
            (backlog > 0 || people?.length === 0) && (
              <button onClick={() => void run()} disabled={working} className="btn-solid">
                <Icon name="eye" size={14} />
                {backlog > 0 ? t("people.workspace.analyse", { count: backlog }) : t("people.workspace.findPeople")}
              </button>
            )
          )}
          {busy && <span className="text-caption text-muted">{t(busy)}</span>}
          <MoreMenu
            disabled={working}
            items={[
              {
                label: "people.menu.regroup",
                hint: "people.menu.regroupHint",
                onClick: () => void group(true),
              },
              {
                label: "people.menu.hideSeenOnce",
                hint: "people.menu.hideSeenOnceHint",
                onClick: () => void hideSeenOnce(),
              },
              {
                label: "people.menu.unplaced",
                hint: "people.menu.unplacedHint",
                onClick: () => setUnplacedOpen(true),
              },
              // Re-detect from scratch. Regrouping re-reads existing face rows,
              // so it cannot fix anything the detection pass itself failed to
              // record — which is why this is separate, and destructive.
              {
                label: "people.menu.redetect",
                hint: "people.menu.redetectHint",
                onClick: () => void reanalyse(),
                danger: true,
              },
              {
                label: "people.menu.erase",
                hint: "people.menu.eraseHint",
                onClick: () => void erase(),
                danger: true,
              },
            ]}
          />
        </div>
      </div>

      <div className={`flex-1 overflow-y-auto p-4 ${viewing ? "hidden" : ""}`}>
        {peopleOn === false && (people?.length ?? 0) === 0 ? (
          // Asked, not assumed: recognising builds a template of
          // everyone's face, which for a working photographer is their
          // clients' personal data.
          <div className="mx-auto max-w-md p-8 text-center">
            <div className="mb-3 flex justify-center text-faint opacity-60">
              <Icon name="user" size={30} strokeWidth={1.4} />
            </div>
            <div className="text-title text-ink">{t("people.optIn.title")}</div>
            <p className="mt-2 text-caption leading-relaxed text-muted">{t("people.optIn.body")}</p>
            <p className="mt-2 text-caption leading-relaxed text-faint">{t("people.optIn.culling")}</p>
            <button onClick={() => void turnOn()} disabled={working} className="btn-accent mx-auto mt-4">
              {t("people.optIn.turnOn")}
            </button>
          </div>
        ) : people === null ? (
          <div className="p-8 text-center text-caption text-faint">{t("common.loading")}</div>
        ) : people.length === 0 ? (
          failure && !stage ? (
            <FailureCard message={failure} onRetry={() => void run()} disabled={working} />
          ) : (
            <div className="mx-auto max-w-sm p-8 text-center">
              <div className="mb-3 flex justify-center text-faint opacity-60">
                <Icon name="user" size={30} strokeWidth={1.4} />
              </div>
              <div className="text-title text-muted">
                {stage
                  ? stageLabel(stage, progress)
                  : backlog > 0
                    ? t("people.workspace.notAnalysed")
                    : t("people.workspace.noPeople")}
              </div>
              <div className="mt-1 text-caption leading-relaxed text-faint">
                {stage
                  ? t("people.workspace.runsOnce")
                  : backlog > 0
                    ? t("people.workspace.backlog", { count: backlog })
                    : t("people.workspace.noFaceClear")}
              </div>
              {!stage && backlog > 0 && (
                <button onClick={() => void run()} disabled={working} className="btn-accent mx-auto mt-4">
                  {t("people.workspace.findPeople")}
                </button>
              )}
            </div>
          )
        ) : (
          <>
            {failure && !stage && (
              <FailureCard message={failure} onRetry={() => void run()} disabled={working} />
            )}
            {pair && (
              // The prompt only: the question itself is asked full size in
              // `MatchReview`, where the faces are big enough to answer it.
              <div className="mb-4 flex max-w-xl items-center gap-3 rounded-lg border border-line bg-panel p-3">
                <div className="flex shrink-0 -space-x-3">
                  {[pair.a, pair.b].map((p) => (
                    <div key={p.id} className="w-10 overflow-hidden rounded-full ring-2 ring-panel">
                      <FaceCrop
                        src={p.coverPath ? fileUrl(p.coverPath) : null}
                        box={p.coverBox}
                        portrait={p.portraitPath}
                        size={40}
                      />
                    </div>
                  ))}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="text-body font-medium text-ink">
                    {t("people.workspace.lookAlike", { count: queue.length })}
                  </div>
                  <div className="text-caption leading-relaxed text-faint">
                    {t("people.workspace.lookAlikeHint")}
                  </div>
                </div>
                <button onClick={() => setReviewing(true)} disabled={working} className="btn-accent shrink-0">
                  {t("people.workspace.review")}
                </button>
              </div>
            )}
            {visible.length > 1 && (
              <div className="mb-3 flex items-center gap-2">
                <input
                  className="input w-56"
                  placeholder={t("people.workspace.findPlaceholder")}
                  aria-label={t("people.workspace.findAria")}
                  value={findPerson}
                  onChange={(e) => setFindPerson(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") setFindPerson("");
                  }}
                />
                <label className="flex items-center gap-1.5 text-caption text-faint">
                  {t("people.workspace.sort")}
                  <select
                    className="rounded-sm border border-line bg-base px-1.5 py-1 text-caption text-ink outline-none"
                    value={peopleOrder}
                    onChange={(e) => setPeopleOrder(e.target.value as PeopleOrder)}
                    aria-label={t("people.workspace.sortAria")}
                  >
                    <option value="photos">{t("people.workspace.sortPhotos")}</option>
                    <option value="name">{t("people.workspace.sortName")}</option>
                  </select>
                </label>
              </div>
            )}
            <div className="flex flex-wrap gap-3">
              {arrangePeople(visible, findPerson, peopleOrder).map((p) => {
                const isSelected = selected.has(p.id);
                return (
                  <div
                    key={p.id}
                    className={`group relative w-[164px] overflow-hidden rounded-lg border bg-panel transition-shadow ${
                      isSelected ? "border-accent ring-1 ring-accent" : "border-line"
                    }`}
                  >
                    <button
                      onClick={(e) =>
                        e.ctrlKey || e.metaKey || selected.size > 0
                          ? toggleSelected(p.id)
                          : viewPerson(p)
                      }
                      className="relative block w-full"
                      title={
                        selected.size > 0
                          ? t("people.workspace.clickToSelect")
                          : p.name
                            ? t("people.workspace.showPhotosOf", { count: p.count, name: p.name, key: mergeKey() })
                            : t("people.workspace.showPhotos", { count: p.count, key: mergeKey() })
                      }
                    >
                      {/* Cropped to the cover face, not to whichever square of
                          the photo the face happens to fall in — a page of
                          people should show faces. */}
                      <FaceCrop
                        src={p.coverPath ? fileUrl(p.coverPath) : null}
                        box={p.coverBox}
                        portrait={p.portraitPath}
                        size={162}
                      />
                      <span className="absolute right-1.5 top-1.5 rounded-sm bg-chip px-1.5 text-label tabular-nums text-ink backdrop-blur">
                        {p.count}
                      </span>
                    </button>
                    {/* Beside the cover rather than in it, so it is its own
                        button and not a click on the photo. */}
                    <button
                      onClick={() => void hide([p.id], true)}
                      disabled={working}
                      className="absolute left-1.5 top-1.5 grid h-6 w-6 place-items-center rounded-sm bg-chip text-ink opacity-0 backdrop-blur transition-opacity hover:text-accent focus-visible:opacity-100 group-hover:opacity-100"
                      title={t("people.workspace.hideHint")}
                      aria-label={t("people.workspace.hideAria", { name: p.name ?? t("people.thisPerson") })}
                    >
                      <Icon name="eye-off" size={13} />
                    </button>

                    {/* Padding on the wrapper, not margin on the input: `.input`
                        is `w-full`, so its own margins pushed it 12px past the
                        edge of the card. */}
                    <div className="p-1.5">
                      <input
                        key={p.id}
                        className="input h-7 text-caption"
                        placeholder={t("people.workspace.namePlaceholder")}
                        defaultValue={p.name ?? ""}
                        onChange={(e) => setNames((n) => ({ ...n, [p.id]: e.target.value }))}
                        onBlur={() => void rename(p)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                        }}
                      />
                      {/* One above the other: side by side, "Gesichter
                          prüfen" and "Fotos hinzufügen" wrapped into a cramped
                          two-line row on every German and French card. */}
                      <div className="mt-1 flex flex-col">
                        <button
                          onClick={() => void openPerson(p)}
                          className="truncate rounded-sm py-0.5 text-label text-faint transition-colors hover:bg-hover hover:text-ink"
                          title={t("people.workspace.reviewFacesHint")}
                        >
                          {t("people.workspace.reviewFaces")}
                        </button>
                        <button
                          onClick={() => addPhotos(p)}
                          className="truncate rounded-sm py-0.5 text-label text-faint transition-colors hover:bg-hover hover:text-ink"
                          title={t("people.workspace.addPhotosHint")}
                        >
                          {t("people.workspace.addPhotos")}
                        </button>
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
            {hiddenPeople.length > 0 && (
              <section className="mt-8">
                <button
                  onClick={() => setShowHidden((v) => !v)}
                  className="btn-ghost -ml-2 text-muted"
                  aria-expanded={showHidden}
                >
                  <Icon name="chevron" size={13} className={showHidden ? "" : "-rotate-90"} />
                  {t("people.workspace.hiddenHeading", { count: hiddenPeople.length })}
                </button>
                {showHidden && (
                  <div className="mt-2 flex flex-wrap gap-2">
                    {hiddenPeople.map((p) => (
                      <div
                        key={p.id}
                        className="group relative overflow-hidden rounded-md border border-line bg-panel"
                        title={t("people.workspace.hiddenTile", { name: p.name ?? t("people.unnamed"), count: p.count })}
                      >
                        <div className="opacity-50">
                          <FaceCrop
                            src={p.coverPath ? fileUrl(p.coverPath) : null}
                            box={p.coverBox}
                            portrait={p.portraitPath}
                            size={92}
                          />
                        </div>
                        <button
                          onClick={() => void hide([p.id], false)}
                          disabled={working}
                          className="absolute inset-x-0 bottom-0 bg-chip py-1 text-label text-ink opacity-0 backdrop-blur transition-opacity focus-visible:opacity-100 group-hover:opacity-100"
                        >
                          {t("people.workspace.showAgain")}
                        </button>
                      </div>
                    ))}
                  </div>
                )}
              </section>
            )}
          </>
        )}
      </div>

      {review.mounted && lastPair.current && (
        <MatchReview
          a={lastPair.current.a}
          b={lastPair.current.b}
          closing={review.closing}
          working={working}
          onAnswer={(same) => void answer(same)}
          onSkip={skip}
          onClose={closeReview}
        />
      )}

      {/* Face review: the "is this really them?" grid, and who else they are. */}
      {open && (
        <FaceReview
          person={open.person}
          faces={open.faces}
          others={visible.filter((p) => p.id !== open.person.id)}
          working={working}
          onDetach={(faceId) => void detach(faceId)}
          onMerge={(into) => void mergeInto(open.person, into)}
          onCover={(faceId) => void chooseCover(faceId)}
          onClose={closeFaces}
        />
      )}

      {unplacedOpen && (
        <UnplacedFaces
          people={people ?? []}
          onChanged={() => void load()}
          onClose={() => setUnplacedOpen(false)}
        />
      )}
    </div>
  );
}
