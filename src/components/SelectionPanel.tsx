import MetadataSection from "./MetadataSection";
import { sentenceCase } from "../lib/format";
import { useWindowWidth } from "../lib/useWindowWidth";
import { useEffect, useMemo, useState } from "react";
import { api, fileUrl } from "../lib/api";
import { toast } from "../lib/dialogs";
import { useStore } from "../store";
import { Icon } from "../lib/icons";
import { toggleTarget, uniformity } from "../lib/selection";
import { parentsOf, pathOf } from "../lib/keywordTree";
import { COLOR_LABELS } from "../types";
import { comboToLabel } from "../lib/keymap";
import { formatNumber, t } from "../i18n";
import { tagName } from "../i18n/tags";
import AddToCollection from "./AddToCollection";
import PeopleSection from "./PeopleSection";
import PhotoMeta from "./PhotoMeta";

/** Marks a control whose value differs across the selection. */
function Mixed() {
  return (
    <span
      className="rounded-sm bg-raised px-1 py-px text-label font-medium text-faint"
      title={t("library.selection.mixedTooltip")}
    >
      {t("library.selection.mixed")}
    </span>
  );
}

/**
 * The contextual panel for a grid selection. Replaces the old ~20-control
 * horizontal action bar with a grouped right-hand panel: the focused photo and
 * its capture data, then rate/flag, colour, place, keywords, collection, and the
 * two big actions. Every control drives an existing store action; the panel is
 * pure re-presentation.
 */
export default function SelectionPanel() {
  const selected = useStore((s) => s.selected);
  const photos = useStore((s) => s.searchResults ?? s.photos);
  const focusId = useStore((s) => s.focusId);
  const clearSelection = useStore((s) => s.clearSelection);
  const rateSelection = useStore((s) => s.rateSelection);
  const flagSelection = useStore((s) => s.flagSelection);
  const colorLabelSelection = useStore((s) => s.colorLabelSelection);
  const assignPlaceToSelection = useStore((s) => s.assignPlaceToSelection);
  const clearPlaceFromSelection = useStore((s) => s.clearPlaceFromSelection);
  const removeSelectionFromCollection = useStore((s) => s.removeSelectionFromCollection);
  const shownCollection = useStore((s) =>
    s.filter.collectionId == null ? null : (s.collections.find((c) => c.id === s.filter.collectionId) ?? null),
  );
  const addTagToSelection = useStore((s) => s.addTagToSelection);
  const keywordNodes = useStore((s) => s.keywords);
  const confirmKeywords = useStore((s) => s.confirmKeywords);
  const setCaptureTimeFor = useStore((s) => s.setCaptureTimeFor);
  const showAiKeywords = useStore((s) => s.showAiKeywords);
  const keywordParents = useMemo(() => parentsOf(keywordNodes), [keywordNodes]);
  const compareSelection = useStore((s) => s.compareSelection);
  const findSimilar = useStore((s) => s.findSimilar);
  const openDevelop = useStore((s) => s.openDevelop);
  const exportSelection = useStore((s) => s.exportSelection);
  const reload = useStore((s) => s.reload);
  const emptyHidden = useStore((s) => s.emptyPanelHidden);
  const setEmptyHidden = useStore((s) => s.setEmptyPanelHidden);
  const clearCombo = useStore((s) => s.keymap["clear-selection"]);

  const [place, setPlace] = useState("");
  const [placeNames, setPlaceNames] = useState<string[]>([]);
  const [tag, setTag] = useState("");

  const count = selected.size;
  const selectedIds = useMemo(() => [...selected], [selected]);

  /**
   * A change to the whole selection from this panel. With one photo its
   * result is right here in the panel; with many the panel shows one of
   * them, so a toast says how far it reached. A failure says why and keeps
   * what was typed — these used to fail with no word at all.
   */
  const apply = async (work: () => Promise<unknown>, many: string): Promise<boolean> => {
    try {
      await work();
      if (count > 1) toast(many, "success");
      return true;
    } catch (e) {
      toast(String(e).replace(/^Error: /, ""), "error");
      return false;
    }
  };
  // The tagger's unconfirmed keywords on the selection, asked for when
  // it or its photos change — the grid's own query doesn't carry them.
  const [ai, setAi] = useState<Map<number, string[]>>(new Map());
  useEffect(() => {
    if (selectedIds.length === 0) return;
    let live = true;
    api
      .aiKeywords(selectedIds)
      .then((list) => live && setAi(new Map(list.map((k) => [k.photoId, k.names]))))
      .catch(() => live && setAi(new Map()));
    return () => {
      live = false;
    };
  }, [selectedIds, photos]);
  // The focused photo, or the first selected as a fallback, drives the *preview*
  // (thumbnail, filename, capture data) — one photo standing in for the set.
  const focus = photos.find((p) => p.id === (focusId ?? [...selected][0]));
  const focusAi = (focus && ai.get(focus.id)) ?? [];
  const shownSubjects = focus ? (showAiKeywords ? focus.subjects : focus.subjects.filter((s) => !focusAi.includes(s))) : [];
  const suggestedCount = selectedIds.reduce((n, id) => n + (ai.get(id)?.length ?? 0), 0);
  // How many of the selection have a place to take off.
  const placed = photos.filter((p) => selected.has(p.id) && p.placeId != null).length;

  // The verdict controls, by contrast, read the whole selection: they write to
  // all of it, so showing one photo's value made them misreport what a click
  // would do. `toggleTarget` then clears only when the set is uniformly there.
  const chosen = useMemo(() => photos.filter((p) => selected.has(p.id)), [photos, selected]);
  const rating = useMemo(() => uniformity(chosen.map((p) => p.rating)), [chosen]);
  const flag = useMemo(() => uniformity(chosen.map((p) => p.flag)), [chosen]);
  const colorLabel = useMemo(() => uniformity(chosen.map((p) => p.colorLabel)), [chosen]);
  // Room for the panel to keep its slot with nothing selected.
  const roomy = useWindowWidth() >= 1180;

  // Its slot stays when the window has room: appearing and going with
  // the selection re-laid the whole grid on every click. Narrower, where a
  // permanent panel would leave the grid a column wide, it still comes and goes,
  // and so it does once the empty slot is closed: it had no way to close, and
  // held a column of the grid back for a hint.
  if (count === 0) {
    if (!roomy || emptyHidden) return null;
    const hide = () => {
      setEmptyHidden(true);
      toast(t("library.selection.hiddenEmpty"), "info", {
        label: t("common.undo"),
        run: () => setEmptyHidden(false),
      });
    };
    return (
      <aside className="flex w-[var(--panel-w)] shrink-0 flex-col border-l border-line bg-panel">
        {/* Where the selection's own × sits, so the corner means "close" either way. */}
        <div className="flex shrink-0 justify-end px-4 py-3">
          <button
            onClick={hide}
            className="icon-chip"
            title={t("library.selection.hideEmpty")}
            aria-label={t("library.selection.hideEmpty")}
          >
            <Icon name="close" size={15} />
          </button>
        </div>
        <div className="flex flex-1 flex-col items-center justify-center gap-2 px-6 pb-14 text-center">
          <Icon name="grid" size={22} className="text-faint" />
          <p className="text-body text-muted">{t("library.selection.emptyTitle")}</p>
          <p className="text-caption leading-relaxed text-faint">{t("library.selection.emptyHint")}</p>
        </div>
      </aside>
    );
  }

  return (
    <aside className="flex w-[var(--panel-w)] shrink-0 flex-col border-l border-line bg-panel">
      {/* Everything scrolls but the actions, which stay in sight:
          Develop and Export sat at the end of the scroll, below the fold. */}
      <div className="min-h-0 flex-1 overflow-y-auto">
      {/* Header */}
      <div className="flex items-center gap-2.5 border-b border-line/60 px-4 py-3">
        <div className="grid h-8 min-w-8 shrink-0 place-items-center rounded-md bg-raised px-1.5 text-body font-semibold tabular-nums text-ink">
          {formatNumber(count)}
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-body font-semibold text-ink">{t("common.selected", { count })}</div>
          {focus && <div className="truncate text-caption text-faint">{focus.filename}</div>}
        </div>
        <button
          onClick={clearSelection}
          className="icon-chip"
          // With its key, so the shortcut can be found from here.
          title={clearCombo ? t("library.selection.clearKey", { key: comboToLabel(clearCombo) }) : t("library.selection.clear")}
          aria-label={t("library.selection.clear")}
        >
          <Icon name="close" size={15} />
        </button>
      </div>

      {/* Focused preview + capture */}
      {focus && (
        <div className="border-b border-line/60 px-4 py-3.5">
          <div className="mb-3 aspect-[3/2] w-full overflow-hidden rounded-md bg-raised">
            {focus.thumbPath ? (
              <img
                src={fileUrl(focus.thumbPath)}
                alt={focus.filename}
                draggable={false}
                className="h-full w-full object-cover"
              />
            ) : (
              <div className="grid h-full place-items-center text-faint">?</div>
            )}
          </div>
          <PhotoMeta photo={focus} />
          {/* The time is only as right as the camera's clock was. */}
          <button
            onClick={() => setCaptureTimeFor(selectedIds)}
            className="mt-1.5 text-caption text-accent hover:underline"
            title={t("library.captureTime.openTitle")}
          >
            {focus.captureEdited ? t("library.captureTime.openEdited") : t("library.captureTime.open")}
          </button>
        </div>
      )}

      {/* Rate & flag */}
      <div className="border-b border-line/60 px-4 py-3.5">
        <div className="section-label px-0 pb-2">{t("library.selection.rateAndFlag")}</div>
        <div className="flex items-center gap-2">
          <div className="flex items-center gap-1 text-lg">
            {[1, 2, 3, 4, 5].map((n) => (
              <button
                key={n}
                onClick={() => void rateSelection(toggleTarget(rating, n))}
                aria-label={count === 1 ? t("library.photoMenu.rate", { rating: n }) : t("library.selection.rateAll", { photos: count, count: n })}
                aria-pressed={rating.kind === "uniform" && rating.value === n}
                className={`leading-none ${
                  rating.kind === "uniform" && rating.value >= n
                    ? "text-accent"
                    : "text-faint hover:text-muted"
                }`}
                title={count === 1 ? t("library.photoMenu.rate", { rating: n }) : t("library.selection.rateAll", { photos: count, count: n })}
              >
                ★
              </button>
            ))}
          </div>
          {rating.kind === "mixed" && <Mixed />}
          <div className="flex-1" />
          {flag.kind === "mixed" && <Mixed />}
          <button
            onClick={() => void flagSelection(toggleTarget(flag, 1))}
            aria-pressed={flag.kind === "uniform" && flag.value === 1}
            className={`icon-chip ${flag.kind === "uniform" && flag.value === 1 ? "seg-on text-positive" : ""}`}
            title={count === 1 ? t("library.photoMenu.pick") : t("library.selection.pickAll", { count })}
          >
            <Icon name="flag" size={17} />
          </button>
          <button
            onClick={() => void flagSelection(toggleTarget(flag, -1))}
            aria-pressed={flag.kind === "uniform" && flag.value === -1}
            className={`icon-chip ${flag.kind === "uniform" && flag.value === -1 ? "seg-on text-danger" : ""}`}
            title={count === 1 ? t("library.photoMenu.reject") : t("library.selection.rejectAll", { count })}
          >
            <Icon name="flag-off" size={17} />
          </button>
        </div>
      </div>

      {/* Colour label */}
      <div className="border-b border-line/60 px-4 py-3.5">
        <div className="section-label flex items-center gap-2 px-0 pb-2">
          <span>{t("library.selection.colourLabel")}</span>
          {colorLabel.kind === "mixed" && <Mixed />}
        </div>
        <div className="flex items-center gap-2.5">
          {COLOR_LABELS.map((c) => (
            <button
              key={c.value}
              onClick={() => void colorLabelSelection(toggleTarget(colorLabel, c.value))}
              aria-label={count === 1 ? t(c.nameKey) : t("library.selection.colourAll", { count, name: t(c.nameKey) })}
              aria-pressed={colorLabel.kind === "uniform" && colorLabel.value === c.value}
              className={`h-5 w-5 rounded-full border transition-transform hover:scale-110 ${
                colorLabel.kind === "uniform" && colorLabel.value === c.value
                  ? "border-ink ring-2 ring-accent"
                  : "border-black/30"
              }`}
              style={{ backgroundColor: c.hex }}
              title={count === 1 ? t(c.nameKey) : t("library.selection.colourAll", { count, name: t(c.nameKey) })}
            />
          ))}
          <button
            onClick={() => void colorLabelSelection(0)}
            className="ml-1 text-caption text-faint hover:text-ink"
            title={count === 1 ? t("library.selection.clearLabelOne") : t("library.selection.clearLabelAll", { count })}
          >
            {t("library.selection.clearLabel")}
          </button>
        </div>
      </div>

      {/* Place */}
      <div className="border-b border-line/60 px-4 py-3.5">
        <div className="section-label px-0 pb-2">{t("library.selection.place")}</div>
        {/* Suggests the places already named, so "Skye" and "Isle of Skye"
            don't become two. Read on focus: places change in another
            workspace, and this panel outlives the visit. */}
        <input
          className="input"
          list="place-names-selection"
          placeholder={t("library.selection.placePlaceholder")}
          onFocus={() =>
            void api
              .listPlaces()
              .then((ps) => setPlaceNames(ps.map((p) => p.name)))
              .catch(() => {})
          }
          value={place}
          onChange={(e) => setPlace(e.target.value)}
          onKeyDown={(e) => {
            const name = place.trim();
            if (e.key === "Enter" && name)
              void apply(
                () => assignPlaceToSelection(name),
                t("library.selection.placedMany", { count, place: name }),
              ).then((ok) => ok && setPlace(""));
          }}
        />
        <datalist id="place-names-selection">
          {placeNames.map((n) => (
            <option key={n} value={n} />
          ))}
        </datalist>
        {placed > 0 && (
          <button
            onClick={() =>
              void apply(
                () => clearPlaceFromSelection(),
                t("library.selection.unplacedMany", { count: placed }),
              )
            }
            className="mt-1.5 text-caption text-accent hover:underline"
            title={t("library.selection.removePlaceTooltip")}
          >
            {count > 1 ? t("library.selection.removePlaceMany", { count: placed }) : t("library.selection.removePlace")}
          </button>
        )}
      </div>

      {/* Keywords */}
      <div className="border-b border-line/60 px-4 py-3.5">
        <div className="section-label px-0 pb-2">{t("library.selection.keywords")}</div>
        {focus && shownSubjects.length > 0 && (
          <div className="mb-2 flex flex-wrap gap-1">
            {shownSubjects.map((s) => {
              // The tagger's, unconfirmed: dashed, with a tick to make
              // it the user's; its × turns it down for this photo for good.
              const suggested = focusAi.includes(s);
              const where = pathOf(keywordParents, s).map(tagName).join(" › ");
              return (
                <span
                  key={s}
                  className={`chip ${suggested ? "border-dashed !border-faint" : ""}`}
                  title={suggested ? t("library.selection.suggestedTitle", { path: where }) : where}
                >
                  {sentenceCase(tagName(s))}
                  {suggested && (
                    <button
                      onClick={() => void confirmKeywords([focus.id], [s])}
                      className="chip-x hover:!text-positive"
                      title={t("library.selection.confirmTitle")}
                      aria-label={t("library.selection.confirmAria", { tag: tagName(s) })}
                    >
                      <Icon name="check" size={11} strokeWidth={2.6} />
                    </button>
                  )}
                  <button
                    onClick={() => void api.removeTag([focus.id], s).then(() => void reload())}
                    className="chip-x"
                    title={suggested ? t("library.selection.declineTitle") : t("library.selection.takeOffTitle")}
                    aria-label={t("library.selection.removeTag", { tag: tagName(s) })}
                  >
                    <Icon name="close" size={11} strokeWidth={2.4} />
                  </button>
                </span>
              );
            })}
          </div>
        )}
        {showAiKeywords && suggestedCount > 0 && (
          <button
            onClick={() => void confirmKeywords(selectedIds, [])}
            className="mb-2 block text-caption text-accent hover:underline"
            title={t("library.selection.confirmAllTitle")}
          >
            {count > 1
              ? t("library.selection.confirmAllMany", { count: suggestedCount, photos: formatNumber(count) })
              : t("library.selection.confirmAll", { count: suggestedCount })}
          </button>
        )}
        <input
          className="input"
          placeholder={count > 1 ? t("library.selection.addTagMany", { count }) : t("library.selection.addTag")}
          title={t("library.selection.addTagTitle")}
          list="keyword-names-selection"
          value={tag}
          onChange={(e) => setTag(e.target.value)}
          onKeyDown={(e) => {
            const name = tag.trim();
            if (e.key === "Enter" && name)
              void apply(
                () => addTagToSelection(name),
                t("library.selection.keywordedMany", { count, keyword: name }),
              ).then((ok) => ok && setTag(""));
          }}
        />
        {/* Existing keywords, each labelled with where it is filed. */}
        <datalist id="keyword-names-selection">
          {keywordNodes.map((k) => (
            <option key={k.name} value={k.name} label={pathOf(keywordParents, k.name).join(" › ")} />
          ))}
        </datalist>
      </div>

      <MetadataSection ids={selectedIds} />

      <PeopleSection />

      {/* Collection */}
      <div className="border-b border-line/60 px-4 py-3.5">
        <div className="section-label px-0 pb-2">{t("library.selection.collection")}</div>
        <AddToCollection />
        {shownCollection && (
          <button
            onClick={() => void removeSelectionFromCollection()}
            className="mt-1.5 text-caption text-accent hover:underline"
            title={t("library.selection.removeFromCollectionTooltip")}
          >
            {count > 1
              ? t("library.selection.removeFromCollectionMany", { count, name: shownCollection.name })
              : t("library.selection.removeFromCollection", { name: shownCollection.name })}
          </button>
        )}
      </div>

      </div>

      {/* Primary actions */}
      <div className="flex shrink-0 flex-col gap-2 border-t border-line px-4 py-3.5">
        {/* Half each where both labels fit, a row each where they don't:
            side by side, "Rechercher des similaires" pushed "Exporter" 28px
            out of the panel. */}
        <div className="flex flex-wrap gap-2">
          {/* One photo can't be compared, but it can be matched: the slot is
              "Find similar" then, which used to be a search-box mode. */}
          {count === 1 && focus ? (
            <button
              onClick={() => void findSimilar(focus.id, focus.filename)}
              className="btn-solid flex-1 basis-[calc(50%-4px)] justify-center"
              title={t("library.selection.findSimilarTooltip")}
            >
              <Icon name="compare" size={15} />
              {t("library.photoMenu.findSimilar")}
            </button>
          ) : (
          <button
            onClick={compareSelection}
            className="btn-solid flex-1 basis-[calc(50%-4px)] justify-center"
            disabled={count < 2}
            title={
              count < 2 ? t("library.selection.compareNeedsTwo") : t("library.selection.compareTooltip")
            }
          >
            <Icon name="compare" size={15} />
            {t("library.selection.compare")}
          </button>
          )}
          <button
            onClick={() => void exportSelection()}
            data-tour="export"
            className="btn-solid flex-1 basis-[calc(50%-4px)] justify-center"
            title={t("library.selection.exportTooltip")}
          >
            <Icon name="import" size={15} className="rotate-180" />
            {t("library.selection.export")}
          </button>
        </div>
        {/* Sized to its label, not a full-width slab: the key is heavy enough by
            weight, and a panel-wide bar is the generated-dashboard pattern. */}
        <button
          onClick={() => focus && void openDevelop(focus.id)}
          className="btn-accent self-end"
          disabled={!focus}
          title={t("library.selection.developTooltip")}
        >
          <Icon name="adjust" size={15} />
          {t("library.selection.develop")}
        </button>
      </div>
    </aside>
  );
}
