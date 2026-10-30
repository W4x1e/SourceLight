import type { CSSProperties } from "react";
import { fileUrl } from "../lib/api";
import { Icon } from "../lib/icons";

/**
 * A square crop framing one face, drawn from whatever rendition we have.
 *
 * The box arrives as **fractions of the image**, which is the only form that is
 * correct here. It used to arrive in pixels, and those pixels were measured on
 * the 1600px preview while this scaled them against the 6000px capture size —
 * so every crop landed in the top-left corner of the frame and the review sheet
 * showed grass and sky.
 *
 * The positioning is done with a **percentage translate**, which is relative to
 * the element's *own* rendered box. That is what makes it aspect-agnostic:
 * setting the width and letting the height follow keeps the photo undistorted,
 * and `translate(-fx%, -fy%)` then moves the face's own centre onto the tile's
 * centre without anyone needing to know how tall the image came out.
 */
export default function FaceCrop({
  src,
  box,
  size,
  portrait,
}: {
  src: string | null | undefined;
  /** `[x, y, w, h]` as fractions of the image, or empty/absent when unknown.
   *  Elements are nullable because specta types a `Vec<f32>` that way. */
  box: (number | null)[] | null | undefined;
  /** Edge in pixels; left out, the crop fills its container's width. */
  size?: number;
  /** A portrait the backend cut from the full preview. Preferred: cropping
   *  the thumbnail here meant stretching a 20px face across the tile. */
  portrait?: string | null;
}) {
  const dims: CSSProperties = size
    ? { width: size, height: size }
    : { width: "100%", aspectRatio: "1 / 1" };
  if (portrait) {
    return (
      <img
        src={fileUrl(portrait)}
        alt=""
        draggable={false}
        loading="lazy"
        decoding="async"
        style={dims}
        className="bg-raised object-cover"
      />
    );
  }
  if (!src) {
    return (
      <div
        className="grid place-items-center bg-raised text-faint"
        style={dims}
      >
        <Icon name="user" size={Math.round((size ?? 120) / 2.5)} strokeWidth={1.4} />
      </div>
    );
  }
  // No box (analysed before the image size was recorded): show the frame rather
  // than a crop of the wrong part of it.
  if (!box || box.length < 4) {
    return (
      <img
        src={src}
        alt=""
        draggable={false}
        loading="lazy"
        decoding="async"
        style={dims}
        className="bg-raised object-cover"
      />
    );
  }

  const [x, y, w, h] = box.map((v) => v ?? 0) as [number, number, number, number];
  if (w <= 0 || h <= 0) {
    return (
      <img
        src={src}
        alt=""
        draggable={false}
        loading="lazy"
        decoding="async"
        style={dims}
        className="bg-raised object-cover"
      />
    );
  }
  // How much of the tile the face should occupy — the rest is the air that
  // makes it read as a portrait rather than a swatch of skin.
  const FILL = 0.62;
  return (
    <div
      className="relative overflow-hidden bg-raised"
      style={dims}
    >
      <img
        src={src}
        alt=""
        draggable={false}
        loading="lazy"
        decoding="async"
        style={{
          position: "absolute",
          left: "50%",
          top: "50%",
          // Scale so the face box spans `FILL` of the tile's width. Height is
          // left to follow, so the photo keeps its own proportions.
          width: `${(100 * FILL) / w}%`,
          height: "auto",
          maxWidth: "none",
          // Percentages here are of the image's own rendered box, which is what
          // puts the face's centre on the tile's centre whatever shape the
          // photo turned out to be.
          transform: `translate(${-(x + w / 2) * 100}%, ${-(y + h / 2) * 100}%)`,
        }}
      />
    </div>
  );
}
