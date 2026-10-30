import { t } from "../i18n";
import { Icon } from "../lib/icons";

/**
 * Shared quality/face/eye indicators for the cull grid and the lightbox, so the
 * two surfaces read the same signals the same way.
 *
 * Focus is shown as a *relative* 5-band meter (the sharpness metric only means
 * anything against a peer set), faces as a count, eyes as an open/closed cue.
 */

/** Colour a focus band by how good it is: soft reads warning, sharp reads
 *  positive, the middle stays neutral — so a soft frame stands out. */
function bandFill(level: number): string {
  if (level <= 1) return "var(--color-warning)";
  if (level === 2) return "var(--color-muted)";
  return "var(--color-positive)";
}

/** A 5-segment focus meter filled to `level` (0…4). `sm` for grid tiles, `md`
 *  for the loupe / lightbox panel. On tiles it sits on the photo, so `onDark`
 *  makes the empty segments legible over any frame. */
export function FocusMeter({
  level,
  size = "md",
  onDark = false,
}: {
  level: number;
  size?: "sm" | "md";
  onDark?: boolean;
}) {
  const seg = size === "sm" ? "h-2 w-[3px]" : "h-3 w-[4px]";
  const empty = onDark ? "rgba(255,255,255,0.3)" : "var(--color-line)";
  const fill = bandFill(level);
  return (
    <span className="flex items-center gap-[2px]" aria-hidden="true">
      {[0, 1, 2, 3, 4].map((i) => (
        <span
          key={i}
          className={`${seg} rounded-sm`}
          style={{ background: i <= level ? fill : empty }}
        />
      ))}
    </span>
  );
}

/**
 * The eyes-closed cue, in one place.
 *
 * The module has always claimed to own "eyes as an open/closed cue" while five
 * call sites drew their own — three sizes in Cull, one in the Lightbox meta
 * row, one in Compare — which is why one of them had no label at all.
 *
 * `onClear` turns it into a button. The classifier reads a downcast gaze as a
 * blink, and this signal ranks the cull, so being able to say "no, they were
 * looking down" is the difference between a wrong badge and a good photo buried
 * at the bottom of every set it appears in.
 */
export function EyesBadge({
  size = "md",
  onClear,
}: {
  size?: "xs" | "sm" | "md";
  onClear?: () => void;
}) {
  const glyph = size === "xs" ? 9 : size === "sm" ? 11 : 15;

  // Tile-sized: a square chip over the photo, with no room for words.
  if (size !== "md") {
    const box = size === "xs" ? "h-3.5 w-3.5" : "h-4 w-4";
    return (
      <span
        className={`grid ${box} place-items-center rounded-sm bg-chip text-warning backdrop-blur`}
        title={t("cull.quality.eyesClosed")}
      >
        <Icon name="eye-off" size={glyph} />
      </span>
    );
  }

  const body = (
    <>
      <Icon name="eye-off" size={glyph} />
      {t("cull.quality.eyesClosed")}
    </>
  );
  if (!onClear) {
    return <span className="inline-flex items-center gap-1.5 text-warning">{body}</span>;
  }
  return (
    <button
      onClick={onClear}
      className="inline-flex items-center gap-1.5 rounded-sm px-1 py-0.5 text-warning transition-colors hover:bg-hover hover:text-ink"
      title={t("cull.quality.notClosedTip")}
    >
      {body}
      <span className="text-label text-faint">{t("cull.quality.notClosed")}</span>
    </button>
  );
}

/** A compact "N faces" badge for a grid tile. Nothing when a frame has no faces
 *  or hasn't been analysed — the caller decides when to render it. */
export function FaceCountBadge({ count, size = "md" }: { count: number; size?: "sm" | "md" }) {
  const cls =
    size === "sm"
      ? "gap-[3px] px-[5px] text-label"
      : "gap-1 px-1.5 text-caption";
  return (
    <span className={`flex items-center rounded-sm bg-chip text-ink backdrop-blur ${cls}`}>
      <Icon name="user" size={size === "sm" ? 10 : 12} />
      {count}
    </span>
  );
}
