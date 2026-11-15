// Window-size checks for SwiftWin Studio (used by ui-mock.mjs, or run alone with
// `node app/tests/sizes.mjs [shots-dir]` against a running `swiftwin --mock ui`).
//
// At each size: no button / pill / tab text overflows or wraps inside its box, no
// control is off-screen or cut off by its panel, the page itself never scrolls, and the
// iPhone frame fits its panel.
import path from "path";

export const SIZES = [
  [800, 560],
  [1024, 700],
  [1280, 800],
  [1440, 900],
  [1920, 1080],
  [2560, 1440],
  [3840, 2160],
  [900, 1200], // narrow and tall
  [1700, 620], // wide and short
];

// Runs in the page: returns a list of problems (strings).
function audit() {
  const out = [];
  const vw = innerWidth;
  const vh = innerHeight;
  // While onboarding covers the window, only it is on screen.
  const ob = document.getElementById("onboarding");
  const root = ob && !ob.classList.contains("hidden") ? ob : document;
  const name = (el) => {
    const id = el.id ? "#" + el.id : "";
    const cls = typeof el.className === "string" && el.className ? "." + el.className.trim().split(/\s+/).slice(0, 2).join(".") : "";
    const t = (el.getAttribute("title") || el.textContent || el.value || "").trim().replace(/\s+/g, " ").slice(0, 30);
    return `${el.tagName.toLowerCase()}${id}${cls}${t ? ` "${t}"` : ""}`;
  };
  const shown = (el) => {
    if (!el.getClientRects().length) return false;
    const s = getComputedStyle(el);
    if (s.visibility === "hidden" || Number(s.opacity) === 0) return false;
    for (let p = el; p; p = p.parentElement) if (Number(getComputedStyle(p).opacity) === 0) return false;
    return true;
  };
  // Is the text inside `el` meant to be cut with an ellipsis?
  const ellipsized = (el) => {
    const s = getComputedStyle(el);
    return s.textOverflow === "ellipsis" && s.overflowX !== "visible";
  };

  if (document.documentElement.scrollWidth > vw + 1 || document.documentElement.scrollHeight > vh + 1)
    out.push(`page scrolls: ${document.documentElement.scrollWidth}x${document.documentElement.scrollHeight} in ${vw}x${vh}`);

  // 1. Text in buttons, pills, tabs, chips, selects never overflows or wraps.
  const boxes = "button, select, .pill, .chip, .count, .ptab, .tab, .seg, .phone-title, .phone-sub, #statusbar > span, .ai-presence, .cmd, .msg-tool, .recent-item, .badge";
  for (const el of root.querySelectorAll(boxes)) {
    if (!shown(el)) continue;
    // Overflowing text that the element (or its text child) cuts with an ellipsis is fine.
    const cut = ellipsized(el) || [...el.children].some((c) => ellipsized(c) && c.scrollWidth > c.clientWidth);
    if (el.scrollWidth > el.clientWidth + 1 && !cut && getComputedStyle(el).overflowX !== "auto")
      out.push(`text overflows ${name(el)}: ${el.scrollWidth} > ${el.clientWidth}`);
    for (const c of el.querySelectorAll("*")) {
      if (!shown(c) || ellipsized(c)) continue;
      const r = c.getBoundingClientRect();
      const b = el.getBoundingClientRect();
      if (r.width && (r.right > b.right + 1 || r.left < b.left - 1)) {
        out.push(`content sticks out of ${name(el)}`);
        break;
      }
    }
    // A single-line control whose label wrapped onto two lines.
    if (el.matches("button:not(.tile), .pill, .chip, .ptab, .seg, .tab, #statusbar > span")) {
      const lh = parseFloat(getComputedStyle(el).lineHeight) || 18;
      for (const c of [el, ...el.querySelectorAll("span, b")]) {
        if (!shown(c) || getComputedStyle(c).display === "block") continue;
        const rects = [...c.getClientRects()];
        const lines = new Set(rects.filter((r) => r.width > 0).map((r) => Math.round(r.top))).size;
        if (lines > 1 || (c === el && el.scrollHeight > el.clientHeight + 2 && el.clientHeight > 0 && el.clientHeight < lh * 1.8 + 20)) {
          out.push(`label wraps in ${name(el)}`);
          break;
        }
      }
    }
  }

  // 2. No control is off-screen or cut off by a panel that hides overflow.
  const controls = "button, input, select, textarea, .pill, .ptab, .tab, .phone-title, #statusbar > span, .device, .ai-presence, .cmd, .toast, .modal";
  for (const el of root.querySelectorAll(controls)) {
    if (!shown(el)) continue;
    const r = el.getBoundingClientRect();
    if (!r.width || !r.height) continue;
    // Inside a scroll container it may legitimately be scrolled away.
    let scroller = null;
    let clip = null;
    for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
      const s = getComputedStyle(p);
      if (/(auto|scroll)/.test(s.overflowX + s.overflowY) && (p.scrollHeight > p.clientHeight || p.scrollWidth > p.clientWidth)) {
        scroller = p;
        break;
      }
      if (!clip && /(hidden|clip)/.test(s.overflowX + s.overflowY) && !p.classList.contains("device-screen")) clip = p;
    }
    if (scroller) continue;
    if (r.right > vw + 1 || r.bottom > vh + 1 || r.left < -1 || r.top < -1) {
      out.push(`off-screen: ${name(el)} at ${Math.round(r.left)},${Math.round(r.top)} ${Math.round(r.width)}x${Math.round(r.height)}`);
      continue;
    }
    if (clip) {
      const c = clip.getBoundingClientRect();
      if (r.right > c.right + 1 || r.bottom > c.bottom + 1 || r.left < c.left - 1 || r.top < c.top - 1)
        out.push(`cut off by ${name(clip)}: ${name(el)}`);
    }
  }

  // 3. The iPhone keeps its shape and fits its stage.
  if (root !== document) return [...new Set(out)];
  const dev = document.getElementById("device");
  const stage = document.getElementById("phone-stage");
  if (dev && shown(dev)) {
    const d = dev.getBoundingClientRect();
    const s = stage.getBoundingClientRect();
    const scr = document.getElementById("screen");
    const ratio = parseFloat(getComputedStyle(dev).aspectRatio.split("/")[0]) || d.width / d.height;
    if (d.right > s.right + 1 || d.bottom > s.bottom + 1 || d.left < s.left - 1 || d.top < s.top - 1) out.push(`iPhone frame sticks out of its panel`);
    if (Math.abs(d.width / d.height - ratio) > 0.02) out.push(`iPhone frame is stretched: ${(d.width / d.height).toFixed(3)} vs ${ratio}`);
    if (d.height < 200) out.push(`iPhone frame is tiny: ${Math.round(d.width)}x${Math.round(d.height)}`);
    if (scr && scr.naturalWidth) {
      const sr = scr.getBoundingClientRect();
      if (Math.abs(sr.width / sr.height - scr.naturalWidth / scr.naturalHeight) > 0.05) out.push(`phone screen is cropped/stretched`);
    }
  }

  // 4. Panels don't overlap each other.
  const panels = [...document.querySelectorAll("#app > .titlebar, #sidebar, #editor-card, #bottom, #ai-connect, #phone-pane, #statusbar")].filter(shown);
  for (let i = 0; i < panels.length; i++)
    for (let j = i + 1; j < panels.length; j++) {
      const a = panels[i].getBoundingClientRect();
      const b = panels[j].getBoundingClientRect();
      const ox = Math.min(a.right, b.right) - Math.max(a.left, b.left);
      const oy = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
      if (ox > 1 && oy > 1) out.push(`panels overlap: ${name(panels[i])} and ${name(panels[j])}`);
    }
  // ...and none is squeezed to nothing.
  for (const p of panels) {
    const r = p.getBoundingClientRect();
    if (r.width < 120 || r.height < 26) out.push(`panel squeezed: ${name(p)} ${Math.round(r.width)}x${Math.round(r.height)}`);
  }
  return [...new Set(out)];
}

/** Check `state` at every size; returns the problems found (and saves a screenshot per size). */
export async function checkSizes(page, state, { shots, sizes = SIZES, prepare } = {}) {
  const problems = [];
  const orig = page.viewportSize();
  for (const [w, h] of sizes) {
    await page.setViewportSize({ width: w, height: h });
    await page.waitForTimeout(250); // layout, ResizeObserver, Monaco
    if (prepare) await prepare(w, h);
    if (shots) await page.screenshot({ path: path.join(shots, `size-${state}-${w}x${h}.png`) });
    for (const p of await page.evaluate(audit)) problems.push(`[${state} ${w}x${h}] ${p}`);
  }
  await page.setViewportSize(orig);
  await page.waitForTimeout(150);
  return problems;
}

// Stand-alone: audit the running mock UI and print every problem (exit 1 if any).
if (import.meta.url === `file:///${process.argv[1].replace(/\\/g, "/")}` || import.meta.url.endsWith(process.argv[1])) {
  const { createRequire } = await import("module");
  const require = createRequire(import.meta.url);
  const { chromium } = require(process.env.PLAYWRIGHT_PATH || "playwright");
  const shots = process.argv[2];
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  await page.goto(process.env.SWIFTWIN_UI_URL || "http://127.0.0.1:8787");
  const all = [];
  await page.waitForSelector("#signin:not(.hidden)");
  all.push(...(await checkSizes(page, "signin", { shots })));
  await page.fill("#auth-email", "demo@swiftwin.dev");
  await page.fill("#auth-password", "swiftwin123");
  await page.click("#auth-submit");
  await page.waitForSelector("#home-screen:not(.hidden)");
  all.push(...(await checkSizes(page, "home", { shots })));
  const fs = await import("fs");
  const os = await import("os");
  const path = await import("path");
  const dir = process.env.SWIFTWIN_UI_PROJECT_DIR || fs.mkdtempSync(path.join(os.tmpdir(), "swiftwin-sizes-"));
  {
    await page.click('[data-home="new"]');
    all.push(...(await checkSizes(page, "dialog", { shots })));
    await page.fill("#np-name", "Sizes" + Date.now());
    await page.fill("#np-parent", dir);
    await page.click("#np-create");
    await page.waitForSelector("#onboarding:not(.hidden)");
    all.push(...(await checkSizes(page, "onboarding", { shots })));
    await page.click("#use-mock");
    await page.waitForFunction(() => document.getElementById("screen").src.startsWith("data:image/png"));
    await page.waitForFunction(() => document.querySelector(".tab.active"));
    await page.click('.tree-row[data-file$="App.swift"]');
    await page.evaluate(() => document.getElementById("sb-msg").textContent = "Launched com.example.sizes on Mock iPhone 17 Pro (iOS 26.0) — build 4.2 s, install 1.1 s");
    all.push(...(await checkSizes(page, "code", { shots })));
  }
  await page.click("#mode-ai");
  all.push(...(await checkSizes(page, "ai", { shots })));
  await page.click("#mode-code");
  await page.click("#doctor-btn");
  await page.waitForFunction(() => document.getElementById("doctor-list")?.textContent.includes("PASS"));
  all.push(...(await checkSizes(page, "doctor", { shots })));
  await page.keyboard.press("Escape");
  await browser.close();
  console.log(all.join("\n") || "no problems");
  console.log(`${all.length} problems`);
  process.exit(all.length ? 1 : 0);
}
