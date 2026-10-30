import { useCallback, useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { api, fileUrl } from "../lib/api";
import { Icon } from "../lib/icons";
import { useContentBox } from "../lib/useContentBox";
import { useEscape } from "../lib/useEscape";
import { PAGE, developShown, useStore, viewFilter } from "../store";
import { normaliseFilter, type Photo, type PhotoFilter } from "../types";
import { t } from "../i18n";
import { ColorLabelDot, FlagBadge, RatingBadge } from "./PhotoBadges";

const GAP = 6;

/**
 * A person's, a place's or a trip's photos, shown where they were asked for.
 *
 * People and Places used to answer a click by setting a Browse filter and
 * switching to Browse: you were moved to another workspace, and getting back
 * meant finding your place again. This is the same query drawn in place, with
 * the Lightbox opened on exactly this set — Browse stays one deliberate click
 * away for the jobs that belong there (export, collections, bulk tagging).
 */
export default function PhotoSet({
  title,
  subtitle,
  backLabel,
  filter,
  onBack,
}: {
  title: string;
  subtitle?: string;
  /** Where Back returns to, as the button names it ("People"). */
  backLabel: string;
  filter: PhotoFilter;
  onBack: () => void;
}) {
  const openLightbox = useStore((s) => s.openLightbox);
  const setFilter = useStore((s) => s.setFilter);
  const setMode = useStore((s) => s.setMode);
  const cellSize = useStore((s) => s.cellSize);
  // Rating in the Lightbox or editing in Develop changes these photos; the set
  // here is its own copy, so it is read again when either closes.
  const overlayOpen = useStore((s) => s.lightboxIndex !== null || developShown(s));

  const [photos, setPhotos] = useState<Photo[] | null>(null);
  const load = useCallback(
    () =>
      api
        .getPhotos(viewFilter(normaliseFilter(filter)), PAGE, 0)
        .then(setPhotos)
        .catch((e) => {
          console.error("photo set failed to load", e);
          setPhotos([]);
        }),
    [filter],
  );
  useEffect(() => {
    setPhotos(null);
    void load();
  }, [load]);
  const wasOpen = useRef(overlayOpen);
  useEffect(() => {
    if (wasOpen.current && !overlayOpen) void load();
    wasOpen.current = overlayOpen;
  }, [overlayOpen, load]);

  // Escape backs out, as it leaves any other view — but not while the
  // Lightbox or Develop is on top, which own the key then.
  useEscape(!overlayOpen, onBack);

  const openInBrowse = () => {
    setFilter(filter);
    setMode("browse");
  };

  const scrollRef = useRef<HTMLDivElement>(null);
  const box = useContentBox(scrollRef, [photos === null]);
  const cols = Math.max(1, Math.floor((box.width + GAP) / (cellSize + GAP)));
  const cellW = box.width > 0 ? (box.width - GAP * (cols - 1)) / cols : cellSize;
  const count = photos?.length ?? 0;
  const rows = useVirtualizer({
    count: Math.ceil(count / cols),
    getScrollElement: () => scrollRef.current,
    estimateSize: () => cellW,
    gap: GAP,
    overscan: 4,
  });
  useEffect(() => {
    rows.measure();
  }, [cellW, cols, rows]);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div
        className="flex h-[52px] shrink-0 items-center gap-3 border-b border-line bg-panel px-4"
        data-tauri-drag-region="deep"
      >
        <button onClick={onBack} className="btn-ghost -ml-2 shrink-0" title={t("library.photoSet.backTo", { place: backLabel })}>
          <Icon name="chevron" size={15} className="rotate-90" />
          {backLabel}
        </button>
        <div className="min-w-0">
          <div className="truncate text-body font-semibold text-ink">{title}</div>
          <div className="truncate text-caption text-faint">
            {photos === null
              ? t("common.loading")
              : subtitle
                ? t("library.photoSet.countWithSubtitle", { count, subtitle })
                : t("common.photos", { count })}
          </div>
        </div>
        <button
          onClick={openInBrowse}
          className="btn-solid ml-auto shrink-0"
          title={t("library.photoSet.openInBrowseTooltip")}
        >
          {t("library.photoSet.openInBrowse")}
        </button>
      </div>

      <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden p-2">
        {photos !== null && count === 0 ? (
          <div className="p-8 text-center text-caption text-faint">{t("library.photoSet.empty")}</div>
        ) : (
          <div style={{ height: rows.getTotalSize(), position: "relative", width: "100%" }}>
            {rows.getVirtualItems().map((row) => {
              const start = row.index * cols;
              return (
                <div
                  key={row.key}
                  className="absolute left-0 top-0 flex w-full"
                  style={{ transform: `translateY(${row.start}px)`, gap: GAP }}
                >
                  {(photos ?? []).slice(start, start + cols).map((p, ci) => (
                    <button
                      key={p.id}
                      onClick={() => photos && openLightbox(start + ci, photos)}
                      title={p.filename}
                      style={{ width: cellW, height: cellW }}
                      className="group relative shrink-0 overflow-hidden rounded-sm bg-raised ring-offset-1 ring-offset-base hover:ring-1 hover:ring-line"
                    >
                      {p.thumbPath ? (
                        <img
                          src={fileUrl(p.thumbPath)}
                          alt={p.filename}
                          loading="lazy"
                          draggable={false}
                          className="h-full w-full object-cover"
                        />
                      ) : (
                        <div className="grid h-full place-items-center text-faint">?</div>
                      )}
                      <div className="pointer-events-none absolute left-1 top-1 flex flex-col items-start gap-0.5">
                        <FlagBadge flag={p.flag} />
                        <RatingBadge rating={p.rating} />
                        <ColorLabelDot label={p.colorLabel} className="mt-0.5" />
                      </div>
                    </button>
                  ))}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
