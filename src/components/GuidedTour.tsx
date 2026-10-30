import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useStore } from "../store";
import { comboToLabel } from "../lib/keymap";
import { TOUR, placeCard, type Box, type TourStep } from "../lib/tour";
import { t, tx } from "../i18n";
import { scrimOpen } from "../lib/useEscape";

/** Room around the spotlit control, so the ring doesn't sit on its edge. */
const PAD = 6;
const CARD_W = 320;

/** The target's box, padded, or null when it isn't on screen to point at. */
function measure(target: string | null): Box | null {
  if (!target) return null;
  const el = document.querySelector(`[data-tour="${target}"]`);
  if (!el) return null;
  const r = el.getBoundingClientRect();
  if (r.width === 0 || r.height === 0) return null;
  return { left: r.left - PAD, top: r.top - PAD, width: r.width + PAD * 2, height: r.height + PAD * 2 };
}

const sameBox = (a: Box | null, b: Box | null) =>
  a === b ||
  (!!a && !!b && a.left === b.left && a.top === b.top && a.width === b.width && a.height === b.height);

function Body({ step }: { step: TourStep }) {
  const keymap = useStore((s) => s.keymap);
  const keycaps: Record<string, ReactNode> = {};
  for (const [name, k] of Object.entries(step.keys ?? {})) {
    keycaps[name] = (
      <span className="kbd mx-0.5 align-[1px]">
        {"key" in k ? comboToLabel(keymap[k.key]) : "label" in k ? t(k.label) : k.keys}
      </span>
    );
  }
  // The body's placeholders differ by step, so the params are checked here, not at the call.
  return <>{tx(step.body, keycaps as never)}</>;
}

/**
 * The guided tour (see lib/tour): dims the app, cuts a hole around one real
 * control, and puts a card beside it.
 *
 * The layer takes every click, and a capture-phase key listener takes every
 * key, so nothing underneath can be acted on by accident — a stray X during
 * the Cull step would otherwise reject a photo. Dialogs (z-60) sit above it and
 * keep their keys.
 *
 * `closing` runs the exit animation: App keeps the tour mounted for it (see `usePresence`).
 */
export default function GuidedTour({ closing = false }: { closing?: boolean }) {
  const tour = useStore((s) => s.tour);
  const tourGo = useStore((s) => s.tourGo);
  const endTour = useStore((s) => s.endTour);

  // Held past the end so the last card fades out as it was, not as step 1.
  const lastStep = useRef(0);
  if (tour) lastStep.current = tour.step;
  const index = Math.min(Math.max(lastStep.current, 0), TOUR.length - 1);
  const step = TOUR[index]!;
  const last = index === TOUR.length - 1;

  const next = useCallback(() => {
    if (last) endTour();
    else tourGo(index + 1);
  }, [last, index, endTour, tourGo]);
  const back = useCallback(() => index > 0 && tourGo(index - 1), [index, tourGo]);

  // Into the step's workspace, then find its target. Most targets are already
  // on screen, so the first look is immediate; one in a workspace just switched
  // to (Cull's verdict panel) renders a frame or so later, so a miss looks again
  // for a few frames before settling on a centred card.
  const [spot, setSpot] = useState<Box | null>(null);
  useEffect(() => {
    if (closing) return;
    if (step.mode && useStore.getState().mode !== step.mode) useStore.getState().setMode(step.mode);
    let timer = 0;
    let tries = 0;
    const look = () => {
      const box = measure(step.target);
      // A miss while still looking keeps the last spot, rather than
      // collapsing to the middle and springing back a frame later.
      if (!box && step.target && ++tries < 20) {
        timer = window.setTimeout(look, 16);
        return;
      }
      setSpot((prev) => (sameBox(prev, box) ? prev : box));
    };
    look();
    const onResize = () => {
      const box = measure(step.target);
      setSpot((prev) => (sameBox(prev, box) ? prev : box));
    };
    window.addEventListener("resize", onResize);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("resize", onResize);
    };
  }, [step, closing]);

  // The card's own height decides where it fits, so it is measured, not assumed.
  const cardRef = useRef<HTMLDivElement>(null);
  const [cardH, setCardH] = useState(180);
  const [viewport, setViewport] = useState({ width: window.innerWidth, height: window.innerHeight });
  useLayoutEffect(() => {
    const h = cardRef.current?.offsetHeight;
    if (h && h !== cardH) setCardH(h);
  });
  useEffect(() => {
    const onResize = () => setViewport({ width: window.innerWidth, height: window.innerHeight });
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  const cardW = Math.min(CARD_W, viewport.width - 32);
  const pos = placeCard(spot, { width: cardW, height: cardH }, viewport);

  // Each step starts with Next focused, so Enter walks the tour.
  const nextRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!closing) nextRef.current?.focus();
  }, [index, closing]);

  useEffect(() => {
    if (closing) return;
    const onKey = (e: KeyboardEvent) => {
      if (scrimOpen()) return;
      const card = cardRef.current;
      const inCard = !!card && card.contains(document.activeElement);
      // Enter and Space on the card's own buttons press them, as usual.
      if (inCard && (e.key === "Enter" || e.key === " ")) {
        e.stopImmediatePropagation();
        return;
      }
      e.stopImmediatePropagation();
      e.preventDefault();
      if (e.key === "ArrowRight" || e.key === "Enter") next();
      else if (e.key === "ArrowLeft") back();
      else if (e.key === "Escape") endTour();
      else if (e.key === "Tab" && card) {
        // Focus stays in the card: the app underneath is not reachable.
        const buttons = [...card.querySelectorAll("button")];
        const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
        const to = (at + (e.shiftKey ? -1 : 1) + buttons.length) % buttons.length;
        buttons[to]?.focus();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [closing, next, back, endTour]);

  // With nothing to point at, the hole shrinks to a point in the middle, so the
  // dim is whole and moving to the next target still reads as one motion.
  const hole = spot ?? { left: viewport.width / 2, top: viewport.height / 2, width: 0, height: 0 };
  const move = "var(--dur-base) var(--ease-out)";

  return (
    <div
      className="anim-scrim fixed inset-0 z-[55]"
      data-closing={closing || undefined}
      data-testid="guided-tour"
    >
      <div
        aria-hidden="true"
        className="pointer-events-none absolute rounded-lg"
        style={{
          ...hole,
          boxShadow: "0 0 0 9999px var(--color-scrim)",
          outline: spot ? "2px solid var(--color-accent)" : "none",
          outlineOffset: 0,
          transition: `left ${move}, top ${move}, width ${move}, height ${move}`,
        }}
      />
      <div
        ref={cardRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="tour-title"
        aria-describedby="tour-body"
        className="absolute rounded-lg border border-line bg-panel p-4 shadow-2xl"
        style={{ left: pos.left, top: pos.top, width: cardW, transition: `left ${move}, top ${move}` }}
      >
        <div className="mb-1 text-label text-faint">
          {tx("shell.tour.progress", {
            current: <span className="tabular-nums">{index + 1}</span>,
            total: <span className="tabular-nums">{TOUR.length}</span>,
          })}
        </div>
        <h2 id="tour-title" className="font-display text-display text-ink">
          {t(step.title)}
        </h2>
        <p id="tour-body" aria-live="polite" className="mt-1.5 text-caption leading-relaxed text-muted">
          <Body step={step} />
        </p>
        <div className="mt-4 flex items-center gap-2">
          {!last && (
            <button onClick={endTour} className="btn-ghost -ml-2 text-muted">
              {t("shell.tour.skip")}
            </button>
          )}
          <div className="flex-1" />
          {index > 0 && (
            <button onClick={back} className="btn-solid">
              {t("common.back")}
            </button>
          )}
          <button ref={nextRef} onClick={next} className="btn-accent">
            {last ? t("common.done") : index === 0 ? t("shell.tour.showMe") : t("common.next")}
          </button>
        </div>
      </div>
    </div>
  );
}
