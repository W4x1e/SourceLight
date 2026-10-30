import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Icon } from "../lib/icons";
import NumberSlider from "./NumberSlider";
import { t } from "../i18n";
import type { LutInfo, Preset, PresetGroupInfo, PresetInfo, Suggestion } from "../bindings";
import {
  lookBlurb,
  lookFamilyName,
  lookMatches,
  lookTitle,
  presetDescription,
  presetGroupDescription,
  presetGroupName,
  presetMatches,
  presetName,
} from "../lib/content";

/**
 * The Presets panel: the built-in presets, the user's own, and the LUTs, each
 * previewed on the photo in front of you. It sits on the other side of the
 * canvas from the adjustments, so a preset and the sliders it moved are both
 * in view.
 *
 * Two tabs, because they are two different things. A preset sets the creative
 * sliders (contrast, curves, colour, grading, effects) and can be fine-tuned
 * or turned up and down afterwards. A LUT is a fixed colour transform laid over
 * the finished image. They also combine, which is easier to see when each has
 * its own place.
 *
 * The Presets tab reads top to bottom as: what is on and how much of it (the
 * Amount slider, pinned so it is there the moment a preset is applied), what
 * suits this photo, the user's own presets, then the built-in groups, by style
 * and then by subject. Groups start folded, so the tab opens as a short list
 * of what there is; a group's previews are rendered when it is opened.
 *
 * Pure presentation: every action is a callback into Develop, which owns the
 * develop session.
 */

type Tab = "presets" | "luts";

const TAB_KEY = "developPresetsTab";
const WIDTH_KEY = "developLooksWidth";
const COLLAPSED_KEY = "developLooksCollapsed";
/** Built-in groups the user has opened. Folded is the default for those. */
const OPENED_KEY = "developPresetGroupsOpen";
const MIN_W = 220;
const MAX_W = 620;
const DEFAULT_W = 300;

/** The stored panel width, clamped — a bad or missing value falls back. */
function storedWidth(): number {
  const raw = Number(localStorage.getItem(WIDTH_KEY));
  if (!Number.isFinite(raw) || raw <= 0) return DEFAULT_W;
  return Math.min(MAX_W, Math.max(MIN_W, raw));
}

function storedList(key: string): string[] {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(key) ?? "[]");
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

function storedTab(): Tab {
  return localStorage.getItem(TAB_KEY) === "luts" ? "luts" : "presets";
}

/** One thing the user can click to apply, drawn as a preview tile. */
interface TileProps {
  title: string;
  blurb: string;
  preview: string | undefined;
  active: boolean;
  aspect: number;
  onClick: () => void;
  onRemove?: () => void;
  removeTitle?: string;
  onGroup?: () => void;
  groupTitle?: string;
  onUngroup?: () => void;
  ungroupTitle?: string;
}

function Tile(p: TileProps) {
  const corner = (
    onClick: (() => void) | undefined,
    icon: "minus" | "folder" | "close",
    title: string | undefined,
    danger = false,
  ) =>
    onClick && (
      <button
        onClick={(ev) => {
          ev.stopPropagation();
          onClick();
        }}
        className={`rounded-sm bg-chip p-0.5 text-faint backdrop-blur ${danger ? "hover:text-danger" : "hover:text-ink"}`}
        title={title}
      >
        <Icon name={icon} size={11} />
      </button>
    );
  return (
    <div className="group relative">
      <button
        onClick={p.onClick}
        title={p.blurb || t("develop.presets.apply", { name: p.title })}
        aria-pressed={p.active}
        className={`block w-full overflow-hidden rounded-md border text-left transition-colors ${
          p.active ? "border-accent" : "border-line hover:border-line-hover"
        }`}
      >
        {p.preview ? (
          <img src={p.preview} alt="" style={{ aspectRatio: p.aspect }} className="w-full object-cover" />
        ) : (
          <div style={{ aspectRatio: p.aspect }} className="w-full animate-pulse bg-raised" />
        )}
        <span
          aria-pressed={p.active}
          className={`block truncate px-1.5 py-1 text-caption ${p.active ? "seg-on" : "text-muted group-hover:text-ink"}`}
        >
          {p.title}
        </span>
      </button>
      {(p.onGroup || p.onRemove) && (
        <div className="absolute right-1 top-1 flex gap-1 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
          {corner(p.onUngroup, "minus", p.ungroupTitle)}
          {corner(p.onGroup, "folder", p.groupTitle)}
          {corner(p.onRemove, "close", p.removeTitle, true)}
        </div>
      )}
    </div>
  );
}

/** A foldable heading with a count, sticky while its items scroll under it. */
function GroupHeading({
  label,
  count,
  open,
  onToggle,
  description,
}: {
  label: string;
  count: number;
  open: boolean;
  onToggle: () => void;
  description?: string;
}) {
  return (
    <div className="sticky top-0 z-10 -mx-2 bg-panel px-1 pb-1 pt-1">
      <button
        onClick={onToggle}
        aria-expanded={open}
        className="flex w-full items-start gap-1 rounded-sm px-2 py-1 text-left hover:bg-hover"
        title={open ? t("develop.presets.collapseGroup", { group: label }) : t("develop.presets.showGroup", { group: label })}
      >
        <Icon
          name="chevron"
          size={12}
          className={`mt-0.5 shrink-0 text-faint transition-transform ${open ? "" : "-rotate-90"}`}
        />
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-1">
            <span className="truncate text-caption font-semibold text-ink">{label}</span>
            <span className="ml-auto shrink-0 text-label tabular-nums text-faint">{count}</span>
          </span>
          {description && <span className="block text-label leading-snug text-faint">{description}</span>}
        </span>
      </button>
    </div>
  );
}

/** Previews in as many columns as the panel's width holds at a readable size. */
const gridStyle = { gridTemplateColumns: "repeat(auto-fill, minmax(118px, 1fr))" };

export default function DevelopPresets({
  groups,
  presetId,
  presetAmount,
  onUseBuiltin,
  onAmountChange,
  onAmountCommit,
  builtinPreviews,
  onWantPreviews,
  suggestions,
  suggesting,
  onRefreshSuggestions,
  onApplySuggestion,
  saved,
  savedPreviews,
  onUseSaved,
  onRemoveSaved,
  onStoreSaved,
  luts,
  lutName,
  lutIntensity,
  lutPreviews,
  onChooseLut,
  onRemoveLut,
  onImportLut,
  onGroupLut,
  onUngroupLut,
  onLutIntensityChange,
  onLutIntensityCommit,
  onTabShown,
  aspect,
  onClose,
  maxWidth,
}: {
  groups: PresetGroupInfo[];
  /** The built-in preset on the edit ("" for none) and its amount, 1 as designed. */
  presetId: string;
  presetAmount: number;
  /** Apply a built-in preset at full amount, or take it off with "". */
  onUseBuiltin: (id: string) => void;
  onAmountChange: (amount: number) => void;
  onAmountCommit: () => void;
  /** Built-in previews rendered so far, by preset id. */
  builtinPreviews: Record<string, string>;
  /** Ask for the previews of these presets, once a group is on screen. */
  onWantPreviews: (ids: string[]) => void;
  suggestions: Suggestion[];
  suggesting: boolean;
  onRefreshSuggestions: () => void;
  onApplySuggestion: (s: Suggestion) => void;
  saved: Preset[];
  /** Saved-preset previews, keyed by the preset's id as a string. */
  savedPreviews: Record<string, string>;
  onUseSaved: (id: number) => void;
  onRemoveSaved: (id: number) => void;
  onStoreSaved: () => void;
  luts: LutInfo[];
  lutName: string;
  lutIntensity: number;
  lutPreviews: Record<string, string>;
  onChooseLut: (name: string) => void;
  onRemoveLut: (name: string) => void;
  onImportLut: () => void;
  /** Ask for a group name for this LUT. Gets its current heading and whether
   *  that heading is one the user assigned. */
  onGroupLut: (name: string, family: string, grouped: boolean) => void;
  onUngroupLut: (name: string) => void;
  onLutIntensityChange: (v: number) => void;
  onLutIntensityCommit: () => void;
  /** Told which tab is showing, so Develop renders only that tab's previews. */
  onTabShown: (tab: Tab) => void;
  /** The photo's width/height, so previews are shown in its own shape. */
  aspect: number;
  onClose: () => void;
  /** How wide the window lets it be. */
  maxWidth?: number;
}) {
  const [tab, setTab] = useState<Tab>(storedTab);
  const [query, setQuery] = useState("");
  const [width, setWidth] = useState(storedWidth);
  const [opened, setOpened] = useState<string[]>(() => storedList(OPENED_KEY));
  const [collapsed, setCollapsed] = useState<string[]>(() => storedList(COLLAPSED_KEY));

  useEffect(() => localStorage.setItem(WIDTH_KEY, String(width)), [width]);
  useEffect(() => localStorage.setItem(OPENED_KEY, JSON.stringify(opened)), [opened]);
  useEffect(() => localStorage.setItem(COLLAPSED_KEY, JSON.stringify(collapsed)), [collapsed]);
  useEffect(() => {
    localStorage.setItem(TAB_KEY, tab);
    onTabShown(tab);
  }, [tab, onTabShown]);

  // Drag-to-resize: the width is the pointer's distance from the panel's left
  // edge, so the handle never drifts from the cursor after a clamped drag.
  const panelRef = useRef<HTMLElement | null>(null);
  const [dragging, setDragging] = useState(false);
  const onResizeDown = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    setDragging(true);
  }, []);
  const onResizeMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      if (!dragging) return;
      const left = panelRef.current?.getBoundingClientRect().left ?? 0;
      setWidth(Math.min(MAX_W, Math.max(MIN_W, Math.round(e.clientX - left))));
    },
    [dragging],
  );
  const endResize = useCallback(() => setDragging(false), []);

  const q = query.trim().toLowerCase();
  /** Escape empties a search box, as it does the library's; an empty one
   *  lets go of the keyboard. */
  const clearOnEscape = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== "Escape") return;
    e.preventDefault();
    if (query) setQuery("");
    else e.currentTarget.blur();
  };
  const toggle = (list: string[], set: (v: string[]) => void, key: string) =>
    set(list.includes(key) ? list.filter((x) => x !== key) : [...list, key]);

  // ------------------------------------------------------------ Presets tab

  /** Where each built-in preset lives, for the applied card's "Group · Name". */
  const groupOf = useMemo(() => {
    const m = new Map<string, PresetGroupInfo>();
    for (const g of groups) for (const p of g.presets) m.set(p.id, g);
    return m;
  }, [groups]);

  const matching = (g: PresetGroupInfo): PresetInfo[] =>
    q ? g.presets.filter((p) => presetMatches(p, q) || presetGroupName(g.id, g.name).toLowerCase().includes(q)) : g.presets;
  const savedShown = q ? saved.filter((p) => p.name.toLowerCase().includes(q)) : saved;
  // A search opens every group it finds something in: folded results would
  // read as no results.
  const isOpen = (id: string) => q !== "" || opened.includes(id);

  // Ask for the previews of every group on screen. Develop renders each id
  // once per photo, so asking again is free.
  const wanted = groups
    .filter((g) => isOpen(g.id))
    .flatMap((g) => matching(g).map((p) => p.id))
    .join(",");
  useEffect(() => {
    if (tab === "presets" && wanted) onWantPreviews(wanted.split(","));
  }, [tab, wanted, onWantPreviews]);

  const applied = presetId ? groupOf.get(presetId) : undefined;
  const appliedInfo = applied?.presets.find((p) => p.id === presetId);
  const suggestionName = (s: Suggestion) =>
    s.kind === "auto" ? t("develop.auto.label") : presetName(s.id, s.name);

  const builtinTile = (p: PresetInfo) => (
    <Tile
      key={p.id}
      title={presetName(p.id, p.name)}
      blurb={presetDescription(p.id, p.description)}
      preview={builtinPreviews[p.id]}
      active={presetId === p.id}
      aspect={aspect}
      onClick={() => onUseBuiltin(presetId === p.id ? "" : p.id)}
    />
  );

  const kindSection = (kind: "style" | "subject", label: string) => {
    const shown = groups.filter((g) => g.kind === kind && matching(g).length > 0);
    if (shown.length === 0) return null;
    return (
      <div className="mb-2">
        <div className="section-label px-1 pb-1 pt-2">{label}</div>
        {shown.map((g) => {
          const open = isOpen(g.id);
          const items = matching(g);
          return (
            <div key={g.id} className="mb-1">
              <GroupHeading
                label={presetGroupName(g.id, g.name)}
                description={presetGroupDescription(g.id, g.description)}
                count={items.length}
                open={open}
                onToggle={() => toggle(opened, setOpened, g.id)}
              />
              {open && (
                <div className="mb-2 mt-1 grid gap-1.5" style={gridStyle}>
                  {items.map(builtinTile)}
                </div>
              )}
            </div>
          );
        })}
      </div>
    );
  };

  const savedOpen = q !== "" || !collapsed.includes("saved");
  const presetsTab = (
    <>
      {/* What is on and how much of it, pinned: the Amount is the control you
          want a second after applying a preset. */}
      {presetId && (
        <div className="border-b border-line bg-raised/60 px-2.5 py-2">
          <div className="flex items-center gap-2 pb-1">
            <span className="section-label px-0">{t("develop.presets.applied")}</span>
            <span className="min-w-0 flex-1 truncate text-caption text-ink">
              {applied && appliedInfo
                ? t("develop.presets.appliedName", {
                    group: presetGroupName(applied.id, applied.name),
                    name: presetName(appliedInfo.id, appliedInfo.name),
                  })
                : presetId}
            </span>
            <button
              onClick={() => onUseBuiltin("")}
              className="btn-ghost shrink-0 text-label"
              title={t("develop.presets.takeOff")}
            >
              {t("common.remove")}
            </button>
          </div>
          <NumberSlider
            label={t("develop.presets.amount")}
            value={Math.round(presetAmount * 100)}
            min={0}
            max={150}
            step={1}
            precision={0}
            suffix="%"
            defaultValue={100}
            onChange={(v) => onAmountChange(v / 100)}
            onCommit={onAmountCommit}
          />
        </div>
      )}

      <div className="border-b border-line px-2 py-2">
        <input
          className="input"
          placeholder={t("develop.presets.findPlaceholder")}
          aria-label={t("develop.presets.findPlaceholder")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={clearOnEscape}
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
        {!q && (
          <div className="mb-3">
            <div className="flex items-center justify-between px-1 pb-1">
              <span className="section-label px-0">{t("develop.presets.forThisPhoto")}</span>
              <button
                onClick={onRefreshSuggestions}
                disabled={suggesting}
                className="btn-ghost gap-1 text-label disabled:opacity-50"
                title={t("develop.presets.refreshTitle")}
              >
                {suggesting ? t("develop.presets.rendering") : t("develop.presets.refresh")}
              </button>
            </div>
            <div className="grid gap-1.5" style={gridStyle}>
              {suggestions.map((s) => (
                <Tile
                  key={`${s.kind}:${s.id}`}
                  title={suggestionName(s)}
                  blurb=""
                  preview={s.preview}
                  active={s.kind === "preset" && s.id === presetId}
                  aspect={aspect}
                  onClick={() => onApplySuggestion(s)}
                />
              ))}
              {suggestions.length === 0 &&
                suggesting &&
                [0, 1, 2, 3].map((i) => (
                  <div key={i} style={{ aspectRatio: aspect }} className="w-full animate-pulse rounded-md bg-raised" />
                ))}
            </div>
            {/* A heading over nothing read as still loading. */}
            {suggestions.length === 0 && !suggesting && (
              <p className="px-1 text-caption leading-relaxed text-faint">{t("develop.presets.noSuggestions")}</p>
            )}
          </div>
        )}

        {(savedShown.length > 0 || !q) && (
          <div className="mb-2">
            <GroupHeading
              label={t("develop.presets.yours")}
              count={savedShown.length}
              open={savedOpen}
              onToggle={() => toggle(collapsed, setCollapsed, "saved")}
            />
            {savedOpen &&
              (savedShown.length > 0 ? (
                <div className="mb-2 mt-1 grid gap-1.5" style={gridStyle}>
                  {savedShown.map((p) => (
                    <Tile
                      key={p.id}
                      title={p.name}
                      blurb=""
                      preview={savedPreviews[String(p.id)]}
                      active={false}
                      aspect={aspect}
                      onClick={() => onUseSaved(p.id)}
                      onRemove={() => onRemoveSaved(p.id)}
                      removeTitle={t("develop.presets.delete", { title: p.name })}
                    />
                  ))}
                </div>
              ) : (
                <p className="px-2 pb-2 text-caption leading-relaxed text-faint">{t("develop.presets.emptyYours")}</p>
              ))}
          </div>
        )}

        {kindSection("style", t("develop.presets.byStyle"))}
        {kindSection("subject", t("develop.presets.bySubject"))}

        {q && savedShown.length === 0 && groups.every((g) => matching(g).length === 0) && (
          <p className="px-1 py-2 text-caption leading-relaxed text-faint">{t("develop.presets.emptyNoMatch")}</p>
        )}
      </div>

      <div className="flex gap-1.5 border-t border-line p-2">
        <button onClick={onStoreSaved} className="btn-solid flex-1 justify-center text-caption">
          {t("develop.presets.saveAsPreset")}
        </button>
      </div>
    </>
  );

  // ------------------------------------------------------------ LUTs tab

  const lutsShown = q ? luts.filter((l) => lookMatches(l, q)) : luts;
  // Families in first-appearance order: the built-in families, then the
  // user's own groups, then Imported, as the core lists them.
  const families: { family: string; label: string; items: LutInfo[] }[] = [];
  for (const l of lutsShown) {
    const f = families.find((x) => x.family === l.family);
    if (f) f.items.push(l);
    else families.push({ family: l.family, label: lookFamilyName(l.family, l.grouped), items: [l] });
  }
  const activeLut = luts.find((l) => l.name === lutName);

  const lutsTab = (
    <>
      {lutName && (
        <div className="border-b border-line bg-raised/60 px-2.5 py-2">
          <div className="flex items-center gap-2 pb-1">
            <span className="section-label px-0">{t("develop.presets.applied")}</span>
            <span className="min-w-0 flex-1 truncate text-caption text-ink">
              {activeLut ? lookTitle(activeLut) : lutName}
            </span>
            <button
              onClick={() => onChooseLut("")}
              className="btn-ghost shrink-0 text-label"
              title={t("develop.luts.takeOff")}
            >
              {t("common.remove")}
            </button>
          </div>
          <NumberSlider
            label={t("develop.luts.intensity")}
            value={lutIntensity}
            min={0}
            max={1}
            step={0.01}
            defaultValue={1}
            onChange={onLutIntensityChange}
            onCommit={onLutIntensityCommit}
          />
        </div>
      )}

      <div className="border-b border-line px-2 py-2">
        <input
          className="input"
          placeholder={t("develop.luts.findPlaceholder")}
          aria-label={t("develop.luts.findPlaceholder")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={clearOnEscape}
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
        <p className="px-1 pb-2 text-label leading-snug text-faint">{t("develop.luts.about")}</p>
        {families.map((f) => {
          const open = q !== "" || !collapsed.includes(`lut:${f.family}`);
          return (
            <div key={f.family} className="mb-2">
              <GroupHeading
                label={f.label}
                count={f.items.length}
                open={open}
                onToggle={() => toggle(collapsed, setCollapsed, `lut:${f.family}`)}
              />
              {open && (
                <div className="mb-2 mt-1 grid gap-1.5" style={gridStyle}>
                  {f.items.map((l) => (
                    <Tile
                      key={l.name}
                      title={lookTitle(l)}
                      blurb={lookBlurb(l)}
                      preview={lutPreviews[l.name]}
                      active={lutName === l.name}
                      aspect={aspect}
                      onClick={() => onChooseLut(lutName === l.name ? "" : l.name)}
                      onRemove={() => onRemoveLut(l.name)}
                      removeTitle={t("develop.presets.delete", { title: lookTitle(l) })}
                      onGroup={() => onGroupLut(l.name, l.family, l.grouped)}
                      groupTitle={
                        l.grouped
                          ? t("develop.luts.moveToGroup", { title: lookTitle(l) })
                          : t("develop.luts.fileUnderGroup", { title: lookTitle(l) })
                      }
                      onUngroup={l.grouped ? () => onUngroupLut(l.name) : undefined}
                      ungroupTitle={t("develop.luts.takeOutOfGroup", { title: lookTitle(l), family: f.label })}
                    />
                  ))}
                </div>
              )}
            </div>
          );
        })}
        {lutsShown.length === 0 && (
          <p className="px-1 py-2 text-caption leading-relaxed text-faint">
            {q ? t("develop.luts.emptyNoMatch") : t("develop.luts.emptyNone")}
          </p>
        )}
      </div>

      <div className="flex gap-1.5 border-t border-line p-2">
        <button onClick={onImportLut} className="btn-solid flex-1 justify-center text-caption">
          {t("develop.luts.importCube")}
        </button>
      </div>
    </>
  );

  return (
    <aside
      ref={panelRef}
      style={{ width: Math.max(MIN_W, Math.min(width, maxWidth ?? width)) }}
      className="relative flex shrink-0 flex-col border-r border-line bg-panel"
    >
      <div className="flex items-center gap-2 border-b border-line px-3 py-2">
        <div className="seg-group flex-1" role="tablist">
          {(["presets", "luts"] as const).map((id) => (
            <button
              key={id}
              role="tab"
              aria-selected={tab === id}
              onClick={() => {
                setTab(id);
                setQuery("");
              }}
              className={`flex-1 px-2 py-1 text-caption ${tab === id ? "seg-on" : "text-muted hover:bg-hover"}`}
            >
              {id === "presets" ? t("develop.presets.tabPresets") : t("develop.presets.tabLuts")}
            </button>
          ))}
        </div>
        <button onClick={onClose} className="icon-chip" title={t("develop.presets.hidePanel")}>
          <Icon name="close" size={14} />
        </button>
      </div>

      {tab === "presets" ? presetsTab : lutsTab}

      {/* Resize handle, over the border so the whole edge is grabbable. */}
      <div
        onPointerDown={onResizeDown}
        onPointerMove={onResizeMove}
        onPointerUp={endResize}
        onPointerCancel={endResize}
        onDoubleClick={() => setWidth(DEFAULT_W)}
        role="separator"
        aria-orientation="vertical"
        aria-label={t("develop.presets.resizeLabel")}
        title={t("develop.presets.resizeTitle")}
        className={`absolute inset-y-0 -right-1 z-20 w-2 cursor-col-resize ${
          dragging ? "bg-accent/40" : "hover:bg-accent/25"
        }`}
      />
    </aside>
  );
}
