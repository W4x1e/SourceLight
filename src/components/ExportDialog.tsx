import { useEffect, useRef, useState } from "react";
import { open as openDirectoryDialog, save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import type { BitDepth, RenderingIntent, SharpenLevel, TextWatermark, Watermark } from "../bindings";
import { api } from "../lib/api";
import { confirm, prompt, toast } from "../lib/dialogs";
import { exportPlan, singleSummary } from "../lib/export";
import { FILE_MANAGER } from "../lib/platform";
import { runBatch, sendToPhone } from "../lib/batchExport";
import { SIZES, exportOptions, isPreset, renamePattern, type ExportSettings } from "../lib/exportSettings";
import { roomToExport } from "../lib/exportSpace";
import { Icon } from "../lib/icons";
import { useModal } from "../lib/useEscape";
import { useStore } from "../store";
import type { ColorSpace, ExportFormat } from "../types";
import NumberSlider from "./NumberSlider";
import { getLocale, t, type Key } from "../i18n";
import { usePresence } from "../lib/usePresence";

/** The nine watermark positions, with the wording for each. */
const ANCHORS: readonly (readonly [string, Key])[] = [
  ["top-left", "transfer.export.anchorTopLeft"],
  ["top-center", "transfer.export.anchorTopCenter"],
  ["top-right", "transfer.export.anchorTopRight"],
  ["middle-left", "transfer.export.anchorMiddleLeft"],
  ["center", "transfer.export.anchorCenter"],
  ["middle-right", "transfer.export.anchorMiddleRight"],
  ["bottom-left", "transfer.export.anchorBottomLeft"],
  ["bottom-center", "transfer.export.anchorBottomCenter"],
  ["bottom-right", "transfer.export.anchorBottomRight"],
] as const;

/**
 * The export dialog: format, size, quality, colour space and rendering intent,
 * output sharpening, metadata, watermark, and single or batch export.
 *
 * Moved out of `Develop.tsx` with its JSX unchanged. Its settings
 * live in the store, remembered across exports and restarts and the
 * same in Develop's dialog and the library's, with named presets.
 */
export default function ExportDialog({
  open,
  onClose,
  filename = "",
  onExportingChange,
  batch = false,
}: {
  open: boolean;
  onClose: () => void;
  /** The open photo's filename, for the save dialog's default name. */
  filename?: string;
  /** Outside Develop there is no open photo, only a selection to export. */
  batch?: boolean;
  /** Told when an export starts and ends, for the toolbar's Export button. */
  onExportingChange?: (exporting: boolean) => void;
}) {
  const selectedCount = useStore((s) => s.selected.size);
  // The open photo's framed size, for what a single export will write.
  // Two numbers, not an object: a selector must not build a new value on
  // every call.
  const framedW = useStore((s) => s.develop?.width ?? 0);
  const framedH = useStore((s) => s.develop?.height ?? 0);
  const framed = framedW > 0 && framedH > 0 ? { width: framedW, height: framedH } : null;
  // Send to phone shows only with a paired phone (docs/MOBILE.md).
  const [phonePaired, setPhonePaired] = useState(false);
  useEffect(() => {
    if (!open) return;
    api
      .phoneLinkStatus()
      .then((s) => setPhonePaired(s.available && s.phones.length > 0))
      .catch(() => setPhonePaired(false));
  }, [open]);
  // Owns its keys: Escape closes this, not Develop under it, and the
  // arrows and verdict keys don't reach the photo behind.
  const card = useRef<HTMLDivElement>(null);
  useModal(open, card, onClose);
  // Every setting is the store's; see lib/exportSettings for the
  // defaults and why each is what it is.
  const settings = useStore((s) => s.exportSettings);
  const patch = useStore((s) => s.setExportSettings);
  const {
    format: exportFormat,
    space: exportSpace,
    intent: exportIntent,
    quality: exportQuality,
    depth: exportDepth,
    size: exportSize,
    sharpen: exportSharpen,
    strip: exportStrip,
    rename: exportRename,
    numbered,
    subfolder: exportSubfolder,
    wmMode,
    wmImage,
    wmText,
  } = settings;
  const setExportFormat = (format: ExportFormat) => patch({ format });
  const setExportSpace = (space: ColorSpace) => patch({ space });
  const setExportIntent = (intent: RenderingIntent) => patch({ intent });
  const setExportQuality = (quality: number) => patch({ quality });
  const setExportDepth = (depth: BitDepth) => patch({ depth });
  const setExportSize = (size: number | null) => patch({ size });
  const setExportSharpen = (sharpen: SharpenLevel) => patch({ sharpen });
  const setExportStrip = (strip: boolean) => patch({ strip });
  const setExportRename = (rename: string) => patch({ rename });
  const setNumbered = (n: boolean) => patch({ numbered: n });
  const setExportSubfolder = (subfolder: string) => patch({ subfolder });
  const setWmMode = (mode: ExportSettings["wmMode"]) => patch({ wmMode: mode });
  const setWmImage = (f: (w: Watermark) => Watermark) =>
    patch({ wmImage: f(useStore.getState().exportSettings.wmImage) });
  const setWmText = (f: (w: TextWatermark) => TextWatermark) =>
    patch({ wmText: f(useStore.getState().exportSettings.wmText) });
  const [exporting, setExporting] = useState(false);
  const [exported, setExported] = useState<string | null>(null);
  // Camera, lens, exposure, date, creator and copyright go into every export
  // unless stripped. Where, who, and the AI's words are each a remembered
  // choice, the same for sidecars.
  const exportChoices = useStore((s) => s.exportMetadata);
  const setExportChoices = useStore((s) => s.setExportMetadata);
  // A long edge other than the buttons', as typed: kept apart from the
  // setting so typing 1500 doesn't pass through a 1 px export on the way.
  const [edgeDraft, setEdgeDraft] = useState(() =>
    settings.size && !(SIZES as readonly number[]).includes(settings.size) ? String(settings.size) : "",
  );
  // The presets, and which one these settings are, if any.
  const presets = useStore((s) => s.exportPresets);
  const current = presets.find((p) => isPreset(p, settings, exportChoices))?.name ?? "";
  const applyPreset = (name: string) => {
    useStore.getState().applyExportPreset(name);
    const size = useStore.getState().exportSettings.size;
    setEdgeDraft(size && !(SIZES as readonly number[]).includes(size) ? String(size) : "");
  };
  const savePreset = async () => {
    const name = await prompt({
      title: t("transfer.preset.saveTitle"),
      message: t("transfer.preset.saveMessage"),
      placeholder: t("transfer.preset.placeholder"),
      defaultValue: current,
    });
    if (!name) return;
    useStore.getState().saveExportPreset(name);
    toast(t("transfer.preset.saved", { name }), "success");
  };
  const deletePreset = async () => {
    if (!current) return;
    const yes = await confirm({
      title: t("transfer.preset.deleteTitle", { name: current }),
      message: t("transfer.preset.deleteMessage"),
      confirmLabel: t("transfer.preset.delete"),
      danger: true,
    });
    if (yes) useStore.getState().deleteExportPreset(current);
  };
  // Naming only applies to a batch: hidden for a single photo.
  const batchy = batch || selectedCount > 1;
  const pickWatermark = async () => {
    const picked = await openDirectoryDialog({
      multiple: false,
      filters: [{ name: t("transfer.export.filterImage"), extensions: ["png", "jpg", "jpeg", "webp"] }],
    });
    if (typeof picked === "string") setWmImage((w) => ({ ...w, imagePath: picked }));
  };
  const pickFont = async () => {
    const picked = await openDirectoryDialog({
      multiple: false,
      filters: [{ name: t("transfer.export.filterFont"), extensions: ["ttf", "otf"] }],
    });
    if (typeof picked === "string") setWmText((w) => ({ ...w, fontPath: picked }));
  };

  const runExport = async () => {
    const stem = filename.replace(/\.[^.]+$/, "");
    const ext = exportFormat === "jpeg" ? "jpg" : exportFormat === "tiff" ? "tif" : exportFormat;
    // A JPEG or TIFF exported as its own format would propose its own name,
    // one "Replace?" from losing the original; the backend refuses
    // that too, but the suggestion shouldn't lead there.
    const sourceExt = filename.split(".").pop()?.toLowerCase() ?? "";
    const same = sourceExt === ext || (ext === "jpg" && sourceExt === "jpeg") || (ext === "tif" && sourceExt === "tiff");
    const path = await saveFileDialog({
      defaultPath: same ? `${stem}-edit.${ext}` : `${stem}.${ext}`,
      filters: [{ name: exportFormat.toUpperCase(), extensions: [ext] }],
    });
    if (!path) return;

    setExporting(true);
    setExported(null);
    try {
      const bytes = await api.exportDevelop(path, exportOptions(settings, exportChoices));
      // Close on success and say so in a toast. The result used to live only
      // in this dialog's own state, so closing the dialog — the obvious next
      // move — destroyed the only evidence the export had happened.
      // Either separator: Windows' save dialog answers with backslashes, and
      // split on "/" alone the toast named the whole path.
      const name = path.split(/[\\/]/).pop() ?? filename;
      onClose();
      // With the way to it: the save dialog chose the folder a moment ago,
      // and the toast was the only trace of where the file went.
      toast(singleSummary(name, bytes), "success", {
        label: t("common.showInFileManager", { fileManager: FILE_MANAGER }),
        run: () => void api.showFolder(path).catch((e) => toast(String(e), "error")),
      });
    } catch (e) {
      setExported(t("transfer.export.failed", { error: String(e) }));
    } finally {
      setExporting(false);
    }
  };

  /**
   * Exports every selected photo through its own saved edits.
   *
   * A folder rather than a filename: the backend names each file after its
   * source and refuses to overwrite anything already there, so pointing this
   * at a folder that already holds exports adds to it instead of replacing it.
   */
  const runBatchExport = async () => {
    // Said before a folder is asked for, not after.
    if (useStore.getState().exportJob?.running) {
      toast(t("transfer.queue.busy"), "error");
      return;
    }
    const directory = await openDirectoryDialog({
      directory: true,
      title: t("transfer.export.pickFolderTitle"),
    });
    if (typeof directory !== "string") return;

    const st = useStore.getState();
    const all = st.searchResults ?? st.photos;
    // In the order they are shown — a collection's own order — so
    // numbered files come out in it.
    const view = st.searchResults ?? st.photos;
    const shown = view.filter((p) => st.selected.has(p.id)).map((p) => p.id);
    const ids = shown.length === st.selected.size ? shown : [...st.selected];
    const byId = new Map(all.map((p) => [p.id, p]));
    const files = ids.map((id) => byId.get(id)?.filename ?? t("transfer.export.photoFallback", { id: String(id) }));
    const options = exportOptions(settings, exportChoices);
    // Asked before anything is written, not discovered at photo 300 of 400.
    if (!(await roomToExport(ids, directory, options))) return;

    // Hand off to the global queue panel, which keeps showing progress after
    // this dialog closes.
    onClose();
    setExporting(true);
    try {
      await runBatch({
        ids,
        files,
        directory,
        options,
        rename: renamePattern(settings),
        subfolder: exportSubfolder.trim() || null,
      });
    } finally {
      setExporting(false);
    }
  };


  /** The selection, exported with these settings, for the paired phone. */
  const runSendToPhone = async () => {
    if (useStore.getState().exportJob?.running) {
      toast(t("transfer.queue.busy"), "error");
      return;
    }
    const st = useStore.getState();
    const view = st.searchResults ?? st.photos;
    const shown = view.filter((p) => st.selected.has(p.id)).map((p) => p.id);
    const ids = shown.length === st.selected.size ? shown : [...st.selected];
    const byId = new Map(view.map((p) => [p.id, p]));
    const files = ids.map((id) => byId.get(id)?.filename ?? t("transfer.export.photoFallback", { id: String(id) }));
    onClose();
    setExporting(true);
    try {
      await sendToPhone({ ids, files, options: exportOptions(settings, exportChoices), rename: renamePattern(settings) });
    } finally {
      setExporting(false);
    }
  };

  useEffect(() => onExportingChange?.(exporting), [exporting, onExportingChange]);

  // Mounted a moment past closing, for the exit (see usePresence).
  const presence = usePresence(open);
  if (!presence.mounted) return null;

  return (
    <div
      data-closing={presence.closing || undefined}
      className="anim-scrim fixed inset-0 z-50 grid place-items-center bg-scrim p-6"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      {/* Bounded by the window, with the settings scrolling between a fixed
          header and footer: at the minimum window (940×620) the dialog is
          taller than the window, and its Export button used to sit below the
          bottom edge with nothing to scroll it into view. */}
      <div
        ref={card}
        role="dialog"
        aria-modal="true"
        aria-labelledby="export-title"
        className="anim-card flex max-h-[calc(100vh-3rem)] w-[420px] max-w-full flex-col overflow-hidden rounded-lg border border-line bg-panel shadow-2xl"
      >
        <div className="flex shrink-0 items-center border-b border-line px-4 py-3">
          <span id="export-title" className="font-display flex-1 text-display">{t("transfer.export.title")}</span>
          <button onClick={() => onClose()} className="icon-chip" title={t("common.close")}>
            <Icon name="close" size={15} />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
          {/* Named settings: "Web proofs", "Print". */}
          <span className="section-label px-0">{t("transfer.preset.label")}</span>
          <div className="mb-3 mt-1 flex items-center gap-1">
            <select
              className="input min-w-0 flex-1"
              value={current}
              onChange={(e) => e.target.value && applyPreset(e.target.value)}
              aria-label={t("transfer.preset.label")}
            >
              <option value="">{presets.length ? t("transfer.preset.custom") : t("transfer.preset.none")}</option>
              {presets.map((p) => (
                <option key={p.name} value={p.name}>
                  {p.name}
                </option>
              ))}
            </select>
            <button onClick={() => void savePreset()} className="btn-solid shrink-0 text-caption">
              {t("transfer.preset.save")}
            </button>
            {current && (
              <button
                onClick={() => void deletePreset()}
                className="icon-chip shrink-0"
                title={t("transfer.preset.delete")}
                aria-label={t("transfer.preset.delete")}
              >
                <Icon name="close" size={14} />
              </button>
            )}
          </div>
          <span className="section-label px-0">{t("transfer.export.format")}</span>
          <div className="mb-3 mt-1 seg-group">
            {(["jpeg", "png", "tiff", "webp"] as const).map((f) => (
              <button aria-pressed={exportFormat === f}
                key={f}
                onClick={() => setExportFormat(f)}
                className={`flex-1 py-1.5 text-caption uppercase ${
                  exportFormat === f ? "seg-on" : "text-muted hover:bg-hover"
                }`}
              >
                {f}
              </button>
            ))}
          </div>
          <span className="section-label px-0">{t("transfer.export.longEdge")}</span>
          <div className="mb-3 mt-1 flex gap-1">
            {/* One choice, so one segmented control like Format's above it,
                with the typed size beside it. They were loose buttons. */}
            <div className="seg-group min-w-0 flex-1">
              {([0, ...SIZES] as const).map((val) => (
                <button aria-pressed={(exportSize ?? 0) === val}
                  key={val}
                  onClick={() => {
                    setExportSize(val || null);
                    setEdgeDraft("");
                  }}
                  className={`flex-1 py-1.5 text-caption ${
                    (exportSize ?? 0) === val ? "seg-on" : "text-muted hover:bg-hover"
                  }`}
                >
                  {val === 0 ? t("transfer.export.sizeFull") : String(val)}
                </button>
              ))}
            </div>
            {/* Any other long edge, such as 2500 for a blog or 1350 for a
                feed: only the four buttons existed. */}
            <input
              type="number"
              inputMode="numeric"
              min={16}
              max={65535}
              // Narrow padding: at the input's own 10px a side, German's
              // "Andere" was cut to "Ander".
              className={`input w-[72px] shrink-0 px-1 text-center ${edgeDraft ? "border-accent" : ""}`}
              placeholder={t("transfer.export.sizeOther")}
              aria-label={t("transfer.export.sizeOtherLabel")}
              title={t("transfer.export.sizeOtherLabel")}
              value={edgeDraft}
              onChange={(e) => {
                setEdgeDraft(e.target.value);
                const px = Math.round(Number(e.target.value));
                if (px >= 16 && px <= 65535) setExportSize(px);
              }}
            />
          </div>
          {/* WebP is written lossless (the encoder has no lossy mode), so a
              quality slider for it changed nothing. */}
          {exportFormat === "webp" && (
            <p className="mb-2 mt-1 text-label text-faint">{t("transfer.export.webpLossless")}</p>
          )}
          {exportFormat === "jpeg" && (
            // Spaced like every other setting: it sat flush against the
            // Colour space heading under it.
            <div className="mb-3">
              <NumberSlider
                label={t("transfer.export.quality")}
                value={exportQuality}
                min={40}
                max={100}
                step={1}
                defaultValue={92}
                precision={0}
                onChange={setExportQuality}
                onCommit={() => {}}
              />
            </div>
          )}
          {(exportFormat === "png" || exportFormat === "tiff") && (
            <>
              <span className="section-label px-0">{t("transfer.export.bitDepth")}</span>
              <div className="mb-1 mt-1 seg-group">
                {(
                  [
                    ["eight", "8-bit"],
                    ["sixteen", "16-bit"],
                  ] as const
                ).map(([id, label]) => (
                  <button aria-pressed={exportDepth === id}
                    key={id}
                    onClick={() => setExportDepth(id)}
                    className={`flex-1 py-1.5 text-caption ${
                      exportDepth === id ? "seg-on" : "text-muted hover:bg-hover"
                    }`}
                  >
                    {label}
                  </button>
                ))}
              </div>
              <p className="mb-3 text-label text-faint">
                {exportDepth === "eight" ? t("transfer.export.depth8Hint") : t("transfer.export.depth16Hint")}
              </p>
            </>
          )}
          <span className="section-label px-0">{t("transfer.export.colourSpace")}</span>
          <div className="mb-1 mt-1 seg-group">
            {(
              [
                ["srgb", "sRGB"],
                ["display-p3", "Display P3"],
                ["adobe-rgb", "Adobe RGB"],
              ] as const
            ).map(([id, label]) => (
              <button aria-pressed={exportSpace === id}
                key={id}
                onClick={() => setExportSpace(id)}
                className={`flex-1 py-1.5 text-caption ${
                  exportSpace === id ? "seg-on" : "text-muted hover:bg-hover"
                }`}
              >
                {label}
              </button>
            ))}
          </div>
          <p className="mb-3 text-label text-faint">
            {exportSpace === "srgb"
              ? t("transfer.export.spaceSrgbHint")
              : exportSpace === "display-p3"
                ? t("transfer.export.spaceP3Hint")
                : t("transfer.export.spaceAdobeHint")}
          </p>

          <span className="section-label px-0">{t("transfer.export.intent")}</span>
          <div className="mb-1 mt-1 seg-group">
            {(
              [
                ["relative-colorimetric", "transfer.export.intentRelative"],
                ["perceptual", "transfer.export.intentPerceptual"],
              ] as const
            ).map(([id, label]) => (
              <button aria-pressed={exportIntent === id}
                key={id}
                onClick={() => setExportIntent(id)}
                className={`flex-1 py-1.5 text-caption ${
                  exportIntent === id ? "seg-on" : "text-muted hover:bg-hover"
                }`}
              >
                {t(label)}
              </button>
            ))}
          </div>
          <p className="mb-3 text-label text-faint">
            {exportIntent === "relative-colorimetric"
              ? t("transfer.export.intentRelativeHint")
              : t("transfer.export.intentPerceptualHint")}
          </p>

          <span className="section-label px-0">{t("transfer.export.sharpening")}</span>
          <div className="mb-3 mt-1 seg-group">
            {(
              [
                ["off", "common.off"],
                ["low", "transfer.export.sharpenLow"],
                ["standard", "transfer.export.sharpenStandard"],
                ["high", "transfer.export.sharpenHigh"],
              ] as const
            ).map(([id, label]) => (
              <button aria-pressed={exportSharpen === id}
                key={id}
                onClick={() => setExportSharpen(id)}
                className={`flex-1 py-1.5 text-caption ${
                  exportSharpen === id ? "seg-on" : "text-muted hover:bg-hover"
                }`}
              >
                {t(label)}
              </button>
            ))}
          </div>

          <span className="section-label px-0">{t("transfer.export.metadata")}</span>
          <div className="mb-1 mt-1 seg-group">
            {(
              [
                [false, "transfer.export.metaInclude"],
                [true, "transfer.export.metaStrip"],
              ] as const
            ).map(([strip, label]) => (
              <button aria-pressed={exportStrip === strip}
                key={label}
                onClick={() => setExportStrip(strip)}
                className={`flex-1 py-1.5 text-caption ${
                  exportStrip === strip ? "seg-on" : "text-muted hover:bg-hover"
                }`}
              >
                {t(label)}
              </button>
            ))}
          </div>
          <p className="mb-1 text-label text-faint">
            {exportStrip ? t("transfer.export.metaStripHint") : t("transfer.export.metaIncludeHint")}
          </p>
          {!exportStrip && (
            <div className="mb-1 grid grid-cols-2 gap-x-3">
              {(
                [
                  ["location", "transfer.export.metaLocation"],
                  ["names", "transfer.export.metaNames"],
                  ["aiCaption", "transfer.export.metaAiCaption"],
                  ["aiKeywords", "transfer.export.metaAiKeywords"],
                ] as const
              ).map(([key, label]) => (
                <label key={key} className="flex items-center gap-2 py-0.5 text-caption text-muted">
                  <input
                    type="checkbox"
                    checked={exportChoices[key]}
                    onChange={(e) => setExportChoices({ [key]: e.target.checked })}
                  />
                  {t(label)}
                </label>
              ))}
            </div>
          )}
          {!exportStrip && (
            <p className="mb-3 text-label text-faint">
              {t("transfer.export.metaChoicesHint")}
              {/* Keywords are the catalogue's English keys, whatever the
                  interface shows: said, not discovered by a client. */}
              {!getLocale().startsWith("en") && ` ${t("transfer.export.keywordsEnglish")}`}
            </p>
          )}

          <span className="section-label px-0">{t("transfer.export.watermark")}</span>
          <div className="mb-3 mt-1 seg-group">
            {(
              [
                ["none", "common.none"],
                ["image", "transfer.export.wmImage"],
                ["text", "transfer.export.wmText"],
              ] as const
            ).map(([m, label]) => (
              <button aria-pressed={wmMode === m}
                key={m}
                onClick={() => setWmMode(m)}
                className={`flex-1 py-1.5 text-caption ${
                  wmMode === m ? "seg-on" : "text-muted hover:bg-hover"
                }`}
              >
                {t(label)}
              </button>
            ))}
          </div>
          {wmMode === "image" && (
            <div className="mb-3">
              <div className="mb-2 flex items-center gap-2">
                <button onClick={() => void pickWatermark()} className="btn-solid text-caption">
                  {t("transfer.export.chooseLogo")}
                </button>
                <span
                  className="min-w-0 flex-1 truncate text-caption text-muted"
                  title={wmImage.imagePath}
                >
                  {wmImage.imagePath
                    ? wmImage.imagePath.split(/[\\/]/).pop()
                    : t("transfer.export.noLogo")}
                </span>
              </div>
              <div className="flex gap-4">
                <div>
                  <span className="section-label px-0">{t("transfer.export.position")}</span>
                  <div className="mt-1 grid w-[84px] grid-cols-3 gap-1">
                    {ANCHORS.map(([a, name]) => (
                      <button
                        key={a}
                        onClick={() => setWmImage((w) => ({ ...w, anchor: a as Watermark["anchor"] }))}
                        title={t(name)}
                        aria-label={t(name)}
                        aria-pressed={wmImage.anchor === a}
                        className={`aspect-square rounded-sm border ${
                          wmImage.anchor === a
                            ? "border-transparent bg-accent"
                            : "border-line hover:bg-hover"
                        }`}
                      />
                    ))}
                  </div>
                </div>
                <div className="min-w-0 flex-1">
                  <NumberSlider
                    label={t("transfer.export.opacity")}
                    value={wmImage.opacity ?? 0.6}
                    min={0}
                    max={1}
                    step={0.01}
                    defaultValue={0.6}
                    onChange={(v) => setWmImage((w) => ({ ...w, opacity: v }))}
                    onCommit={() => {}}
                  />
                  <NumberSlider
                    label={t("transfer.export.size")}
                    value={wmImage.scale ?? 0.2}
                    min={0.05}
                    max={0.6}
                    step={0.01}
                    defaultValue={0.2}
                    onChange={(v) => setWmImage((w) => ({ ...w, scale: v }))}
                    onCommit={() => {}}
                  />
                </div>
              </div>
            </div>
          )}

          {wmMode === "text" && (
            <div className="mb-3">
              <input
                className="input mb-2"
                placeholder={t("transfer.export.textPlaceholder")}
                value={wmText.text}
                onChange={(e) => setWmText((w) => ({ ...w, text: e.target.value }))}
              />
              <div className="mb-2 flex items-center gap-2">
                <button onClick={() => void pickFont()} className="btn-solid text-caption">
                  {t("transfer.export.chooseFont")}
                </button>
                <span
                  className="min-w-0 flex-1 truncate text-caption text-muted"
                  title={wmText.fontPath}
                >
                  {wmText.fontPath
                    ? wmText.fontPath.split(/[\\/]/).pop()
                    : t("transfer.export.noFont")}
                </span>
              </div>
              <div className="flex gap-4">
                <div>
                  <span className="section-label px-0">{t("transfer.export.position")}</span>
                  <div className="mt-1 grid w-[84px] grid-cols-3 gap-1">
                    {ANCHORS.map(([a, name]) => (
                      <button
                        key={a}
                        onClick={() => setWmText((w) => ({ ...w, anchor: a as TextWatermark["anchor"] }))}
                        title={t(name)}
                        aria-label={t(name)}
                        aria-pressed={wmText.anchor === a}
                        className={`aspect-square rounded-sm border ${
                          wmText.anchor === a
                            ? "border-transparent bg-accent"
                            : "border-line hover:bg-hover"
                        }`}
                      />
                    ))}
                  </div>
                </div>
                <div className="min-w-0 flex-1">
                  <NumberSlider
                    label={t("transfer.export.opacity")}
                    value={wmText.opacity ?? 0.7}
                    min={0}
                    max={1}
                    step={0.01}
                    defaultValue={0.7}
                    onChange={(v) => setWmText((w) => ({ ...w, opacity: v }))}
                    onCommit={() => {}}
                  />
                  <NumberSlider
                    label={t("transfer.export.size")}
                    value={wmText.size ?? 0.04}
                    min={0.01}
                    max={0.2}
                    step={0.005}
                    defaultValue={0.04}
                    onChange={(v) => setWmText((w) => ({ ...w, size: v }))}
                    onCommit={() => {}}
                  />
                </div>
              </div>
            </div>
          )}

          {batchy && (
            <>
          <span className="section-label px-0">{t("transfer.export.naming")}</span>
          <div className="mb-1 mt-1 grid grid-cols-2 gap-2">
            <input
              className="input"
              // i18n-ok: a pattern, shown as typed
              placeholder="{name}-edited"
              value={exportRename}
              onChange={(e) => setExportRename(e.target.value)}
              title={t("transfer.export.renameHint", { name: "{name}" })}
            />
            <input
              className="input"
              placeholder={t("transfer.export.subfolderPlaceholder")}
              value={exportSubfolder}
              onChange={(e) => setExportSubfolder(e.target.value)}
              title={t("transfer.export.subfolderHint")}
            />
          </div>
          <label className="mb-1 flex items-center gap-2 text-caption text-muted" title={t("transfer.export.numberedHint")}>
            <input type="checkbox" checked={numbered} onChange={(e) => setNumbered(e.target.checked)} />
            {t("transfer.export.numbered")}
          </label>
          {(exportRename.trim() || numbered) && (
            <p className="mb-2 text-label text-faint">
              → {numbered ? "001-" : ""}
              {(exportRename.trim() || "{name}").replace("{name}", "IMG_2043")}
            </p>
          )}
            </>
          )}
          {/* Only failures land here now: a success closes the dialog and
              says so in a toast, which survives the close. */}
          {exported && <p className="mt-1 text-caption text-danger-ink">{exported}</p>}
          {exporting && (
            <div className="mt-2 h-1 overflow-hidden rounded-full bg-base">
              {/* Indeterminate: a single-photo render reports no progress,
                  so a percentage would be a lie. This says "working". */}
              <div className="export-pulse h-full w-1/3 rounded-full bg-accent" />
            </div>
          )}
        </div>
        {/* What will be written, said before it is: the dialog's settings
            never added up to "a 2048 × 1365 JPEG" anywhere. */}
        <p className="shrink-0 truncate border-t border-line px-4 pt-2.5 text-label tabular-nums text-faint" aria-live="polite">
          {exportPlan(settings, batch ? null : framed, batch ? selectedCount : 1)}
        </p>
        <div className="flex shrink-0 items-center gap-2 px-4 pb-3 pt-2">
          {!batch && selectedCount > 1 && (
            <button
              onClick={() => void runBatchExport()}
              disabled={exporting}
              className="btn-solid text-caption"
            >
              {exporting ? t("transfer.export.exporting") : t("transfer.export.exportSelected", { count: selectedCount })}
            </button>
          )}
          {phonePaired && selectedCount > 0 && (batch || selectedCount > 1) && (
            <button
              onClick={() => void runSendToPhone()}
              disabled={exporting}
              className="btn-solid text-caption"
              title={t("transfer.phone.sendTitle")}
            >
              {t("transfer.phone.send")}
            </button>
          )}
          <div className="flex-1" />
          <button onClick={() => onClose()} className="btn-ghost text-caption">
            {t("common.close")}
          </button>
          {batch ? (
            <button
              onClick={() => void runBatchExport()}
              disabled={exporting || selectedCount === 0}
              className="btn-accent text-caption"
            >
              {exporting ? t("transfer.export.exporting") : t("transfer.export.exportPhotos", { count: selectedCount })}
            </button>
          ) : (
            <button
              onClick={() => void runExport()}
              disabled={exporting}
              className="btn-accent text-caption"
            >
              {exporting ? t("transfer.export.exporting") : t("transfer.export.exportPhoto")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
