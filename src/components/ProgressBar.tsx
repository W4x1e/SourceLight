/**
 * A thin progress bar that a screen reader can hear: role=progressbar
 * with the percentage, named by what is in progress. `pct` null is a bar that
 * doesn't know yet, sweeping across rather than sitting at zero: a block that
 * only pulsed in place looked stuck. The track clips it (`overflow-hidden`).
 */
export default function ProgressBar({
  pct,
  label,
  className = "h-1 overflow-hidden rounded-full bg-line",
  barClassName = "h-full rounded-full bg-accent transition-[width]",
}: {
  pct: number | null;
  label: string;
  className?: string;
  barClassName?: string;
}) {
  const known = pct === null ? null : Math.max(0, Math.min(100, Math.round(pct)));
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={known ?? undefined}
      className={className}
    >
      <div
        className={`${barClassName} ${known === null ? "progress-sweep w-1/3" : ""}`}
        style={known === null ? undefined : { width: `${known}%` }}
      />
    </div>
  );
}
