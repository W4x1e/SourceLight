import { Icon, type IconName } from "../../lib/icons";
import { t } from "../../i18n";
import { ASPECT_PRESETS, type Stack } from "../../types";
import NumberSlider from "../NumberSlider";

type Geometry = Stack["geometry"];

interface Props {
  geometry: Geometry;
  /** The locked aspect ratio, or null for free. */
  aspect: number | null;
  /** The photo's own shape before the crop, turns included — "Original". */
  originalAspect: number;
  /** Anything about the framing differs from the camera's. */
  changed: boolean;
  onAspect: (ratio: number | null, label: string) => void;
  onGeometry: (patch: Partial<Geometry>, label: string) => void;
  onStraighten: (degrees: number) => void;
  onStraightenCommit: () => void;
  onReset: () => void;
  onDone: () => void;
}

/**
 * The crop tool's controls, in the rail while cropping — as Mask and Retouch
 * have theirs.
 *
 * They used to live in a Geometry group at the foot of the adjustment list,
 * collapsed by default, so pressing Crop drew a rectangle over the photo and
 * left the aspect ratios, rotation and straightening somewhere below the fold.
 * Now entering crop brings its controls with it and leaving takes them away.
 *
 * "Free" unlocks the ratio and nothing else; putting the frame back is Reset's
 * job. Free used to double as the reset because there was no other visible
 * one, so reaching for a free ratio threw the crop away.
 */
export default function CropPanel({
  geometry,
  aspect,
  originalAspect,
  changed,
  onAspect,
  onGeometry,
  onStraighten,
  onStraightenCommit,
  onReset,
  onDone,
}: Props) {
  const ratios: { label: string; ratio: number | null }[] = [
    { ...ASPECT_PRESETS[0]!, label: t("tools.crop.free") },
    { label: t("tools.crop.original"), ratio: originalAspect },
    ...ASPECT_PRESETS.slice(1),
  ];
  // Two presets can land on the same number (a 3:2 camera's Original is 3:2);
  // only one of them should look chosen.
  const chosen = ratios.find((r) => r.ratio === aspect || (r.ratio && aspect && Math.abs(r.ratio - aspect) < 1e-6));

  const turn = (icon: IconName, title: string, short: string, onClick: () => void, on = false) => (
    <button
      onClick={onClick}
      title={title}
      aria-pressed={on}
      className={`flex flex-1 items-center justify-center gap-1.5 py-1.5 text-caption ${
        on ? "seg-on" : "text-muted hover:bg-hover hover:text-ink"
      }`}
    >
      <Icon name={icon} size={14} />
      {short}
    </button>
  );

  return (
    <div className="p-3">
      <div className="mb-2 flex items-center gap-2">
        <span className="flex-1 text-title font-semibold">{t("tools.crop.title")}</span>
        {/* Solid, not the key: Export is the one key on Develop's screen. */}
        <button onClick={onDone} className="btn-solid px-2.5 py-1 text-caption" title={t("tools.crop.doneTitle")}>
          {t("common.done")}
        </button>
      </div>
      <p className="mb-3 text-label leading-relaxed text-faint">
        {t("tools.crop.intro")}
      </p>

      <div className="section-label px-0">{t("tools.crop.aspectRatio")}</div>
      <div className="mb-3 grid grid-cols-3 gap-px overflow-hidden rounded-md bg-raised">
        {ratios.map((r) => (
          <button
            key={r.label}
            onClick={() => onAspect(r.ratio, r.label)}
            aria-pressed={chosen === r}
            className={`py-1.5 text-caption ${
              chosen === r ? "seg-on" : "bg-raised text-muted hover:bg-hover hover:text-ink"
            }`}
          >
            {r.label}
          </button>
        ))}
      </div>

      <div className="section-label px-0">{t("tools.crop.rotateFlip")}</div>
      <div className="mb-3 seg-group">
        {turn("rotate-left", t("tools.crop.rotateLeftTitle"), t("tools.crop.rotateLeft"), () =>
          onGeometry({ quarterTurns: (geometry.quarterTurns + 3) % 4 }, t("tools.crop.history.rotateLeft")),
        )}
        {turn("rotate-right", t("tools.crop.rotateRightTitle"), t("tools.crop.rotateRight"), () =>
          onGeometry({ quarterTurns: (geometry.quarterTurns + 1) % 4 }, t("tools.crop.history.rotateRight")),
        )}
        {turn(
          "flip",
          t("tools.crop.flipTitle"),
          t("tools.crop.flip"),
          () => onGeometry({ flip: !geometry.flip }, t("tools.crop.history.flip")),
          geometry.flip,
        )}
      </div>

      <NumberSlider
        label={t("tools.crop.straighten")}
        value={geometry.straighten}
        min={-15}
        max={15}
        step={0.1}
        precision={1}
        suffix="°"
        onChange={onStraighten}
        onCommit={onStraightenCommit}
      />

      <button
        onClick={onReset}
        disabled={!changed}
        className="btn-ghost mt-2 w-full text-muted hover:text-ink disabled:pointer-events-none disabled:opacity-40"
        title={t("tools.crop.resetTitle")}
      >
        {t("tools.crop.reset")}
      </button>
    </div>
  );
}
