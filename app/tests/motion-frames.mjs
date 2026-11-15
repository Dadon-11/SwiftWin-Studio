// Capture frames in the middle of the app's animations, for looking at them by eye.
// Usage (server as for ui-mock.mjs): node tests/motion-frames.mjs <out dir>
import { createRequire } from "node:module";
import path from "node:path";
import fs from "node:fs";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_PATH || "playwright");
const base = process.env.SWIFTWIN_UI_URL || "http://127.0.0.1:8787";
const out = process.argv[2] || "motion-frames";
fs.mkdirSync(out, { recursive: true });

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
await page.goto(base);
const sim = page.locator("text=Explore with a simulated iPhone");
if (await sim.isVisible().catch(() => false)) await sim.click();
await page.waitForFunction(() => document.getElementById("onboarding")?.classList.contains("hidden"));
await page.waitForTimeout(800);

const frames = async (name, act, times) => {
  await act();
  let t0 = 0;
  for (const t of times) {
    await page.waitForTimeout(t - t0);
    t0 = t;
    await page.screenshot({ path: path.join(out, `${name}-${String(t).padStart(3, "0")}ms.png`) });
  }
};
await frames("1-to-ai-mode", () => page.click("#mode-ai"), [90, 200, 600]);
await frames("2-to-code-mode", () => page.click("#mode-code"), [90, 200, 600]);
await frames("3-doctor-modal", () => page.click("#doctor-btn"), [40, 100, 400]);
await browser.close();
console.log("frames in", out);
