// A typography pass in the real app rather than Chromium:
// every main surface at the minimum window (940×620) and the default
// (1500×940), hunting text the layout clips, with a screenshot of each.
//
// The app must already be running with a remote-debugging port and a library
// open; see docs/BENCHMARKS.md, "The real app at 50,000 photos", for launching
// a development build against a throwaway one. Throwaway, never your own: the
// screenshots are committed, and the baseline in docs/type-baseline is taken
// from the browser harness's placeholder images for exactly that reason.
//
//   node scripts/audit-webview.mjs [out-dir] [port]
//
// Develop needs a photo with a real raw behind it. Set AUDIT_RAW to the
// filename of one in the open library (a seeded library points at raws that
// do not exist): Develop is then opened on that photo, found by search.
//
// The check itself is `src/dev/clipAudit.ts` — the same one the browser harness
// runs — compiled here and injected into WebView2. It also asks WebView2 which
// installed font actually renders each role of text, which a stylesheet cannot
// tell you.

import { mkdir, readFile, writeFile } from "node:fs/promises";
import { chromium } from "playwright-core";
import { transformWithEsbuild } from "vite";

const out = process.argv[2] ?? "docs/type-baseline";
const port = process.argv[3] ?? process.env.CDP_PORT ?? "9222";
const SIZES = [
  [940, 620],
  [1500, 940],
];

const source = await readFile(new URL("../src/dev/clipAudit.ts", import.meta.url), "utf8");
const { code } = await transformWithEsbuild(source, "clipAudit.ts", {
  format: "iife",
  globalName: "__clip",
});

const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
const page = browser.contexts()[0]?.pages()[0];
if (!page) throw new Error("no page on the debugging port — is the app running?");
const cdp = await page.context().newCDPSession(page);
await cdp.send("DOM.enable");
await cdp.send("CSS.enable");
await mkdir(out, { recursive: true });

const button = (name) => page.getByRole("button", { name, exact: true }).first();
const settle = () => page.waitForTimeout(600);
const firstPhoto = () => page.locator('[role="gridcell"]').first();
const RAW = process.env.AUDIT_RAW;
const search = async (text) => {
  const box = page.getByPlaceholder(/Search filenames/);
  await box.fill(text);
  await box.press("Enter");
  await settle();
};
const openDevelop = async () => {
  if (RAW) await search(RAW);
  await firstPhoto().click();
  await button("Develop").click();
  await page.getByRole("button", { name: "Export…", exact: true }).waitFor({ timeout: 20_000 });
};
const leaveDevelop = async () => {
  await page.keyboard.press("Escape");
  await settle();
  if (RAW) await search("");
};

/** Each surface: how to get there from Browse, and how to leave again. */
const SURFACES = [
  ["browse", "Browse: grid and filter rail", async () => {}, async () => {}],
  ["browse-selected", "Browse: a photo selected", async () => firstPhoto().click(), async () => {}],
  [
    "lightbox",
    "Lightbox and filmstrip",
    async () => {
      await firstPhoto().click();
      await page.keyboard.press("Enter");
    },
    async () => page.keyboard.press("Escape"),
  ],
  [
    "cull-loupe",
    "Cull: loupe",
    async () => {
      await button("Cull").click();
      await settle();
      await button("loupe").click();
    },
    async () => button("Browse").click(),
  ],
  [
    "cull-survey",
    "Cull: survey",
    async () => {
      await button("Cull").click();
      await settle();
      await button("survey").click();
    },
    async () => {
      await button("loupe").click();
      await button("Browse").click();
    },
  ],
  ["develop", "Develop", openDevelop, leaveDevelop],
  [
    "develop-masks",
    "Develop: masking",
    async () => {
      await openDevelop();
      await page.keyboard.press("m");
    },
    async () => {
      await page.keyboard.press("m");
      await leaveDevelop();
    },
  ],
  [
    "export",
    "Export dialog",
    async () => {
      await openDevelop();
      await button("Export…").click();
    },
    async () => {
      await button("Close").click();
      await leaveDevelop();
    },
  ],
  ...["AI", "Advanced", "Export", "About"].map((tab) => [
    `settings-${tab.toLowerCase()}`,
    `Settings: ${tab}`,
    async () => {
      await button("Settings").click();
      await settle();
      await page.getByRole("tab", { name: tab, exact: true }).click();
    },
    async () => page.keyboard.press("Escape"),
  ]),
  [
    "shortcuts",
    "Keyboard shortcuts",
    async () => page.getByTitle("Keyboard shortcuts").first().click(),
    async () => page.keyboard.press("Escape"),
  ],
  ["places", "Places", async () => button("Places").click(), async () => button("Browse").click()],
  ["people", "People", async () => button("People").click(), async () => button("Browse").click()],
];

/** Which installed font WebView2 used for the first element matching each
 *  selector — the one thing a stylesheet cannot say. */
const renderedFonts = async () => {
  const roles = {
    "UI text": "button",
    "section label": ".section-label",
    "display heading": ".font-display",
  };
  const result = {};
  const { root } = await cdp.send("DOM.getDocument", { depth: 0 });
  for (const [role, selector] of Object.entries(roles)) {
    const { nodeId } = await cdp.send("DOM.querySelector", { nodeId: root.nodeId, selector });
    if (!nodeId) continue;
    const { fonts } = await cdp.send("CSS.getPlatformFontsForNode", { nodeId });
    result[role] = fonts.map((f) => f.familyName).join(" + ") || "(no text)";
  }
  return result;
};

const report = [];
let fonts = null;
for (const [w, h] of SIZES) {
  await cdp.send("Emulation.setDeviceMetricsOverride", { width: w, height: h, deviceScaleFactor: 1, mobile: false });
  await button("Browse").click();
  await settle();
  for (const [id, label, enter, leave] of SURFACES) {
    let result;
    try {
      await enter();
      await settle();
      result = await page.evaluate(`(() => { ${code}; return __clip.clipAudit(); })()`);
      await page.screenshot({ path: `${out}/${id}-${w}x${h}.jpg`, type: "jpeg", quality: 80 });
      fonts ??= await renderedFonts();
    } catch (e) {
      result = { error: String(e).split("\n")[0] };
    }
    report.push({ size: `${w}×${h}`, id, label, ...result });
    try {
      await leave();
      await settle();
    } catch {
      // Put the app back in Browse however the surface was left.
      await page.keyboard.press("Escape").catch(() => {});
      await button("Browse").click().catch(() => {});
    }
  }
}
await cdp.send("Emulation.clearDeviceMetricsOverride");

const lines = [
  "| Surface | Size | Clipped | Unreachable | Ellipsised (deliberate) |",
  "| --- | --- | --: | --: | --: |",
  ...report.map((r) =>
    r.error
      ? `| ${r.label} | ${r.size} | — | — | could not open: ${r.error} |`
      : `| ${r.label} | ${r.size} | ${r.clipped.length} | ${r.unreachable.length} | ${r.ellipsised} |`,
  ),
];
const findings = report.flatMap((r) => [
  ...(r.clipped ?? []).map((c) => `- ${r.label} at ${r.size}: "${c.text}" — ${c.where}`),
  ...(r.unreachable ?? []).map((u) => `- ${r.label} at ${r.size}: control out of reach — "${u}"`),
]);
const text = [
  "# Typography in the real app",
  "",
  `Generated by \`scripts/audit-webview.mjs\` on ${new Date().toISOString().slice(0, 10)}: a release build in`,
  "WebView2, every surface at the minimum and the default window size, with the screenshot of each",
  "beside this file as the baseline for the next type change.",
  "",
  `Rendered fonts: ${Object.entries(fonts ?? {}).map(([k, v]) => `${k} → ${v}`).join("; ")}`,
  "",
  ...lines,
  "",
  findings.length ? "Findings:" : "No clipped text and no unreachable controls.",
  ...findings,
].join("\n");
await writeFile(`${out}/audit.md`, text + "\n");
console.log(text);
await browser.close();
