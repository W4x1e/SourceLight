import { gridFractions, type GridKind } from "../lib/composition";

/**
 * A composition grid over whatever box it is put in: the Cull,
 * Lightbox and Compare frames, and the crop rectangle in Develop. Lines are
 * positioned in percent, so they follow the box; `thickness` keeps them a
 * hairline when the box itself is scaled up by a zoom.
 */
export default function CompositionGrid({ kind, thickness = 1 }: { kind: GridKind; thickness?: number }) {
  const at = gridFractions(kind);
  if (at.length === 0) return null;
  const line = "pointer-events-none absolute bg-white/40 shadow-[0_0_1px_rgba(0,0,0,0.6)]";
  return (
    <div className="pointer-events-none absolute inset-0" data-grid={kind}>
      {at.map((f) => (
        <div key={`v${f}`} className={`${line} top-0 h-full`} style={{ left: `${f * 100}%`, width: thickness }} />
      ))}
      {at.map((f) => (
        <div key={`h${f}`} className={`${line} left-0 w-full`} style={{ top: `${f * 100}%`, height: thickness }} />
      ))}
    </div>
  );
}
