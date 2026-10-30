import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { t } from "../../src/i18n";
import { PanelHeader } from "./ui";
import { api, type GpuReport } from "./api";

/** How a slider frame reaches the screen: encoded, or as raw pixels. */
type Mode = "jpeg" | "rgba";
type Live = { frameMs: number; renderMs: number; encodeMs: number; fps: number };

const ms = (v: number) => (v < 10 ? v.toFixed(1) : String(Math.round(v)));

/**
 * The GPU test (docs/MOBILE.md section 4, step 1): runs the desktop's Develop
 * pipeline on this phone's GPU and times it, then puts the result under two
 * sliders to feel how it responds. Temporary, until editing is built.
 */
export default function GpuTest({ photoId, onClose }: { photoId: number | null; onClose: () => void }) {
  const [report, setReport] = useState<GpuReport | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [exposure, setExposure] = useState(0);
  const [warmth, setWarmth] = useState(0);
  const [mode, setMode] = useState<Mode>("jpeg");
  const [live, setLive] = useState<Live | null>(null);
  const [copied, setCopied] = useState(false);
  const canvas = useRef<HTMLCanvasElement>(null);

  // Frames as large as the photo shows on this screen, in device pixels.
  const screen = {
    width: Math.round(window.innerWidth * window.devicePixelRatio),
    height: Math.round(window.innerHeight * window.devicePixelRatio),
  };

  const run = async () => {
    setRunning(true);
    setError(null);
    setLive(null);
    try {
      // The first frame follows from the report, through the effect below.
      setReport(await api.gpuTest(photoId, screen.width, screen.height));
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  };

  // Latest wins: one frame in flight, and when it lands, the newest settings
  // are asked for if they moved meanwhile. That is how a slider drag behaves.
  const want = useRef<{ exposure: number; warmth: number; mode: Mode } | null>(null);
  const busy = useRef(false);
  const done = useRef<number[]>([]);
  const pump = useCallback(async () => {
    if (busy.current || !want.current) return;
    const asked = want.current;
    want.current = null;
    busy.current = true;
    const started = performance.now();
    try {
      const name = `${asked.mode}_${asked.exposure.toFixed(2)}_${asked.warmth.toFixed(2)}`;
      const res = await fetch(convertFileSrc(name, "gputest"));
      if (!res.ok) throw new Error(await res.text());
      const w = Number(res.headers.get("X-Width"));
      const h = Number(res.headers.get("X-Height"));
      const c = canvas.current;
      if (c) {
        if (c.width !== w || c.height !== h) Object.assign(c, { width: w, height: h });
        const ctx = c.getContext("2d")!;
        if (asked.mode === "jpeg") {
          ctx.drawImage(await createImageBitmap(await res.blob()), 0, 0);
        } else {
          ctx.putImageData(new ImageData(new Uint8ClampedArray(await res.arrayBuffer()), w, h), 0, 0);
        }
      }
      const now = performance.now();
      done.current = [...done.current.filter((d) => now - d < 1000), now];
      setLive({
        frameMs: now - started,
        renderMs: Number(res.headers.get("X-Render-Ms")),
        encodeMs: Number(res.headers.get("X-Encode-Ms")),
        fps: done.current.length,
      });
    } catch (e) {
      setError(String(e));
    } finally {
      busy.current = false;
      if (want.current) void pump();
    }
  }, []);

  useEffect(() => {
    if (!report || report.error) return;
    want.current = { exposure, warmth, mode };
    void pump();
  }, [exposure, warmth, mode, report, pump]);

  const copy = async () => {
    const text = JSON.stringify({ screen, report, live, mode, userAgent: navigator.userAgent }, null, 2);
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="flex h-full flex-col bg-base">
      <PanelHeader title={t("mobile.gpuTest.title")} onClose={onClose} />
      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5 pb-[calc(var(--safe-bottom)+1.25rem)]">
        <p className="text-body leading-relaxed text-muted">{t("mobile.gpuTest.blurb")}</p>
        <div className="mt-4 flex gap-2">
          <button className="btn-solid min-h-12" disabled={running} onClick={() => void run()}>
            {running ? t("mobile.gpuTest.running") : report ? t("mobile.gpuTest.again") : t("mobile.gpuTest.run")}
          </button>
          {report && (
            <button className="btn-ghost min-h-12 text-muted" onClick={() => void copy()}>
              {copied ? t("mobile.gpuTest.copied") : t("mobile.gpuTest.copy")}
            </button>
          )}
        </div>

        {report && (
          <div className="mt-5 space-y-1 rounded-md bg-raised p-4 text-label text-muted">
            <div className="text-body text-ink">
              {t("mobile.gpuTest.gpu", { name: report.adapter || "?", backend: report.backend || "?" })}
            </div>
            {report.shortOf.length === 0 ? (
              report.adapter && <div>{t("mobile.gpuTest.meets")}</div>
            ) : (
              <div className="text-danger">
                {t("mobile.gpuTest.short")}
                {report.shortOf.map((line) => (
                  <div key={line}>{line}</div>
                ))}
              </div>
            )}
            {report.fallback && <div>{t("mobile.gpuTest.fallback")}</div>}
            {report.deviceMs > 0 && (
              <div>{t("mobile.gpuTest.setup", { device: ms(report.deviceMs), shaders: ms(report.pipelineMs) })}</div>
            )}
            {report.photos.map((p) => (
              <div key={p.label} className="pt-2">
                <div className="text-ink">{t("mobile.gpuTest.photo", { label: p.label, width: p.width, height: p.height })}</div>
                <div>{t("mobile.gpuTest.open", { decode: ms(p.decodeMs), upload: ms(p.uploadMs) })}</div>
                <div>
                  {t("mobile.gpuTest.frames", {
                    width: p.frameWidth,
                    height: p.frameHeight,
                    first: ms(p.firstFrameMs),
                    median: ms(p.frameMedianMs),
                    worst: ms(p.frameWorstMs),
                  })}
                </div>
                <div>{t("mobile.gpuTest.jpeg", { ms: ms(p.jpegMs) })}</div>
                <div>{t("mobile.gpuTest.full", { ms: ms(p.fullSizeMs) })}</div>
              </div>
            ))}
            {report.error && <div className="pt-2 text-danger">{t("mobile.gpuTest.failed", { error: report.error })}</div>}
          </div>
        )}
        {error && <p className="mt-4 text-label text-danger">{error}</p>}

        {report && !report.error && (
          <div className="mt-6">
            <div className="text-body font-medium text-ink">{t("mobile.gpuTest.tryIt")}</div>
            <canvas ref={canvas} className="mt-3 h-auto w-full rounded-md bg-raised" />
            <label className="mt-3 block text-label text-muted">
              {t("mobile.gpuTest.exposure")}
              <input
                type="range"
                min={-3}
                max={3}
                step={0.05}
                value={exposure}
                onChange={(e) => setExposure(Number(e.target.value))}
                className="block min-h-12 w-full"
              />
            </label>
            <label className="block text-label text-muted">
              {t("mobile.gpuTest.warmth")}
              <input
                type="range"
                min={-1}
                max={1}
                step={0.02}
                value={warmth}
                onChange={(e) => setWarmth(Number(e.target.value))}
                className="block min-h-12 w-full"
              />
            </label>
            <div className="mt-2 flex gap-2">
              {(["jpeg", "rgba"] as const).map((m) => (
                <button
                  key={m}
                  className={`min-h-12 flex-1 ${mode === m ? "btn-solid" : "btn-ghost text-muted"}`}
                  onClick={() => setMode(m)}
                >
                  {m === "jpeg" ? t("mobile.gpuTest.asJpeg") : t("mobile.gpuTest.asPixels")}
                </button>
              ))}
            </div>
            {live && (
              <p className="mt-3 text-label text-faint">
                {t("mobile.gpuTest.live", {
                  fps: live.fps,
                  frame: ms(live.frameMs),
                  render: ms(live.renderMs),
                  encode: ms(live.encodeMs),
                })}
              </p>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
