import type { ReactNode } from "react";

/**
 * One line-icon set, drawn to a 24×24 grid at stroke width 1.7.
 *
 * A single component keyed by name rather than a file per glyph: the app uses a
 * dozen small chrome icons and a shared `<Icon>` keeps them one size, one
 * weight, and one place to change. Colour comes from `currentColor`, so an icon
 * takes the text colour of whatever it sits in.
 *
 * Glyphs are JSX nodes rather than markup strings so the component can render
 * them directly as children — no `dangerouslySetInnerHTML`, so there is no HTML
 * sink to keep an eye on even though the data is static.
 */
export type IconName =
  | "chevron"
  | "trash"
  | "back"
  | "desktop"
  | "expand"
  | "sync"
  | "chevrons"
  | "check"
  | "eye"
  | "eye-off"
  | "user"
  | "info"
  | "copy"
  | "compare"
  | "calendar"
  | "star"
  | "clock"
  | "loupe"
  | "cull"
  | "minus"
  | "ban"
  | "more"
  | "plus"
  | "solo"
  | "crop"
  | "mask"
  | "spot"
  | "dropper"
  | "pointer"
  | "warn"
  | "folder"
  | "search"
  | "grid"
  | "trips"
  | "settings"
  | "keyboard"
  | "close"
  | "rotate-left"
  | "rotate-right"
  | "flip"
  | "layers"
  | "before-after"
  | "adjust"
  | "activity"
  | "import"
  | "sort"
  | "filter"
  | "flag"
  | "flag-off"
  | "geometry"
  | "heal"
  | "bookmark"
  | "pin"
  | "pencil"
  | "invert"
  | "undo"
  | "redo"
  | "play"
  | "peaking"
  | "thirds"
  | "phone"
  | "menu"
  | "select"
  | "sun"
  | "drop"
  | "curve"
  | "mixer"
  | "wheel"
  | "detail"
  | "sparkle"
  | "lens"
  | "cube";

const PATHS: Record<IconName, ReactNode> = {
  // Focus peaking: a focus point's brackets, with the sharp spot marked.
  peaking: (
    <>
      <path d="M4 9V5a1 1 0 0 1 1-1h4" />
      <path d="M15 4h4a1 1 0 0 1 1 1v4" />
      <path d="M20 15v4a1 1 0 0 1-1 1h-4" />
      <path d="M9 20H5a1 1 0 0 1-1-1v-4" />
      <circle cx="12" cy="12" r="2.5" />
    </>
  ),
  // A composition grid: a frame cut in thirds.
  thirds: (
    <>
      <rect x="3" y="5" width="18" height="14" rx="1" />
      <path d="M9 5v14M15 5v14M3 9.67h18M3 14.33h18" />
    </>
  ),
  // A slideshow.
  play: <path d="M7 4.5v15l12-7.5z" />,
  chevron: <path d="M8 10l4 4 4-4" />,
  back: <path d="M15 5l-7 7 7 7" />,
  desktop: (
    <>
      <rect x="3" y="4" width="18" height="12" rx="1.5" />
      <path d="M9 20h6M12 16v4" />
    </>
  ),
  expand: <path d="M14 4h6v6M20 4l-7 7M10 20H4v-6M4 20l7-7" />,
  sync: (
    <>
      <path d="M19.5 9A8 8 0 0 0 5.2 7.6L4 9M4.5 15a8 8 0 0 0 14.3 1.4L20 15" />
      <path d="M4 4.5V9h4.5M20 19.5V15h-4.5" />
    </>
  ),
  chevrons: (
    <>
      <path d="M13 6l-6 6 6 6" />
      <path d="M19 6l-6 6 6 6" />
    </>
  ),
  check: <path d="M5 12.5l4.5 4.5L19 6.5" />,
  eye: (
    <>
      <path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7-10-7-10-7z" />
      <circle cx="12" cy="12" r="3" />
    </>
  ),
  "eye-off": (
    <>
      <path d="M3 3l18 18" />
      <path d="M10.6 5.1A9.9 9.9 0 0 1 12 5c6.4 0 10 7 10 7a17.9 17.9 0 0 1-3.1 3.9M6.1 6.2A17.7 17.7 0 0 0 2 12s3.6 7 10 7a9.8 9.8 0 0 0 4-.8" />
      <path d="M9.9 9.9a3 3 0 0 0 4.2 4.2" />
    </>
  ),
  user: (
    <>
      <circle cx="12" cy="8" r="4" />
      <path d="M4 21c0-4.4 3.6-7 8-7s8 2.6 8 7" />
    </>
  ),
  // There is deliberately no sparkle glyph: as a stand-in for "AI" it was the
  // most generic mark in the app. Name the action and use its own icon.
  info: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v5.5" />
      <path d="M12 7.6v.01" />
    </>
  ),
  copy: (
    <>
      <rect x="9" y="9" width="11" height="11" rx="2" />
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
    </>
  ),
  compare: (
    <>
      <rect x="2" y="6" width="9" height="12" rx="1.5" />
      <rect x="13" y="6" width="9" height="12" rx="1.5" />
    </>
  ),
  calendar: (
    <>
      <rect x="3" y="4" width="18" height="17" rx="2" />
      <path d="M8 2v4M16 2v4M3 9h18" />
    </>
  ),
  star: <path d="M12 3l2.6 5.6 6 .7-4.4 4.2 1.2 6-5.4-3-5.4 3 1.2-6L3.4 9.3l6-.7z" />,
  // The phone editor's tools.
  // A sun: light.
  sun: (
    <>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2.5V5M12 19v2.5M2.5 12H5M19 12h2.5M5.3 5.3l1.8 1.8M16.9 16.9l1.8 1.8M5.3 18.7l1.8-1.8M16.9 7.1l1.8-1.8" />
    </>
  ),
  // A drop: colour.
  drop: <path d="M12 3c3.6 4.3 6 7.5 6 10.9a6 6 0 0 1-12 0C6 10.5 8.4 7.3 12 3z" />,
  // A tone curve on its axes.
  curve: (
    <>
      <path d="M4 4v16h16" />
      <path d="M4 19c7 0 9-14 16-14" />
    </>
  ),
  // Three upright sliders: the colour mixer.
  mixer: (
    <>
      <path d="M6 4v16M12 4v16M18 4v16" />
      <circle cx="6" cy="14" r="2" />
      <circle cx="12" cy="8" r="2" />
      <circle cx="18" cy="16" r="2" />
    </>
  ),
  // A colour wheel with its point: colour grading.
  wheel: (
    <>
      <circle cx="12" cy="12" r="9" />
      <circle cx="15" cy="9" r="2.2" />
    </>
  ),
  // A cut stone: detail and sharpening.
  detail: (
    <>
      <path d="M12 3l7 9-7 9-7-9z" />
      <path d="M12 3v18" />
    </>
  ),
  // A four-pointed star: effects.
  sparkle: <path d="M12 3l1.9 5.6 5.6 1.9-5.6 1.9L12 18l-1.9-5.6-5.6-1.9 5.6-1.9z" />,
  // A lens, front on: optics.
  lens: (
    <>
      <circle cx="12" cy="12" r="9" />
      <circle cx="12" cy="12" r="4" />
    </>
  ),
  // Three bars: the phone app's menu.
  menu: <path d="M4 7h16M4 12h16M4 17h16" />,
  // A ticked box: choosing photos.
  select: (
    <>
      <rect x="4" y="4" width="16" height="16" rx="2" />
      <path d="M8.5 12.5l2.5 2.5 4.5-5" />
    </>
  ),
  // A phone, upright, with its speaker slot: the phone app's own photos.
  phone: (
    <>
      <rect x="6.5" y="2.5" width="11" height="19" rx="2" />
      <path d="M10.5 5.5h3" />
    </>
  ),
  clock: (
    <>
      <circle cx="12" cy="12" r="8" />
      <path d="M12 8v4l3 2" />
    </>
  ),
  loupe: (
    <>
      <circle cx="10" cy="10" r="6" />
      <path d="M20 20l-6-6" />
    </>
  ),
  // Cull: a frame with a verdict on it. Its own glyph because the loupe it
  // used before is the same magnifier as Search.
  cull: (
    <>
      <rect x="3" y="5" width="18" height="14" rx="1.5" />
      <path d="M8.5 12.5l2.5 2.5 4.5-5" />
    </>
  ),
  minus: <path d="M5 12h14" />,
  // Left out: a facet value a filter excludes.
  ban: <path d="M12 3.5a8.5 8.5 0 1 0 0 17a8.5 8.5 0 1 0 0-17ZM6 6l12 12" />,
  more: <path d="M6 12h.01M12 12h.01M18 12h.01" strokeWidth={3} strokeLinecap="round" />,
  plus: <path d="M12 5v14M5 12h14" />,
  // An accordion: one group open between two closed ones — "one at a time".
  solo: (
    <>
      <path d="M4 4.5h16M4 19.5h16" />
      <rect x="4" y="8.5" width="16" height="7" rx="1.5" />
    </>
  ),
  crop: (
    <>
      <path d="M6 2v14a2 2 0 0 0 2 2h14" />
      <path d="M2 6h14a2 2 0 0 1 2 2v14" />
    </>
  ),
  mask: (
    <>
      <circle cx="9" cy="12" r="7" />
      <circle cx="15" cy="12" r="7" />
    </>
  ),
  spot: (
    <>
      <circle cx="12" cy="12" r="5" />
      <path d="M12 3v2M12 19v2M3 12h2M19 12h2" />
    </>
  ),
  dropper: (
    <>
      <path d="M19 3l2 2-9 9-3 1 1-3 9-9z" />
      <path d="M11 8l-6 6a2 2 0 0 0 0 3 2 2 0 0 0 3 0l6-6" />
    </>
  ),
  // A pointer: click on the photo to choose what to select. The eyedropper
  // is the white-balance picker's, and doubling as this said the wrong thing.
  pointer: (
    <>
      <path d="M6 3.5v14l3.6-3.4 2.6 6 2.4-1-2.6-6 5-.3z" />
    </>
  ),
  warn: (
    <>
      <path d="M12 3l9 16H3z" />
      <path d="M12 10v4M12 17h.01" />
    </>
  ),
  folder: <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />,
  search: (
    <>
      <circle cx="10" cy="10" r="6" />
      <path d="M20 20l-6-6" />
    </>
  ),
  grid: (
    <>
      <rect x="3" y="3" width="7" height="7" rx="1" />
      <rect x="14" y="3" width="7" height="7" rx="1" />
      <rect x="3" y="14" width="7" height="7" rx="1" />
      <rect x="14" y="14" width="7" height="7" rx="1" />
    </>
  ),
  trips: (
    <>
      <circle cx="12" cy="10" r="3" />
      <path d="M12 21c-5-5-8-8.5-8-12a8 8 0 1 1 16 0c0 3.5-3 7-8 12z" />
    </>
  ),
  settings: (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M19 12a7 7 0 0 0-.1-1.2l2-1.5-2-3.5-2.3 1a7 7 0 0 0-2-1.2l-.3-2.4h-4l-.3 2.4a7 7 0 0 0-2 1.2l-2.3-1-2 3.5 2 1.5A7 7 0 0 0 5 12a7 7 0 0 0 .1 1.2l-2 1.5 2 3.5 2.3-1a7 7 0 0 0 2 1.2l.3 2.4h4l.3-2.4a7 7 0 0 0 2-1.2l2.3 1 2-3.5-2-1.5A7 7 0 0 0 19 12z" />
    </>
  ),
  keyboard: (
    <>
      <rect x="2" y="6" width="20" height="12" rx="2" />
      <path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M6 14h.01M18 14h.01M9 14h6" />
    </>
  ),
  close: <path d="M6 6l12 12M18 6L6 18" />,
  trash: <path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3" />,
  "rotate-left": (
    <>
      <path d="M9 5H5V1" />
      <path d="M5 5a9 9 0 1 1-2 6" />
    </>
  ),
  "rotate-right": (
    <>
      <path d="M15 5h4V1" />
      <path d="M19 5a9 9 0 1 0 2 6" />
    </>
  ),
  // A hooked arrow back and forward: Develop's undo and redo.
  undo: (
    <>
      <path d="M9 14 4 9l5-5" />
      <path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11" />
    </>
  ),
  redo: (
    <>
      <path d="m15 14 5-5-5-5" />
      <path d="M20 9H9.5a5.5 5.5 0 0 0 0 11H13" />
    </>
  ),
  flip: (
    <>
      <path d="M12 3v18" />
      <path d="M7 8l-4 4 4 4" />
      <path d="M17 8l4 4-4 4" />
    </>
  ),
  "before-after": (
    <>
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M12 4v16" />
    </>
  ),
  layers: (
    <>
      <path d="M12 3l9 5-9 5-9-5z" />
      <path d="M3 12l9 5 9-5" />
    </>
  ),
  // A cube: a LUT, which maps every colour through a cube of colours.
  cube: (
    <>
      <path d="M12 3l8 4.5-8 4.5-8-4.5z" />
      <path d="M4 7.5v9l8 4.5 8-4.5v-9" />
      <path d="M12 12v9" />
    </>
  ),
  // Two stacked slider tracks with a knob on each — the Develop / adjust glyph.
  adjust: (
    <>
      <path d="M4 7h16M4 17h16" />
      <circle cx="9" cy="7" r="2.3" />
      <circle cx="15" cy="17" r="2.3" />
    </>
  ),
  // A pulse trace — the Activity / background-jobs glyph.
  activity: <path d="M3 12h4l2.5 6 5-13 2.5 7H21" />,
  // Tray with a down arrow — Import / scan into the library.
  import: <path d="M12 3v10m0 0 4-4m-4 4-4-4M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2" />,
  // Ascending-order bars with a direction arrow.
  sort: <path d="M7 4v16M7 20l-3-3M7 4l3 3M14 7h6M14 12h4M14 17h2" />,
  filter: <path d="M4 5h16l-6 7.5V19l-4-2v-4.5z" />,
  // A raised flag on a pole, filled — pick. Pick and reject are told apart
  // by shape as well as hue: they were one outline at two opacities,
  // red against green, which about one man in twelve cannot tell apart.
  flag: <path d="M6 21V4h11l-2 4 2 4H6" strokeLinejoin="round" fill="currentColor" />,
  // The same flag, empty and struck through — reject / unflag.
  "flag-off": (
    <>
      <path d="M6 21V4h11l-2 4 2 4H6" strokeLinejoin="round" />
      <path d="M3.5 3.5 20.5 20.5" />
    </>
  ),
  // Corner brackets with a transform arrow — the geometry / straighten tool.
  geometry: <path d="M4 20 20 4M5 9V5h4M15 19h4v-4" />,
  // A healing brush — clone/heal spot tool.
  heal: (
    <>
      <path d="M14 4 5 13l-1.5 5.5L9 17l9-9z" />
      <path d="M14 4l4 4" />
    </>
  ),
  // A ribbon bookmark — the Cull workspace marker.
  bookmark: <path d="M6 3.5v17l6-4 6 4v-17z" strokeLinejoin="round" />,
  // A map pin with a dot — Places.
  pin: (
    <>
      <path d="M12 21c5-5.5 7-8.4 7-11a7 7 0 1 0-14 0c0 2.6 2 5.5 7 11Z" />
      <circle cx="12" cy="10" r="2.4" />
    </>
  ),
  // A pencil — rename / edit in place.
  pencil: (
    <>
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z" />
    </>
  ),
  // A half-filled circle — invert a mask component's coverage.
  invert: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 3a9 9 0 0 1 0 18z" fill="currentColor" stroke="none" />
    </>
  ),
};

interface Props {
  name: IconName;
  /** Pixel size; the glyph scales to it. Default 17, matching the chrome. */
  size?: number;
  className?: string;
  strokeWidth?: number;
  /** Filled in the current colour as well as stroked: a star that is set. */
  filled?: boolean;
}

export function Icon({ name, size = 17, className, strokeWidth = 1.7, filled = false }: Props) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill={filled ? "currentColor" : "none"}
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      {PATHS[name]}
    </svg>
  );
}
