import { call } from "./api.js";
import { icon, hydrateIcons } from "./icons.js";
import { $, esc, toast, modal, ago, store } from "./ui.js";
import { stagger, pulse, swapIcon, arcMove, slidingIndicator, transition, rubber } from "./motion.js";
import { A, ensureSignedIn, accountMenu, uploadDialog, accountDialog, refresh as refreshAccount } from "./account.js";
import { setupDialog, maybeFirstRunSetup } from "./setup.js";
import { inspoDialog } from "./inspo.js";
import { maybeAskForSigning, signingDialog, signingLabel } from "./signing.js";
import { maybeSetUpWda, needsWda, wdaDialog } from "./wda.js";
import { autoGlass } from "./vendor/glisse/index.ts";
import * as monaco from "monaco-editor/esm/vs/editor/editor.api";
import "monaco-editor/esm/vs/basic-languages/swift/swift.contribution";
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";

self.MonacoEnvironment = { getWorker: () => new EditorWorker() };
hydrateIcons();
// Liquid Glass (glisse) on every [data-glass] element, including toasts and dialogs added later.
autoGlass();

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const S = {
  connected: false,
  seq: 0,
  logIndex: 0,
  logs: [],
  actIndex: 0,
  activity: [],
  tabs: [], // {path, model, dirty, view}
  active: null,
  project: null,
  openDirs: new Set(["Sources"]),
  busy: false,
  frames: [],
  follow: true,
  panel: "logs",
};

function status(text, isError = false) {
  const el = $("sb-msg");
  el.textContent = text;
  el.title = text; // cut with "…" when the window is narrow
  el.className = isError ? "error" : "";
}
function fail(e) {
  const m = e?.message || String(e);
  status(m, true);
  toast(m, "error");
}

// ===================== Modes =====================

const modeIndicator = slidingIndicator(document.querySelector(".segmented"), ".seg");

/** Switch modes. With `from` (the control pressed), the new layout grows out of it. */
function setMode(mode, from) {
  if (from !== undefined && document.body.dataset.mode !== mode) {
    transition(() => applyMode(mode), from || $(mode === "ai" ? "mode-ai" : "mode-code"));
  } else applyMode(mode);
}
function applyMode(mode) {
  document.body.dataset.mode = mode;
  $("mode-code").classList.toggle("active", mode === "code");
  $("mode-ai").classList.toggle("active", mode === "ai");
  modeIndicator();
  store("mode", mode);
  if (mode === "ai") {
    $("phone-stage").focus();
  } else {
    setTimeout(() => editor.layout(), 0);
  }
}
$("mode-code").onclick = (e) => setMode("code", e.currentTarget);
$("mode-ai").onclick = (e) => setMode("ai", e.currentTarget);
$("ai-exit").onclick = (e) => setMode("code", e.currentTarget);

// ===================== Onboarding =====================

async function scanLoop() {
  if (S.scanning) return;
  S.scanning = true;
  while (!S.connected && !$("onboarding").classList.contains("hidden")) {
    try {
      renderDevices(await call("scan"));
    } catch (e) {
      $("scan-error").textContent = e.message;
      $("scan-error").classList.remove("hidden");
    }
    await sleep(1500);
  }
  S.scanning = false;
}

/** "Let's run <project> on your iPhone": shown once a project is open and no iPhone is connected. */
function showConnect() {
  if (S.connected) return;
  $("ob-project").textContent = S.project?.name || "your app";
  $("onboarding").classList.remove("hidden");
  scanLoop();
}
$("ob-skip").onclick = () => {
  S.skipConnect = true;
  $("onboarding").classList.add("hidden");
};
$("connect-phone").onclick = () => {
  S.skipConnect = false;
  showConnect();
};

// ===================== Home =====================

/** The start screen: new / open project and recent projects. */
function showHome() {
  const recent = store("recent") || [];
  const name = A.status?.user?.name || (A.status?.user?.email || "").split("@")[0];
  $("home-hello").textContent = name ? `Welcome back, ${name.split(" ")[0]}` : "Welcome back";
  $("home-recent-wrap").classList.toggle("hidden", !recent.length);
  const list = $("home-recent");
  list.innerHTML = "";
  for (const r of recent) {
    const b = document.createElement("button");
    b.className = "home-recent-item";
    b.dataset.glass = "bend press";
    b.innerHTML = `<span class="home-recent-icon">${icon("swift", "swift")}</span><span class="meta"><b>${esc(r.name)}</b><span>${esc(r.root)}</span></span>`;
    b.title = r.root;
    b.onclick = () => openProject(r.root);
    list.appendChild(b);
  }
  $("onboarding").classList.add("hidden");
  $("home-screen").classList.remove("hidden");
}
document.querySelectorAll("[data-home]").forEach((b) => (b.onclick = () => projectDialog(b.dataset.home === "new" ? "new" : "open")));
$("home-account").onclick = (e) => accountMenu(e.currentTarget);
$("brand-home").onclick = showHome;
window.addEventListener("swiftwin:signed-in", showHome);
window.addEventListener("swiftwin:profile", () => !$("home-screen").classList.contains("hidden") && showHome());

function setStep(id, state) {
  const li = $(id);
  li.classList.toggle("done", state === "done");
  li.classList.toggle("current", state === "current");
  li.querySelector(".step-dot").innerHTML = state === "done" ? icon("check") : li.dataset.icon || li.querySelector(".step-dot").innerHTML;
}

function renderDevices(r) {
  for (const id of ["step-usb", "step-trust", "step-dev"]) {
    const dot = $(id).querySelector(".step-dot");
    if (!$(id).dataset.icon) $(id).dataset.icon = dot.innerHTML;
  }
  const list = $("device-list");
  $("scan-error").classList.toggle("hidden", !r.error);
  if (r.error) $("scan-error").textContent = r.error;
  if (r.mock_only) {
    list.innerHTML = `<div class="scanning">${icon("info")} SWIFTWIN_MOCK is set: only the simulated iPhone is available.</div>`;
    return;
  }
  const any = r.devices.length > 0;
  const trusted = r.devices.some((d) => d.paired);
  setStep("step-usb", any ? "done" : "current");
  setStep("step-trust", trusted ? "done" : any ? "current" : "");
  setStep("step-dev", trusted ? "current" : "");
  if (!any) {
    list.innerHTML = `<div class="scanning"><span class="spinner"></span> Looking for iPhones… plug one in and unlock it.</div>`;
    return;
  }
  const shown = new Set([...list.querySelectorAll(".device-card")].map((c) => c.dataset.udid));
  list.innerHTML = "";
  const fresh = [];
  for (const d of r.devices) {
    const card = document.createElement("div");
    card.className = "device-card";
    card.dataset.udid = d.udid;
    if (!shown.has(d.udid)) fresh.push(card);
    card.dataset.glass = "bend";
    card.innerHTML = `${icon("phone")}<div class="meta"><b>iPhone</b><span>${esc(d.udid)} · ${esc(d.connection)}</span></div>
      <span class="chip ${d.paired ? "ok" : "wait"}">${d.paired ? "Trusted" : "Waiting for Trust"}</span>`;
    const b = document.createElement("button");
    b.className = "btn primary small";
    b.textContent = "Connect";
    b.disabled = !d.paired;
    b.onclick = () => doConnect({ udid: d.udid }, b);
    card.appendChild(b);
    list.appendChild(card);
  }
  stagger(fresh);
}

async function doConnect(args, btn) {
  $("connect-error").classList.add("hidden");
  btn?.classList.add("busy");
  try {
    const info = await call("connect", args);
    S.connected = true;
    $("onboarding").classList.add("hidden");
    toast(`Connected to ${info.name}`, "success");
    $("phone-stage").focus();
    if (!args.mock) maybeAskForSigning(); // a real phone: it runs only signed apps, and needs WebDriverAgent
  } catch (e) {
    $("connect-error").textContent = e.message;
    $("connect-error").classList.remove("hidden");
  } finally {
    btn?.classList.remove("busy");
  }
}
$("use-mock").onclick = () => doConnect({ mock: true });
// A new Apple sign-in needs WebDriverAgent signed with it.
window.addEventListener("swiftwin:signing", () => maybeSetUpWda({ always: true }));

// ===================== Phone: mirror + input =====================

/** Mirror sources and modes as the backend names them (swiftwin-mirror). */
const MIRROR_SOURCES = { usb: "Screenshots", stream: "Video", airplay: "AirPlay" };
const MIRROR_MODES = { auto: "auto", usbonly: "screenshots only", videoonly: "video only" };

async function frameLoop() {
  for (;;) {
    if (!S.connected) {
      await sleep(400);
      continue;
    }
    try {
      const f = await call("frame", { after: S.seq, wait_ms: 1000 });
      if (f) {
        S.seq = f.seq;
        $("screen").src = `data:${f.mime};base64,${f.data}`;
        if (f.width && f.height) $("device").style.setProperty("--screen-ratio", (f.width / f.height).toFixed(4));
        $("no-screen").classList.add("hidden");
        S.frames.push(Date.now());
      }
    } catch (e) {
      status(e.message, true);
      await sleep(1000);
    }
  }
}

// Phone input must reach the device in order (a tap that focuses a field, then the typing).
let inputChain = Promise.resolve();
function input(cmd, args) {
  const p = inputChain.then(() => call(cmd, args));
  inputChain = p.catch(() => {});
  return p.catch((e) => (needsWda(e.message) ? wdaDialog({ reason: e.message }) : fail(e)));
}

// --- visual feedback (yours in blue, Claude's in orange) ---
function fx(cls, x, y, extra = {}) {
  const el = document.createElement("div");
  el.className = cls;
  el.style.left = x * 100 + "%";
  el.style.top = y * 100 + "%";
  Object.assign(el.style, extra.style || {});
  if (extra.text) el.textContent = extra.text;
  $("fx-layer").appendChild(el);
  setTimeout(() => el.remove(), 1900);
}
function ripple(x, y, who = "") {
  fx(`ripple ${who}`, x, y);
}
function trail(x1, y1, x2, y2, who = "") {
  const layer = $("fx-layer").getBoundingClientRect();
  const dx = (x2 - x1) * layer.width;
  const dy = (y2 - y1) * layer.height;
  fx(`trail ${who}`, x1, y1, { style: { width: Math.hypot(dx, dy) + "px", transform: `rotate(${Math.atan2(dy, dx)}rad)` } });
  ripple(x2, y2, who);
}

let press = null;
const img = $("screen");
function frac(ev) {
  const r = img.getBoundingClientRect();
  return { fx: (ev.clientX - r.left) / r.width, fy: (ev.clientY - r.top) / r.height, t: performance.now() };
}
img.addEventListener("mousedown", (ev) => {
  ev.preventDefault();
  press = frac(ev);
  $("phone-stage").focus();
});
img.addEventListener("mouseup", (ev) => {
  if (!press) return;
  const a = press;
  const b = frac(ev);
  press = null;
  clearTimeout(typeTimer);
  flushTyping();
  if (Math.hypot(b.fx - a.fx, b.fy - a.fy) < 0.015) {
    ripple(a.fx, a.fy);
    input("tap", { fx: a.fx, fy: a.fy });
  } else {
    trail(a.fx, a.fy, b.fx, b.fy);
    input("swipe", { fx1: a.fx, fy1: a.fy, fx2: b.fx, fy2: b.fy, duration: Math.max(0.1, (b.t - a.t) / 1000) });
  }
});
let wheelLock = 0;
img.addEventListener("wheel", (ev) => {
  ev.preventDefault();
  if (Date.now() < wheelLock) return;
  wheelLock = Date.now() + 250;
  const dy = Math.sign(ev.deltaY) * 0.25;
  input("swipe", { fx1: 0.5, fy1: 0.5 + dy / 2, fx2: 0.5, fy2: 0.5 - dy / 2, duration: 0.15 });
}, { passive: false });

let typeBuf = "";
let typeTimer = null;
function flushTyping() {
  const text = typeBuf;
  typeBuf = "";
  if (text) input("type", { text });
}
$("phone-stage").addEventListener("keydown", (ev) => {
  if (ev.ctrlKey || ev.metaKey || ev.altKey) return;
  let k = null;
  if (ev.key.length === 1) k = ev.key;
  else if (ev.key === "Enter") k = "\n";
  else if (ev.key === "Backspace") k = "\b";
  if (k === null) return;
  ev.preventDefault();
  typeBuf += k;
  clearTimeout(typeTimer);
  typeTimer = setTimeout(flushTyping, 150);
});

$("home").onclick = () => input("press", { button: "home" });
$("toggle-tree").onclick = async () => {
  const t = $("tree");
  const btn = $("toggle-tree");
  if (!t.classList.contains("hidden")) {
    t.classList.add("hidden");
    btn.classList.remove("toggled");
    return;
  }
  try {
    t.textContent = (await call("ui_tree")).text;
    t.classList.remove("hidden");
    btn.classList.add("toggled");
  } catch (e) {
    fail(e);
  }
};
$("mirror-mode").onchange = (ev) => call("set_mirror_mode", { mode: ev.target.value }).catch(fail);
$("snap").onclick = () => {
  if (!img.src) return;
  const a = document.createElement("a");
  a.href = img.src;
  a.download = `iphone-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.png`;
  a.click();
  toast("Screenshot saved to Downloads", "success");
};
// Each "connect your AI agent" row copies its own command.
document.querySelectorAll(".cmd [data-copy]").forEach((b) => {
  b.onclick = async () => {
    try {
      await navigator.clipboard.writeText($(b.dataset.copy).textContent);
      swapIcon(b, icon("check"));
      pulse(b);
      setTimeout(() => swapIcon(b, icon("copy")), 1400);
      toast("Copied – paste it in a terminal", "success");
    } catch {
      toast("Select the command and copy it with Ctrl+C", "info");
    }
  };
});

// ===================== AI mode: the phone your AI agent drives (through `swiftwin mcp`) =====================

const M = { busy: null, cursorTimer: 0 };

// --- The agent's cursor on the phone: glides to each tap, and along each swipe ---
function claudeCursor(x, y, x2, y2) {
  const c = $("claude-cursor");
  const at = () => ({ x: parseFloat(c.style.left) / 100 || 0.5, y: parseFloat(c.style.top) / 100 || 0.5 });
  if (c.classList.contains("off")) {
    c.getAnimations().forEach((a) => a.cancel());
    c.style.left = x * 100 + "%";
    c.style.top = y * 100 + "%";
  } else arcMove(c, at(), { x, y });
  c.classList.remove("off");
  if (x2 != null) setTimeout(() => arcMove(c, { x, y }, { x: x2, y: y2 }, 380), 460);
  clearTimeout(M.cursorTimer);
  M.cursorTimer = setTimeout(() => c.classList.add("off"), 6000);
}

// --- Over the phone: "AI agent is using this device" while it works, else its last action ---
function renderPresence() {
  const pill = $("ai-last");
  const a = S.activity[S.activity.length - 1];
  const live = !!M.busy;
  if (!live && !a) {
    pill.classList.add("hidden");
    return;
  }
  pill.classList.remove("hidden");
  pill.classList.toggle("live", live);
  pill.classList.toggle("fail", !live && !a.ok);
  $("ai-text").textContent = live ? M.busy.step || "AI agent is using this device" : a.summary;
  $("ai-when").textContent = live ? "" : ago(a.time);
}
setInterval(renderPresence, 1000);

$("tb-snap").onclick = () => $("snap").click();
$("tb-run").onclick = () => run();
$("tb-exit").onclick = (e) => setMode("code", e.currentTarget);

async function activityLoop() {
  for (;;) {
    await sleep(350);
    if (!S.connected) continue;
    let r;
    try {
      r = await call("activity_since", { index: S.actIndex });
    } catch {
      continue;
    }
    S.actIndex = r.next;
    M.busy = r.busy;
    for (const a of r.entries) {
      S.activity.push(a);
      if (a.fx != null && a.fx2 != null) {
        trail(a.fx, a.fy, a.fx2, a.fy2, "claude");
        claudeCursor(a.fx, a.fy, a.fx2, a.fy2);
      } else if (a.fx != null) {
        ripple(a.fx, a.fy, "claude");
        claudeCursor(a.fx, a.fy);
      }
    }
    if (r.entries.length) {
      renderActivity();
      const a = S.activity[S.activity.length - 1];
      const sb = $("sb-claude");
      sb.classList.remove("hidden");
      sb.querySelector("span").textContent = sb.title = `AI agent: ${a.summary}`;
    }
    renderPresence();
  }
}

function renderActivity() {
  $("act-count").textContent = S.activity.length || "";
  const list = $("activity-list");
  if (!S.activity.length) {
    list.innerHTML = `<div class="empty-panel">Nothing yet. When your AI agent (Claude Code, Codex) uses SwiftWin (tap, swipe, run…), every action shows up here and on the phone.</div>`;
    return;
  }
  list.innerHTML = S.activity
    .slice(-300)
    .map((a) => `<div class="act-row ${a.ok ? "" : "fail"}"><span class="t">${esc(a.time.slice(11, 19))}</span>${icon(a.ok ? "sparkles" : "alert")}<span class="s">${esc(a.summary)}</span></div>`)
    .join("");
  list.scrollTop = list.scrollHeight;
}

// ===================== Logs =====================

const LEVELS = { debug: 0, info: 1, notice: 2, error: 3, fault: 4 };

async function logLoop() {
  for (;;) {
    await sleep(600);
    if (!S.connected) continue;
    try {
      const r = await call("logs_since", { index: S.logIndex });
      S.logIndex = r.next;
      if (r.entries.length) {
        S.logs.push(...r.entries);
        if (S.logs.length > 4000) S.logs.splice(0, S.logs.length - 4000);
        renderLogs();
      }
    } catch {
      /* not connected */
    }
  }
}

function renderLogs() {
  const f = $("log-filter").value.toLowerCase();
  const min = { all: 0, notice: 2, error: 3 }[$("log-level").value];
  const rows = S.logs.filter(
    (e) => (LEVELS[e.level] ?? 1) >= min && (!f || e.process.toLowerCase().includes(f) || e.message.toLowerCase().includes(f)),
  );
  $("log-count").textContent = S.logs.length || "";
  const list = $("log-list");
  if (!rows.length) {
    list.innerHTML = `<div class="empty-panel">${S.logs.length ? "No log lines match the filter." : "Device logs appear here once a phone is connected."}</div>`;
    return;
  }
  list.innerHTML = rows
    .slice(-800)
    .map((e) => `<div class="log-row ${esc(e.level)}"><span class="t">${esc(e.timestamp.slice(11, 23))}</span><span class="p" title="${esc(e.process)}[${e.pid}]">${esc(e.process)}</span><span class="m">${esc(e.message)}</span></div>`)
    .join("");
  if (S.follow) list.scrollTop = list.scrollHeight;
}
$("log-filter").oninput = renderLogs;
$("log-level").onchange = renderLogs;
$("log-follow").onclick = () => {
  S.follow = !S.follow;
  $("log-follow").classList.toggle("toggled", S.follow);
  if (S.follow) renderLogs();
};
$("log-clear").onclick = () => {
  S.logs = [];
  renderLogs();
};
const tabIndicator = slidingIndicator(document.querySelector(".tabs-head"), ".ptab");
for (const t of document.querySelectorAll(".ptab")) {
  t.onclick = () => {
    S.panel = t.dataset.panel;
    document.querySelectorAll(".ptab").forEach((x) => x.classList.toggle("active", x === t));
    tabIndicator();
    $("log-list").classList.toggle("hidden", S.panel !== "logs");
    $("activity-list").classList.toggle("hidden", S.panel !== "activity");
    document.querySelector(".log-tools").style.visibility = S.panel === "logs" ? "visible" : "hidden";
    if (S.panel === "activity") renderActivity();
  };
}

// ===================== Status =====================

async function statusLoop() {
  for (;;) {
    try {
      const s = await call("status");
      const title = $("phone-title");
      const dev = $("sb-device");
      if (s.connected) {
        const d = s.device;
        const dot = `<span class="dot ${s.mock ? "mock" : ""}"></span>`;
        title.innerHTML = `${dot}<span class="ell">${esc(d.name)}</span>`;
        title.title = `${d.model} · iOS ${d.ios_version}${s.mock ? " · simulated" : ""}`;
        $("phone-sub").textContent = `iOS ${d.ios_version}${s.mock ? " · simulated" : ""}`;
        dev.innerHTML = `${dot}<span class="ell">${esc(d.name)} · iOS ${esc(d.ios_version)}${s.mock ? " · simulated" : ""}</span>`;
        dev.title = `${d.name} · iOS ${d.ios_version}`;
        const now = Date.now();
        S.frames = S.frames.filter((t) => now - t < 5000);
        const src = s.mirror.active;
        const pill = $("mirror-pill");
        const fps = S.frames.length / 5;
        const name = MIRROR_SOURCES[src] || src;
        pill.textContent = `${name} · ${fps >= 1 ? fps.toFixed(0) + " fps" : "live"}`;
        pill.title =
          (src === "usb" && s.mirror.video === "stream"
            ? "USB screenshots until the phone's video stream (WebDriverAgent) runs. "
            : "") + "Frames are only sent when the screen changes";
        pill.className = `pill ${src}`;
        $("sb-mirror").textContent = `Mirror: ${name} (${MIRROR_MODES[s.mirror.mode] || s.mirror.mode})`;
        $("connect-phone").classList.add("hidden");
        $("no-screen-spin").classList.remove("hidden");
        $("no-screen-text").textContent = "Waiting for the screen…";
      } else {
        $("no-screen").classList.remove("hidden");
        $("connect-phone").classList.remove("hidden");
        $("no-screen-spin").classList.add("hidden");
        $("no-screen-text").textContent = "No iPhone connected";
        title.innerHTML = `<span class="dot off"></span>No iPhone`;
        $("phone-sub").textContent = "";
        dev.innerHTML = `<span class="dot off"></span>No iPhone`;
        $("mirror-pill").textContent = "—";
      }
      S.offline = false;
    } catch (e) {
      // In the Claude app's Browser pane the page outlives the app that served it.
      if (PHONE_VIEW && !S.offline) {
        S.offline = true;
        S.connected = false;
        $("screen").removeAttribute("src");
        $("no-screen").classList.remove("hidden");
        $("connect-phone").classList.add("hidden");
        $("no-screen-spin").classList.add("hidden");
        $("no-screen-text").textContent = e instanceof TypeError
          ? "SwiftWin isn't running. Start SwiftWin, then ask Claude to show the phone again."
          : e.message;
        $("phone-title").innerHTML = `<span class="dot off"></span>SwiftWin`;
      }
    }
    await sleep(1000);
  }
}

// ===================== Editor, tabs, file tree =====================

monaco.editor.defineTheme("swiftwin", {
  base: "vs-dark",
  inherit: true,
  rules: [],
  // Transparent, so the editor sits on its glass card.
  colors: {
    "editor.background": "#00000000",
    "editorGutter.background": "#00000000",
    "focusBorder": "#00000000",
    "editor.lineHighlightBackground": "#ffffff0a",
    "editor.lineHighlightBorder": "#00000000",
    "editorLineNumber.foreground": "#56607180",
    "editorLineNumber.activeForeground": "#b3bbc9",
    "editor.selectionBackground": "#3d8bff55",
    "editorIndentGuide.background1": "#ffffff10",
    "editorWidget.background": "#1c1f27",
    "scrollbarSlider.background": "#ffffff1f",
    "scrollbarSlider.hoverBackground": "#ffffff33",
  },
});
const editor = monaco.editor.create($("editor"), {
  model: null,
  theme: "swiftwin",
  automaticLayout: true,
  minimap: { enabled: false },
  fontSize: 14,
  fontFamily: "'Cascadia Code', Consolas, monospace",
  fontLigatures: true,
  smoothScrolling: true,
  cursorSmoothCaretAnimation: "on",
  padding: { top: 10 },
  scrollBeyondLastLine: false,
  renderLineHighlight: "all",
});

const langFor = (p) => (p.endsWith(".swift") ? "swift" : "plaintext");
const fileIcon = (p) => (p.endsWith(".swift") ? icon("swift", "swift") : icon("file"));

function updateChrome() {
  $("welcome").classList.toggle("hidden", S.tabs.length > 0);
  const dirty = S.tabs.some((t) => t.dirty);
  $("save").disabled = !S.tabs.find((t) => t.path === S.active)?.dirty;
  document.title = `${dirty ? "● " : ""}${S.project ? S.project.name + " – " : ""}SwiftWin`;
  renderTabs();
  renderTree();
}

function renderTabs() {
  const bar = $("tabs");
  bar.innerHTML = "";
  for (const t of S.tabs) {
    const el = document.createElement("div");
    el.className = `tab ${t.path === S.active ? "active" : ""} ${t.dirty ? "dirty" : ""}`;
    el.title = t.path;
    el.innerHTML = `${fileIcon(t.path)}<span>${esc(t.path.split("/").pop())}</span><span class="close" title="Close">${icon("x")}</span>`;
    el.onclick = (e) => (e.target.closest(".close") ? closeTab(t.path) : activate(t.path));
    el.onauxclick = (e) => e.button === 1 && closeTab(t.path);
    bar.appendChild(el);
  }
}

function buildTree(files) {
  const root = { dirs: {}, files: [] };
  for (const f of files) {
    const parts = f.split("/");
    let node = root;
    for (let i = 0; i < parts.length - 1; i++) {
      const key = parts.slice(0, i + 1).join("/");
      node.dirs[parts[i]] ??= { key, dirs: {}, files: [] };
      node = node.dirs[parts[i]];
    }
    node.files.push(f);
  }
  return root;
}

let treeSeen = new Set(); // tree rows already shown (new ones stagger in)
function renderTree() {
  const el = $("tree-root");
  if (!S.project) return;
  const tree = buildTree(S.project.files);
  const rows = [`<div class="proj-head"><b>${esc(S.project.name)}</b><span>${esc(S.project.bundle_id)}</span></div>`];
  const walk = (node, depth) => {
    for (const name of Object.keys(node.dirs).sort()) {
      const d = node.dirs[name];
      const open = S.openDirs.has(d.key);
      rows.push(`<div class="tree-row dir ${open ? "open" : ""}" data-dir="${esc(d.key)}" style="padding-left:${8 + depth * 14}px">${icon("chevron", "twisty")}${icon(open ? "folderOpen" : "folder", "folder")}<span class="name">${esc(name)}</span></div>`);
      if (open) walk(d, depth + 1);
    }
    for (const f of node.files.sort()) {
      const tab = S.tabs.find((t) => t.path === f);
      rows.push(`<div class="tree-row file ${f === S.active ? "active" : ""}" data-file="${esc(f)}" style="padding-left:${8 + depth * 14}px"><span class="spacer"></span>${fileIcon(f)}<span class="name">${esc(f.split("/").pop())}</span>${tab?.dirty ? '<span class="dirty-dot"></span>' : ""}</div>`);
    }
  };
  walk(tree, 0);
  el.innerHTML = rows.join("");
  const key = (r) => r.dataset.dir || r.dataset.file;
  const fresh = [...el.querySelectorAll(".tree-row")].filter((r) => !treeSeen.has(key(r)));
  treeSeen = new Set([...el.querySelectorAll(".tree-row")].map(key));
  stagger(fresh);
}
$("tree-root").addEventListener("click", (e) => {
  const row = e.target.closest(".tree-row");
  const act = e.target.closest("[data-action]");
  if (act) return action(act.dataset.action);
  if (!row) return;
  if (row.dataset.dir) {
    const k = row.dataset.dir;
    S.openDirs.has(k) ? S.openDirs.delete(k) : S.openDirs.add(k);
    renderTree();
  } else {
    openFile(row.dataset.file);
    document.body.classList.remove("explorer-open"); // small windows: the tree is a drawer
  }
});

// Small windows: the file tree is a drawer opened from the title bar.
$("explorer-btn").onclick = (e) => {
  e.stopPropagation();
  document.body.classList.toggle("explorer-open");
};
document.addEventListener("mousedown", (e) => {
  if (!document.body.classList.contains("explorer-open")) return;
  if (!e.target.closest("#sidebar, #explorer-btn")) document.body.classList.remove("explorer-open");
});

async function openFile(path) {
  if (!S.tabs.find((t) => t.path === path)) {
    try {
      const r = await call("read_file", { path });
      const tab = { path, model: monaco.editor.createModel(r.text, langFor(path)), dirty: false, view: null };
      tab.model.onDidChangeContent(() => {
        if (!tab.dirty) {
          tab.dirty = true;
          updateChrome();
        }
      });
      S.tabs.push(tab);
    } catch (e) {
      return fail(e);
    }
  }
  activate(path);
}

function activate(path) {
  const cur = S.tabs.find((t) => t.path === S.active);
  if (cur) cur.view = editor.saveViewState();
  const t = S.tabs.find((x) => x.path === path);
  if (!t) return;
  S.active = path;
  editor.setModel(t.model);
  if (t.view) editor.restoreViewState(t.view);
  editor.focus();
  updateChrome();
}

async function closeTab(path) {
  const t = S.tabs.find((x) => x.path === path);
  if (!t) return;
  if (t.dirty) {
    const choice = await confirmBox(`Save changes to ${path.split("/").pop()} before closing?`);
    if (choice === null) return; // dialog dismissed: keep the tab
    if (choice === "save") {
      try {
        await saveTab(t);
      } catch (e) {
        return fail(e);
      }
    }
  }
  const i = S.tabs.indexOf(t);
  S.tabs.splice(i, 1);
  t.model.dispose();
  if (S.active === path) {
    const next = S.tabs[i] || S.tabs[i - 1];
    S.active = null;
    if (next) activate(next.path);
    else editor.setModel(null);
  }
  updateChrome();
}

/** Resolves "save", "discard", or null when dismissed. */
function confirmBox(text) {
  return new Promise((resolve) => {
    const body = document.createElement("div");
    body.innerHTML = `<p style="margin:0 0 16px">${esc(text)}</p><div class="form-actions"><button class="btn ghost" data-v="discard">Don't save</button><button class="btn primary" data-v="save">Save</button></div>`;
    let answered = false;
    const close = modal({ title: "Unsaved changes", body, onClose: () => !answered && resolve(null) });
    body.querySelectorAll("button").forEach((b) => {
      b.onclick = () => {
        answered = true;
        resolve(b.dataset.v);
        close();
      };
    });
  });
}

async function saveTab(t) {
  await call("write_file", { path: t.path, text: t.model.getValue() });
  t.dirty = false;
}

async function save() {
  const t = S.tabs.find((x) => x.path === S.active);
  if (!t || !t.dirty) return true;
  try {
    await saveTab(t);
    status(`Saved ${t.path}`);
    updateChrome();
    return true;
  } catch (e) {
    fail(e);
    return false;
  }
}

async function saveAll() {
  try {
    for (const t of S.tabs.filter((x) => x.dirty)) await saveTab(t);
    updateChrome();
    return true;
  } catch (e) {
    fail(e);
    return false;
  }
}

/** Save or drop unsaved edits before the backend switches projects (open or new): after the
 *  switch, a save would write them into the other project. False: the user kept them open. */
async function keepEdits() {
  if (!S.project || !S.tabs.some((t) => t.dirty)) return true;
  const choice = await confirmBox(`Save changes to ${S.project.name} before opening another project?`);
  if (choice === "discard") S.tabs.forEach((t) => (t.dirty = false));
  return choice === "discard" || (choice === "save" && (await saveAll()));
}

async function openProject(path) {
  if (path !== S.project?.root && !(await keepEdits())) return false;
  try {
    const p = await call("open_project", { path });
    if (S.project?.root !== p.root) {
      treeSeen = new Set();
      for (const t of S.tabs) t.model.dispose();
      S.tabs = [];
      S.active = null;
      editor.setModel(null);
    }
    S.project = p;
    $("project-name").textContent = p.name;
    $("project-btn").title = p.root;
    const recent = (store("recent") || []).filter((r) => r.root !== p.root);
    recent.unshift({ root: p.root, name: p.name });
    store("recent", recent.slice(0, 6));
    store("project", p.root);
    renderRecent();
    $("home-screen").classList.add("hidden");
    if (!S.connected && !S.skipConnect) showConnect();
    const main = p.files.find((f) => f.endsWith("ContentView.swift")) || p.files.find((f) => f.endsWith(".swift"));
    if (main && !S.tabs.length) await openFile(main);
    updateChrome();
    status(`Opened ${p.name}`);
    return true;
  } catch (e) {
    fail(e);
    return false;
  }
}

function renderRecent() {
  const recent = store("recent") || [];
  const el = $("recent");
  el.innerHTML = recent.length ? `<div class="recent-title">Recent</div>` : "";
  for (const r of recent) {
    const b = document.createElement("button");
    b.className = "recent-item";
    b.innerHTML = `${icon("folder")}<b>${esc(r.name)}</b><span class="path">${esc(r.root)}</span>`;
    b.onclick = () => openProject(r.root);
    el.appendChild(b);
  }
}

// --- project dialog (open / new) ---
function projectDialog(tab = "open") {
  const body = document.createElement("div");
  body.innerHTML = `
    <div class="seg-tabs segmented"><button class="seg" data-t="open">${icon("folder")}Open</button><button class="seg" data-t="new">${icon("plus")}New project</button></div>
    <form class="form" data-f="open">
      <div class="field"><label for="op-path">Project folder</label><input id="op-path" placeholder="C:\\Users\\you\\SwiftWinProjects\\Hello" spellcheck="false" />
        <small>The folder that contains swiftwin.toml.</small></div>
      <div class="form-actions"><button type="submit" class="btn primary" id="op-open">Open</button></div>
    </form>
    <form class="form" data-f="new">
      <div class="field"><label for="np-name">Name</label><input id="np-name" value="MyApp" spellcheck="false" /></div>
      <div class="field"><label for="np-bundle">Bundle ID</label><input id="np-bundle" placeholder="com.example.myapp" spellcheck="false" />
        <small>Unique reverse-DNS id, e.g. com.yourname.myapp.</small></div>
      <div class="field"><label for="np-parent">Create in folder</label><input id="np-parent" spellcheck="false" /></div>
      <div class="form-actions"><button type="submit" class="btn primary" id="np-create">${icon("plus")}Create project</button></div>
    </form>`;
  const close = modal({ title: "Project", body });
  const show = (t) => {
    body.querySelectorAll(".seg").forEach((b) => b.classList.toggle("active", b.dataset.t === t));
    body.querySelectorAll("form").forEach((f) => f.classList.toggle("hidden", f.dataset.f !== t));
    body.querySelector(t === "open" ? "#op-path" : "#np-name").focus();
  };
  body.querySelectorAll(".seg").forEach((b) => (b.onclick = () => show(b.dataset.t)));
  const last = store("project") || "";
  body.querySelector("#op-path").value = last;
  body.querySelector("#np-parent").value = last ? last.replace(/[\\/][^\\/]+$/, "") : "";
  const name = body.querySelector("#np-name");
  const bundle = body.querySelector("#np-bundle");
  const suggest = () => (bundle.placeholder = `com.example.${name.value.toLowerCase().replace(/[^a-z0-9]/g, "")}`);
  name.oninput = suggest;
  suggest();
  body.querySelector('[data-f="open"]').onsubmit = async (e) => {
    e.preventDefault();
    if (await openProject(body.querySelector("#op-path").value.trim())) close();
  };
  body.querySelector('[data-f="new"]').onsubmit = async (e) => {
    e.preventDefault();
    if (!(await keepEdits())) return;
    const btn = body.querySelector("#np-create");
    btn.classList.add("busy");
    try {
      const p = await call("new_project", {
        parent: body.querySelector("#np-parent").value.trim() || ".",
        name: name.value.trim(),
        bundle_id: bundle.value.trim() || null,
      });
      close();
      await openProject(p.root);
      toast(`Created ${p.name}`, "success");
    } catch (err) {
      fail(err);
    } finally {
      btn.classList.remove("busy");
    }
  };
  show(tab);
}

// --- doctor dialog ---
async function doctorDialog() {
  const body = document.createElement("div");
  body.innerHTML = `<div class="scanning"><span class="spinner"></span> Running checks…</div>`;
  const close = modal({ title: "Doctor", body, wide: true });
  try {
    const r = await call("doctor");
    const fails = r.checks.filter((c) => c.status === "FAIL");
    const first = fails[0];
    body.innerHTML =
      `<div id="doctor-list"><div class="doctor-summary ${r.ok ? "ok" : "bad"}">${icon(r.ok ? "check" : "alert")}${
        r.ok ? "No failures – SwiftWin is ready." : `${fails.length} check(s) failed. Fix “${esc(first.title)}” first.`
      }</div><div class="form-actions"><button class="btn small" id="doctor-setup">${icon("settings")}Set up this PC</button></div>` +
      r.checks
        .map(
          (c) => `<div class="check"><span class="badge ${c.status}">${c.status}</span><div><b>${esc(c.title)}</b><div class="detail">${esc(c.detail)}</div>${
            c.fix && c.status !== "PASS" ? `<div class="fix">${esc(c.fix)}</div>` : ""
          }</div></div>`,
        )
        .join("") +
      `</div>`;
    stagger(body.querySelectorAll(".doctor-summary, .check"));
    body.querySelector("#doctor-setup").onclick = () => (close(), setupDialog());
  } catch (e) {
    body.innerHTML = `<div class="callout error">${esc(e.message)}</div>`;
  }
}

// --- build / run ---
async function busy(btnId, label, fn) {
  if (S.busy) return;
  if (!S.project) return projectDialog("new");
  S.busy = true;
  const btn = $(btnId);
  btn.classList.add("busy");
  ["run", "build"].forEach((id) => ($(id).disabled = true));
  status(label);
  try {
    if (!(await saveAll())) return;
    const r = await fn();
    status(r.message);
    pulse(btn);
    toast(r.message, "success");
    for (const w of r.warnings || []) toast(w, "info", 6000);
  } catch (e) {
    fail(e);
  } finally {
    S.busy = false;
    btn.classList.remove("busy");
    ["run", "build"].forEach((id) => ($(id).disabled = false));
  }
}
const run = () => busy("run", "Building, signing and installing…", () => call("run"));
const build = () => busy("build", "Building…", () => call("build"));
$("run").onclick = run;
$("build").onclick = build;
$("save").onclick = save;
$("project-btn").onclick = () => projectDialog(S.project ? "open" : "new");
$("doctor-btn").onclick = doctorDialog;
$("account-btn").onclick = (e) => accountMenu(e.currentTarget);
$("inspo-btn").onclick = () =>
  inspoDialog({
    ready: () => (!S.connected ? "Connect your iPhone first" : !S.project ? "Open or create a project first: Inspo saves its style guide there" : null),
    onThemeAdded: () => openProject(S.project.root),
    onApply: (prompt) => {
      setMode("ai", $("mode-ai"));
      navigator.clipboard.writeText(prompt).then(
        () => toast("Prompt copied: paste it into Claude Code or Codex", "success"),
        () => toast("Couldn't copy the prompt", "error"),
      );
    },
  });
$("upload-btn").onclick = () => uploadDialog(!!S.project, saveAll);

function action(a) {
  if (a === "new-project") projectDialog("new");
  if (a === "open-project") projectDialog("open");
  if (a === "ai-mode") setMode("ai", document.querySelector('[data-action="ai-mode"]'));
}
document.querySelectorAll("#welcome [data-action]").forEach((b) => (b.onclick = () => action(b.dataset.action)));

// ===================== Resizable panes =====================

// `current()` is the pane's real size: the CSS clamps the variable to the window, so start from what is shown.
function resizer(id, cssVar, axis, min, max, current) {
  const el = $(id);
  const saved = store(cssVar);
  if (saved) document.documentElement.style.setProperty(cssVar, saved + "px");
  el.addEventListener("mousedown", (e) => {
    e.preventDefault();
    el.classList.add("dragging");
    const start = axis === "x" ? e.clientX : e.clientY;
    const startVal = current();
    document.documentElement.classList.remove("settling");
    let raw = startVal;
    const move = (ev) => {
      raw = startVal - ((axis === "x" ? ev.clientX : ev.clientY) - start);
      const hi = max();
      const v = raw > hi ? hi + rubber(raw - hi) : raw < min ? min - rubber(min - raw) : raw;
      document.documentElement.style.setProperty(cssVar, v + "px");
      store(cssVar, Math.min(hi, Math.max(min, raw)));
    };
    const up = () => {
      const v = Math.min(max(), Math.max(min, raw));
      if (v !== raw) {
        const root = document.documentElement;
        root.classList.add("settling");
        root.style.setProperty(cssVar, v + "px");
        setTimeout(() => root.classList.remove("settling"), 360);
      }
      el.classList.remove("dragging");
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  });
}
resizer("resize-phone", "--phone-w", "x", 280, () => window.innerWidth - 400, // the CSS (--phone-col) keeps the editor visible
   () => $("phone-pane").getBoundingClientRect().width);
resizer("resize-bottom", "--bottom-h", "y", 90, () => window.innerHeight - 260, () => $("bottom").getBoundingClientRect().height);

// ===================== Keyboard shortcuts =====================

window.addEventListener("keydown", (e) => {
  const mod = e.ctrlKey || e.metaKey;
  const k = e.key.toLowerCase();
  if (mod && e.shiftKey && k === "a") {
    e.preventDefault();
    setMode(document.body.dataset.mode === "ai" ? "code" : "ai", null);
  } else if (mod && k === "s") {
    e.preventDefault();
    save();
  } else if (mod && k === "r") {
    e.preventDefault();
    run();
  } else if (mod && k === "b") {
    e.preventDefault();
    build();
  } else if (e.key === "Escape" && document.body.dataset.mode === "ai" && !document.querySelector(".modal-back")) {
    setMode("code", null);
  }
});
editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, save);
editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyR, run);
editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyB, build);
window.addEventListener("beforeunload", (e) => {
  if (S.tabs.some((t) => t.dirty)) e.preventDefault();
});
// The desktop window closes without `beforeunload`: ask there too (main window only).
if (window.__TAURI_INTERNALS__ && window.__SWIFTWIN_VIEW !== "phone")
  import("@tauri-apps/api/window").then(({ getCurrentWindow }) =>
    getCurrentWindow().onCloseRequested(async (e) => {
      if (!S.tabs.some((t) => t.dirty)) return; // not prevented: the window closes
      e.preventDefault();
      const choice = await confirmBox("Save your changes before closing SwiftWin?");
      if (choice === "discard" || (choice === "save" && (await saveAll()))) getCurrentWindow().destroy();
    }),
  );

// ===================== Start =====================

/** Phone-only window (`swiftwin-app --phone`, or Claude Code's `show_phone`): just the iPhone, for
 *  working in the Claude Code app or a terminal. No sign-in, no project: it connects by itself. */
const PHONE_VIEW = window.__SWIFTWIN_VIEW === "phone" || location.hash.startsWith("#phone");

async function phoneAutoConnect() {
  for (;;) {
    if (!S.connected) {
      try {
        const r = await call("scan");
        const d = r.devices.find((x) => x.paired);
        if (r.mock_only || d) {
          await call("connect", r.mock_only ? { mock: true } : { udid: d.udid });
          S.connected = true;
          pvStatus();
        }
      } catch {
        /* not plugged in / not trusted yet: try again */
      }
    }
    await sleep(2000);
  }
}

// --- The phone page's toolbar. Where the page has every command (the phone window), Account and
// Signing open here; on the MCP bridge (Claude's Browser pane) those commands are refused by design,
// so the buttons bring the SwiftWin Studio window forward on that page instead.
const PV = { full: false };

async function showStudio(page = "") {
  try {
    if (!(await call("show_studio", { page })).opened) toast("Open SwiftWin Studio from the Start menu", "info");
  } catch (e) {
    fail(e);
  }
}

/** Warning dot on Signing when phone control (WebDriverAgent) or signing still needs setting up. */
async function pvStatus() {
  const wda = S.connected ? await call("wda_status").catch(() => null) : null;
  const sg = PV.full ? await call("signing_status").catch(() => null) : null;
  const parts = [sg ? `Signing: ${signingLabel(sg)}` : "Signing", wda ? `Phone control: ${wda.installed ? "ready" : "not set up"}` : ""].filter(Boolean);
  const b = $("pv-setup");
  b.classList.toggle("warn", wda?.installed === false || sg?.mode === "none");
  b.title = parts.join(" · ");
  return wda;
}

function wirePhoneToolbar() {
  $("pv-studio").onclick = () => showStudio();
  $("pv-account").onclick = (e) => (PV.full ? accountMenu(e.currentTarget) : showStudio("account"));
  $("pv-setup").onclick = async () => {
    const wda = await pvStatus();
    if (wda?.installed === false) wdaDialog();
    else if (PV.full) await signingDialog();
    else showStudio("signing");
    pvStatus();
  };
  $("pv-inspo").onclick = async () => {
    const st = await call("status").catch(() => ({}));
    inspoDialog({
      ready: () => (!S.connected ? "Connect your iPhone first" : !st.project ? "Open a project in SwiftWin Studio first: Inspo saves its style guide there" : null),
      onApply: (prompt) =>
        navigator.clipboard.writeText(prompt).then(
          () => toast("Prompt copied: paste it into Claude Code or Codex", "success"),
          () => toast("Couldn't copy the prompt", "error"),
        ),
    });
  };
}

async function startPhoneView() {
  wirePhoneToolbar();
  document.body.dataset.view = "phone";
  document.title = "iPhone · SwiftWin";
  $("connect-phone").onclick = () => toast("Plug in the iPhone, unlock it and tap Trust: this window connects by itself", "info");
  try {
    S.connected = !!(await call("status")).connected;
    // Show Claude's actions from now on, not a replay of earlier ones.
    S.actIndex = (await call("activity_since", { index: 1e9 })).next;
  } catch {
    /* ignore */
  }
  // Account commands answer only where they are allowed (not on the MCP bridge).
  PV.full = await refreshAccount().then(() => true, () => false);
  pvStatus();
  frameLoop();
  activityLoop();
  statusLoop();
  phoneAutoConnect();
  $("phone-stage").focus();
}

if (PHONE_VIEW) startPhoneView();
else (async () => {
  setMode(store("mode") === "ai" ? "ai" : "code");
  stagger(document.querySelectorAll(".ob-steps > li, .welcome-actions > .tile"), { from: 80 });
  renderRecent();
  renderLogs();
  renderActivity();
  // Signed out → the sign-in screen first, with "Continue without an account" (an account is
  // needed only for cloud builds and GitHub upload).
  try {
    await ensureSignedIn();
  } catch (e) {
    fail(e);
  }
  try {
    S.connected = !!(await call("status")).connected;
  } catch {
    /* ignore */
  }
  // Start on Home (new / open a project); connecting the iPhone comes after a project is open.
  showHome();
  // A new PC: offer to install what is missing (Python, Swift, Apple's USB service, the iOS SDK…).
  maybeFirstRunSetup();
  updateChrome();
  frameLoop();
  logLoop();
  activityLoop();
  statusLoop();
})();

// The phone page asked the desktop app (`show_studio`) to show the Studio on a page.
if (!PHONE_VIEW)
  window.__swiftwinShow = (page) => {
    if (page === "account") accountDialog();
    if (page === "signing") signingDialog();
    if (page === "inspo") $("inspo-btn").click();
  };

// Updates (installed app only): one quiet check at start; the update is installed only on a click.
if (window.__TAURI_INTERNALS__ && window.__SWIFTWIN_VIEW !== "phone") {
  setTimeout(async () => {
    const u = await call("app_update_check").catch(() => null);
    if (!u?.available) return;
    const t = toast(`SwiftWin ${u.version} is available. Click to install and restart.`, "info", 0);
    t.onclick = async () => {
      t.onclick = null;
      t.querySelector("span").textContent = "Downloading the update…";
      await call("app_update_install").catch((e) => (t.remove(), toast(`Update failed: ${e.message}`, "error")));
    };
  }, 5000);
}
