// WebDriverAgent setup: the helper app on the iPhone that lets SwiftWin tap, swipe and type.
// Backend: `wda_status`, `wda_install` + `wda_job` (= `swiftwin wda prepare`), `wda_check`.
import { call } from "./api.js";
import { icon } from "./icons.js";
import { esc, toast, modal } from "./ui.js";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let open = false;

/** Does this error mean WebDriverAgent must be (re)installed? */
export function needsWda(message) {
  return /WebDriverAgent/.test(message) && /not installed|expired/.test(message);
}

/** The dialog. `reason`: why it opened (an error from a tap, say). */
export function wdaDialog({ reason = "" } = {}) {
  if (open) return;
  open = true;
  const body = document.createElement("div");
  body.className = "wda";
  body.innerHTML = `
    ${reason ? `<div class="callout error">${esc(reason)}</div>` : ""}
    <p><b>WebDriverAgent</b> is a small helper app that lets SwiftWin control your iPhone from the PC:
      taps, swipes, typing and reading the screen. SwiftWin downloads it, signs it with your Apple account and installs it.</p>
    <p class="muted">With a free Apple ID it counts as one of your <b>3 apps</b> on the phone, uses App IDs from your
      <b>10 a week</b>, and stops after <b>7 days</b>: SwiftWin offers to install it again when that happens.</p>
    <div class="check hidden" id="wda-msg" role="status"></div>
    <div class="form-actions" id="wda-install-row"><button class="btn primary" id="wda-install">${icon("phone")}Install WebDriverAgent</button></div>
    <div class="hidden" id="wda-trust">
      <p><b>Trust it on the iPhone</b> (once per Apple account):</p>
      <ol class="sg-steps">
        <li>Open <b>Settings › General › VPN &amp; Device Management</b>.</li>
        <li>Under <b>Developer App</b>, tap your Apple ID, then <b>Trust</b>.</li>
        <li>Unlock the phone and come back here.</li>
      </ol>
      <div class="form-actions"><button class="btn primary" id="wda-check">${icon("check")}Check it starts</button></div>
    </div>`;
  const close = modal({ title: "Phone control", body, wide: true, onClose: () => (open = false) });
  const q = (s) => body.querySelector(s);
  const msg = (kind, text) => {
    q("#wda-msg").innerHTML = `<span class="badge ${kind}">${kind === "WARN" ? "…" : kind}</span><div>${esc(text)}</div>`;
    q("#wda-msg").classList.remove("hidden");
  };
  q("#wda-install").onclick = async () => {
    const btn = q("#wda-install");
    btn.classList.add("busy");
    btn.disabled = true;
    try {
      await call("wda_install");
      let j;
      while ((j = await call("wda_job")).running) {
        msg("WARN", j.step);
        await sleep(400);
      }
      if (j.error) return msg("FAIL", j.error);
      msg("PASS", `Installed ${j.result.bundle_id}.`);
      q("#wda-install-row").classList.add("hidden");
      q("#wda-trust").classList.remove("hidden");
    } catch (e) {
      msg("FAIL", e.message);
    } finally {
      btn.classList.remove("busy");
      btn.disabled = false;
    }
  };
  q("#wda-check").onclick = async () => {
    const btn = q("#wda-check");
    btn.classList.add("busy");
    btn.disabled = true;
    msg("WARN", "Starting WebDriverAgent on the phone…");
    try {
      const r = await call("wda_check");
      if (!r.ok) return msg("FAIL", `${r.error} Trusted it already? Unlock the phone and check again.`);
      msg("PASS", "WebDriverAgent started: SwiftWin can control your iPhone.");
      toast("Phone control is ready", "success");
      setTimeout(close, 900);
    } catch (e) {
      msg("FAIL", e.message);
    } finally {
      btn.classList.remove("busy");
      btn.disabled = false;
    }
  };
}

/** After connecting or signing in: open the dialog when the runner is missing. `always`: after a
 *  new Apple sign-in, which needs a freshly signed runner. */
export async function maybeSetUpWda({ always = false } = {}) {
  try {
    const st = await call("status");
    if (!st.connected) return;
    if (always || !(await call("wda_status")).installed) wdaDialog();
  } catch {
    /* informational */
  }
}
