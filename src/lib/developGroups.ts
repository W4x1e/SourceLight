import type { Group } from "../bindings";
import type { Key } from "../i18n";

/**
 * The parts of an edit that copy, paste and save as a preset on their own
 * — mirroring `develop::stack::Group`, in the order the panels run.
 */
export const DEVELOP_GROUPS: { value: Group; label: Key }[] = [
  { value: "whiteBalance", label: "develop.group.whiteBalance" },
  { value: "light", label: "develop.group.light" },
  { value: "color", label: "develop.group.color" },
  { value: "curves", label: "develop.group.curves" },
  { value: "detail", label: "develop.group.detail" },
  { value: "lens", label: "develop.group.lens" },
  { value: "effects", label: "develop.group.effects" },
  { value: "lut", label: "develop.group.lut" },
  { value: "crop", label: "develop.group.crop" },
  { value: "masks", label: "develop.group.masks" },
  { value: "spots", label: "develop.group.spots" },
];

/** Drawn on one photo's content, so left behind unless asked for — as
 *  `Group::copied_by_default` says on the Rust side. */
const PHOTO_BOUND: Group[] = ["crop", "masks", "spots"];

/** What a copy, a paste or a new preset takes unless told otherwise. */
export const COPIED_BY_DEFAULT: Group[] = DEVELOP_GROUPS.map((g) => g.value).filter((g) => !PHOTO_BOUND.includes(g));
