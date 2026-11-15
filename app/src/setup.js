// "Set up this PC": installs what SwiftWin needs (Apple's USB service, Python, Swift, LLVM, helper
// tools) and gets the iOS SDK through the user's own Apple sign-in. Backend: `setup_*` commands
// (swiftwin-app-core/src/setup.rs); the Apple sign-in window is opened by the desktop shell.
import { call } from "./api.js";
import { icon } from "./icons.js";
import { esc, toast, modal } from "./ui.js";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const BADGE = { done: ["PASS", "DONE"], missing: ["SKIP", "TO DO"], working: ["WARN", "…"], failed: ["FAIL", "FAIL"], action: ["WARN", "NEXT"] };
let open = false;

const gb = (b) => (b / 1e9).toFixed(1);

function sdkLine(sdk) {
  if (sdk?.running) return sdk.total ? `${sdk.phase}: ${gb(sdk.done)} of ${gb(sdk.total)} GB` : sdk.done ? `${sdk.phase}: ${gb(sdk.done)} GB` : sdk.phase;
  return "";
}

function row(it, sdk) {
  let state = it.state;
  let detail = it.detail;
  if (it.id === "sdk" && sdk?.running) [state, detail] = ["working", sdkLine(sdk)];
  if (it.id === "sdk" && sdk?.error && state !== "done") [state, detail] = ["failed", sdk.error];
  const [cls, label] = BADGE[state] || BADGE.missing;
  const pct = it.id === "sdk" && sdk?.running && sdk.total ? Math.round((100 * sdk.done) / sdk.total) : null;
  const signin = it.id === "sdk" && state !== "done" && state !== "working";
  return `<div class="check" data-id="${it.id}"><span class="badge ${cls}">${label}</span><div><b>${esc(it.title)}</b>
    <div class="detail">${esc(detail)}</div>
    ${pct !== null ? `<div class="setup-bar"><i style="width:${pct}%"></i></div>` : ""}
    ${it.fix && (state === "failed" || state === "action") ? `<div class="fix">${esc(it.fix)}</div>` : ""}
    ${signin ? `<div class="form-actions"><button class="btn small" id="setup-signin">${icon("user")}Sign in with your Apple ID</button></div>` : ""}
  </div></div>`;
}

/** The Setup dialog. `first`: opened at start because something is missing. */
export async function setupDialog({ first = false } = {}) {
  if (open) return;
  open = true;
  const body = document.createElement("div");
  body.className = "setup";
  body.innerHTML = `
    <p class="muted">${first ? "Welcome! A few things to install before SwiftWin can build for your iPhone. " : ""}SwiftWin downloads each
      one from its official source (Apple, swift.org, python.org, LLVM) with Windows' own package manager. Nothing of Apple's comes
      with SwiftWin. Windows may ask for permission, and Apple's USB service may need a restart.</p>
    <div id="setup-list"><div class="scanning"><span class="spinner"></span> Checking this PC…</div></div>
    <div class="form-actions"><button class="btn primary" id="setup-run">${icon("check")}Set up everything</button></div>`;
  let alive = true;
  const close = modal({ title: "Set up this PC", body, wide: true, onClose: () => ((open = false), (alive = false)) });
  const q = (s) => body.querySelector(s);
  let last = null;

  const render = (st) => {
    last = st;
    const items = st.job.items || [];
    q("#setup-list").innerHTML = items.map((it) => row(it, st.sdk)).join("");
    const busy = st.job.running || st.sdk?.running;
    const done = items.length && items.every((i) => i.state === "done");
    q("#setup-run").disabled = !!busy;
    q("#setup-run").classList.toggle("busy", !!busy);
    q("#setup-run").innerHTML = done ? `${icon("check")}All set` : `${icon("check")}Set up everything`;
    if (done) q("#setup-run").onclick = close;
    const b = q("#setup-signin");
    if (b) b.onclick = signin;
  };

  const signin = async () => {
    try {
      const before = JSON.stringify((await call("setup_job")).sdk);
      await call("setup_sdk_signin");
      toast("Sign in with your Apple ID in the new window. The Xcode download starts by itself, and SwiftWin keeps only the iOS SDK.", "info", 9000);
      poll(before);
    } catch (e) {
      toast(e.message, "error");
    }
  };

  let polling = false;
  /** Follow the jobs until they settle. `sdkBefore`: also wait (up to 20 minutes) for the sign-in
   *  window to start the SDK download, i.e. until the SDK state differs from this. */
  const poll = async (sdkBefore = null) => {
    if (polling) return;
    polling = true;
    const until = Date.now() + 20 * 60e3;
    try {
      let st;
      while (alive) {
        st = await call("setup_job");
        render({ ...st, job: { ...st.job, items: st.job.items || last?.job.items } });
        const waiting = sdkBefore !== null && JSON.stringify(st.sdk) === sdkBefore && Date.now() < until;
        if (!st.job.running && !st.sdk?.running && !waiting) break;
        await sleep(700);
      }
      if (!alive) return;
      // Settled: probe again, so the list shows what is really installed now.
      const fresh = await call("setup_status");
      render(fresh);
      if (st?.sdk?.done_name) toast(`iOS SDK ready: ${st.sdk.done_name}`, "success");
      return fresh;
    } catch (e) {
      toast(e.message, "error");
    } finally {
      polling = false;
    }
  };

  q("#setup-run").onclick = async () => {
    try {
      render(await call("setup_run"));
      const st = await poll();
      // Everything else is in: go straight on to the Apple sign-in for the SDK.
      const sdk = st?.job.items?.find((i) => i.id === "sdk");
      if (alive && sdk && sdk.state !== "done" && !st.sdk?.running) signin();
    } catch (e) {
      toast(e.message, "error");
    }
  };

  try {
    const st = await call("setup_status");
    render(st);
    if (st.sdk?.running) poll();
  } catch (e) {
    q("#setup-list").innerHTML = `<div class="callout error">${esc(e.message)}</div>`;
  }
}

/** At start: open the dialog when this PC is missing something (not with the simulated iPhone). */
export async function maybeFirstRunSetup() {
  try {
    const st = await call("setup_status");
    if (!st.mock && st.job.items?.some((i) => i.state !== "done")) await setupDialog({ first: true });
  } catch {
    /* informational */
  }
}
