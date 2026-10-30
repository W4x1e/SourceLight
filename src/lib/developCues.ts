/** Track gradients painted on the colour-cue sliders, shared by the global White
 *  Balance panel, the Colour panel and the per-mask local adjustments so the same
 *  control looks the same in all of them. Endpoints are the poles of each axis
 *  with a neutral grey at centre (0). */
export const TEMP_GRADIENT = "linear-gradient(90deg, #4a86d8 0%, #8a8a90 50%, #e8b23a 100%)";
export const TINT_GRADIENT = "linear-gradient(90deg, #5bb85e 0%, #8a8a90 50%, #c74fc7 100%)";
/** Saturation reads left-to-right as "no colour → full colour": a spread of hues
 *  that fades in out of grey. Its right pole used to be the accent, which said
 *  "more colour" while the accent was blue; since the accent became white it
 *  drew a grey-to-white bar, which says nothing about colour at all. */
export const SAT_GRADIENT =
  "linear-gradient(90deg, #8a8a90 0%, rgb(138 138 144 / 0) 100%), " +
  "linear-gradient(90deg, #d8584e, #d8b04e, #5bb85e, #4ab0c8, #4a86d8, #c74fc7)";
