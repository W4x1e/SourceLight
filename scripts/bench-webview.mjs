// Measures the real app's webview — WebView2 painting real thumbnails over a
// big seeded library — which the browser harness cannot.
//
// The app must already be running with a remote-debugging port and pointed at
// a seeded library; see docs/BENCHMARKS.md, "The real app at 50,000 photos".
//
//   node scripts/bench-webview.mjs [port]
//
// It attaches over CDP (playwright-core, no browser download), drives the UI
// the way a person would — wheel scrolling, the mode rail, Enter on the grid —
// and prints a Markdown table:
//   - main-thread tasks over 50 ms, the threshold where input starts to lag;
//   - frames during a scroll, and how many took longer than two vsyncs;
//   - how long the thumbnails on screen take to finish decoding after a scroll
//     or a jump, which is the WebView2 painting cost the harness never saw.

import { chromium } from "playwright-core";

const port = process.argv[2] ?? process.env.CDP_PORT ?? "9222";
const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
const page = browser.contexts()[0]?.pages()[0];
if (!page) throw new Error("no page on the debugging port — is the app running?");

const grid = page.locator('[role="grid"][aria-label="Photos"]');
await grid.waitFor({ timeout: 60_000 });

// Observers live in the page so they see what the page sees.
const install = () => page.evaluate(() => {
  const w = /** @type {any} */ (window);
  w.__bench = { long: [], frames: [], recording: false };
  new PerformanceObserver((list) => {
    for (const e of list.getEntries()) w.__bench.long.push(Math.round(e.duration));
  }).observe({ type: "longtask", buffered: true });
  let last = performance.now();
  const tick = (now) => {
    if (w.__bench.recording) w.__bench.frames.push(now - last);
    last = now;
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
});
await install();

const reset = () =>
  page.evaluate(() => {
    const b = /** @type {any} */ (window).__bench;
    b.long = [];
    b.frames = [];
  });
const longTasks = () => page.evaluate(() => /** @type {any} */ (window).__bench.long);

/** Times one interaction: from the pointer or key event that starts it to the
 *  last DOM change before the page goes quiet for 300 ms. Armed in the page
 *  first, so the clock starts at the input itself, not at Playwright's call. */
const timed = async (act) => {
  await page.evaluate(() => {
    const w = /** @type {any} */ (window);
    const st = { start: null, last: null };
    const onInput = () => {
      if (st.start === null) st.start = performance.now();
    };
    addEventListener("pointerdown", onInput, true);
    addEventListener("keydown", onInput, true);
    const mo = new MutationObserver(() => {
      if (st.start !== null) st.last = performance.now();
    });
    mo.observe(document.body, { subtree: true, childList: true, attributes: true });
    w.__armed = new Promise((resolve) => {
      const check = () => {
        const end = st.last ?? st.start;
        if (st.start !== null && performance.now() - end >= 300) {
          mo.disconnect();
          removeEventListener("pointerdown", onInput, true);
          removeEventListener("keydown", onInput, true);
          resolve(Math.round(end - st.start));
        } else setTimeout(check, 50);
      };
      setTimeout(check, 50);
    });
  });
  await act();
  return page.evaluate(() => /** @type {any} */ (window).__armed);
};

/** Time until every thumbnail inside the grid's viewport has decoded. */
const thumbsOnScreen = (limit = 15_000) =>
  page.evaluate(
    (limit) =>
      new Promise((resolve) => {
        const start = performance.now();
        const g = document.querySelector('[role="grid"][aria-label="Photos"]');
        const check = () => {
          const box = g.getBoundingClientRect();
          const imgs = [...g.querySelectorAll("img")].filter((i) => {
            const r = i.getBoundingClientRect();
            return r.bottom > box.top && r.top < box.bottom;
          });
          const pending = imgs.filter((i) => !(i.complete && i.naturalWidth > 0)).length;
          const elapsed = performance.now() - start;
          if ((imgs.length > 0 && pending === 0) || elapsed > limit)
            resolve({ ms: Math.round(elapsed), at: Math.round(performance.now()), shown: imgs.length, pending });
          else requestAnimationFrame(check);
        };
        check();
      }),
    limit,
  );

const rows = [];
const record = (action, result, long, extra = "") =>
  rows.push(`| ${action} | ${result} | ${long.length}${long.length ? ` (${long.join(", ")} ms)` : ""} | ${extra} |`);

// 1. A cold start of the page: reload, then time from navigation to the first
//    screen of thumbnails decoded. The observers are re-installed after.
await page.reload();
await grid.waitFor({ timeout: 60_000 });
const first = await thumbsOnScreen(30_000);
await install();
record(
  "Reload → first screen of thumbnails decoded",
  `${first.at} ms from navigation (${first.shown} shown${first.pending ? `, ${first.pending} still pending` : ""})`,
  await longTasks(),
);

// 2. Wheel-scroll thirty screens, as a person would.
const box = await grid.boundingBox();
await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
await reset();
await page.evaluate(() => (/** @type {any} */ (window).__bench.recording = true));
for (let i = 0; i < 30; i++) {
  for (let s = 0; s < 4; s++) {
    await page.mouse.wheel(0, box.height / 4);
    await page.waitForTimeout(16);
  }
}
await page.evaluate(() => (/** @type {any} */ (window).__bench.recording = false));
const frames = await page.evaluate(() => /** @type {any} */ (window).__bench.frames);
const sorted = [...frames].sort((a, b) => a - b);
const p = (q) => Math.round(sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))] ?? 0);
const slow = frames.filter((f) => f > 33.4).length;
const scrollLong = await longTasks();
const afterScroll = await thumbsOnScreen();
record(
  "Wheel-scroll 30 screens",
  `${frames.length} frames, median ${p(0.5)} ms, p95 ${p(0.95)} ms, ${slow} over 33 ms`,
  scrollLong,
  `thumbnails caught up ${afterScroll.ms} ms after`,
);

// 3. Jump to the far end of the library (End on the focused grid).
await grid.locator('[role="gridcell"]').first().click();
await reset();
await page.keyboard.press("End");
await page.waitForTimeout(50);
const jump = await thumbsOnScreen();
record("Jump to the last photo (End)", `thumbnails decoded in ${jump.ms} ms`, await longTasks());
await page.keyboard.press("Home");
await thumbsOnScreen();

// 4. Workspaces, from the mode rail.
const rail = (name) => page.getByRole("button", { name, exact: true }).first();
for (const [from, to] of [
  ["Browse", "Cull"],
  ["Cull", "Browse"],
  ["Browse", "People"],
  ["People", "Browse"],
]) {
  await reset();
  const ms = await timed(() => rail(to).click());
  const extra = to === "Browse" ? `thumbnails ${(await thumbsOnScreen()).ms} ms after` : "";
  record(`${from} → ${to}`, `${ms} ms`, await longTasks(), extra);
}

// 5. The Lightbox, with its filmstrip over the whole library.
await grid.locator('[role="gridcell"]').first().click();
await reset();
record("Open the Lightbox", `${await timed(() => page.keyboard.press("Enter"))} ms`, await longTasks());
await reset();
record("Close it (Escape)", `${await timed(() => page.keyboard.press("Escape"))} ms`, await longTasks());

const count = await page.evaluate(() => document.querySelector('[role="grid"]')?.getAttribute("aria-rowcount"));
console.log(`WebView2, grid of ${count} rows, window ${box.width}×${box.height} grid area\n`);
console.log("| Action | Result | Main-thread tasks over 50 ms | |");
console.log("| --- | --- | --: | --- |");
for (const r of rows) console.log(r);
await browser.close();
