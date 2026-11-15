// Signing setup: free Apple ID (plumesign) or a paid account's App Store Connect API key.
// Backend: `signing_*` commands (swiftwin-app-core/src/signing.rs). The .p8 is read here and sent
// to the backend once; no reply ever contains it.
import { call } from "./api.js";
import { icon } from "./icons.js";
import { esc, toast, modal } from "./ui.js";
import { maybeSetUpWda } from "./wda.js";

const ASC_KEYS = "https://appstoreconnect.apple.com/access/integrations/api";
const MEMBERSHIP = "https://developer.apple.com/account#MembershipDetailsCard";

/** One line for menus: "Free Apple ID", "Paid account (ABCDE12345)" or "Not set up". */
export function signingLabel(st) {
  if (st?.mode === "apple-id") return "Free Apple ID";
  if (st?.mode === "asc") return `Paid account${st.team_id ? ` (${st.team_id})` : ""}`;
  return "Not set up";
}

function summary(st) {
  if (st.mode === "apple-id")
    return `<b>Free Apple ID</b> ${esc(st.apple_id || "")}<div class="detail">Bundle ids start with ${esc(st.id_prefix || "?")}. ${esc(st.limits)}</div>`;
  if (st.mode === "asc")
    return `<b>Paid Apple Developer account</b><div class="detail">Team ${esc(st.team_id || "(not set)")} · key ${esc(st.asc_key_id || "(not set)")} · .p8 ${
      st.p8_stored ? "stored in the Windows Credential Manager" : "missing"
    }</div>`;
  return `<b>Not set up yet</b><div class="detail">Pick how SwiftWin signs your apps. You can switch at any time.</div>`;
}

/** The Signing dialog. `first`: shown right after connecting the phone. */
export async function signingDialog({ first = false } = {}) {
  const st = await call("signing_status");
  const body = document.createElement("div");
  body.className = "signing";
  body.innerHTML = `
    ${first ? `<p class="muted">One more step: Apple only installs signed apps. Choose the kind of Apple account you have.</p>` : ""}
    <div class="check sg-now" id="sg-now"><span class="badge ${st.mode === "none" ? "WARN" : "PASS"}">${st.mode === "none" ? "NONE" : "ON"}</span><div>${summary(st)}</div></div>
    ${st.paid_team_hint ? `<div class="callout" id="sg-hint">${icon("info")} Your Apple ID signed with team ${esc(st.paid_team_hint)}, which looks like a <b>paid</b> team.
      Switch to “Paid Apple Developer account” to drop the 7-day limit.</div>` : ""}
    <div class="seg-tabs segmented"><button class="seg" data-t="free">${icon("user")}Free Apple ID</button><button class="seg" data-t="paid">${icon("shield")}Paid Apple Developer account</button></div>
    <form class="form" data-f="free" novalidate>
      <p class="muted">Any Apple ID works. Apple's limits: apps stop launching after <b>7 days</b> (press Run again),
        <b>3 apps</b> per phone, <b>10 new App IDs</b> a week.</p>
      <div class="field"><label for="sg-email">Apple ID</label><input id="sg-email" type="email" placeholder="you@example.com" spellcheck="false" autocomplete="username" /></div>
      <div class="field"><label for="sg-password">Password</label><input id="sg-password" type="password" autocomplete="current-password" />
        <small>Sent only to Apple, through plumesign (open source). SwiftWin never stores it.</small></div>
      <div class="field hidden" id="sg-code-field"><label for="sg-code">Verification code</label><input id="sg-code" inputmode="numeric" autocomplete="one-time-code" maxlength="6" placeholder="123456" />
        <small>Apple sent a 6-digit code to your iPhone or other Apple devices.</small></div>
      <div id="sg-free-msg" class="callout hidden" role="status"></div>
      <div class="form-actions">
        <button type="submit" class="btn primary" id="sg-use-free">${icon("check")}Sign in with your Apple ID</button>
      </div>
    </form>
    <form class="form" data-f="paid">
      <p class="muted">With a paid membership apps last a year and there is no app limit. SwiftWin uses an App Store Connect API key:</p>
      <ol class="sg-steps">
        <li>Open <button type="button" class="link-btn" data-open="${ASC_KEYS}">App Store Connect › Users and Access › Integrations › Team Keys</button> and click <b>+</b>.</li>
        <li>Name it <b>SwiftWin</b>, access <b>Admin</b> (needed to create certificates and profiles), then <b>Download API Key</b> (Apple lets you download it only once).</li>
        <li>Copy the <b>Key ID</b> and the <b>Issuer ID</b> shown on that page, and your <button type="button" class="link-btn" data-open="${MEMBERSHIP}">Team ID</button>.</li>
      </ol>
      <div class="field"><label for="sg-p8">Key file (AuthKey_….p8)</label><input id="sg-p8" type="file" accept=".p8" /></div>
      <div class="field"><label for="sg-key">Key ID</label><input id="sg-key" placeholder="XXXXXXXXXX" spellcheck="false" value="${esc(st.asc_key_id || "")}" /></div>
      <div class="field"><label for="sg-issuer">Issuer ID</label><input id="sg-issuer" placeholder="00000000-0000-0000-0000-000000000000" spellcheck="false" value="${esc(st.asc_issuer_id || "")}" /></div>
      <div class="field"><label for="sg-team">Team ID</label><input id="sg-team" placeholder="ABCDE12345" spellcheck="false" value="${esc(st.team_id || "")}" /></div>
      <div id="sg-paid-msg" class="check hidden" role="status"></div>
      <div class="form-actions">
        <button type="button" class="btn" id="sg-check">Check key</button>
        <button type="submit" class="btn primary" id="sg-save">${icon("check")}Save and use</button>
      </div>
    </form>`;
  const close = modal({ title: "Signing", body, wide: true });
  const q = (s) => body.querySelector(s);
  const show = (t) => {
    body.querySelectorAll(".seg").forEach((b) => b.classList.toggle("active", b.dataset.t === t));
    body.querySelectorAll("form").forEach((f) => f.classList.toggle("hidden", f.dataset.f !== t));
  };
  body.querySelectorAll(".seg").forEach((b) => (b.onclick = () => show(b.dataset.t)));
  body.querySelectorAll("[data-open]").forEach((b) => (b.onclick = () => call("open_url", { url: b.dataset.open }).catch((e) => toast(e.message, "error"))));
  q("#sg-email").value = st.apple_id || "";
  show(st.mode === "asc" || st.paid_team_hint ? "paid" : "free");

  const busy = async (btn, fn) => {
    btn.classList.add("busy");
    btn.disabled = true;
    try {
      return await fn();
    } finally {
      btn.classList.remove("busy");
      btn.disabled = false;
    }
  };
  const freeMsg = (text, error = false) => {
    const m = q("#sg-free-msg");
    m.textContent = text;
    m.classList.toggle("error", error);
    m.classList.remove("hidden");
  };
  const done = (text) => {
    toast(text, "success", 6000);
    close();
    window.dispatchEvent(new CustomEvent("swiftwin:signing"));
  };

  q('[data-f="free"]').onsubmit = (e) => {
    e.preventDefault();
    const codeStep = !q("#sg-code-field").classList.contains("hidden");
    busy(q("#sg-use-free"), async () => {
      try {
        const args = codeStep ? { code: q("#sg-code").value } : { email: q("#sg-email").value, password: q("#sg-password").value };
        if (!codeStep) freeMsg("Signing in with Apple…");
        const r = await call("signing_apple_id", args);
        q("#sg-password").value = "";
        if (r.need_code) {
          q("#sg-code-field").classList.remove("hidden");
          q("#sg-code").focus();
          q("#sg-use-free").lastChild.textContent = "Verify";
          return freeMsg("Enter the verification code Apple sent to your devices.");
        }
        done(`Signed in. SwiftWin now signs with your free Apple ID; new projects use ${r.id_prefix}.`);
      } catch (err) {
        // A failed sign-in starts over (plumesign has quit).
        q("#sg-code-field").classList.add("hidden");
        q("#sg-code").value = "";
        q("#sg-use-free").lastChild.textContent = "Sign in with your Apple ID";
        freeMsg(err.message, true);
      }
    });
  };

  const paidMsg = (ok, text) => {
    const m = q("#sg-paid-msg");
    m.innerHTML = `<span class="badge ${ok ? "PASS" : "FAIL"}">${ok ? "PASS" : "FAIL"}</span><div>${esc(text)}</div>`;
    m.classList.remove("hidden");
  };
  const asc = async (save) => {
    const file = q("#sg-p8").files[0];
    if (!file) return paidMsg(false, "Choose the AuthKey_….p8 file you downloaded.");
    try {
      const r = await call("signing_asc", {
        p8: await file.text(),
        key_id: q("#sg-key").value,
        issuer_id: q("#sg-issuer").value,
        team_id: q("#sg-team").value,
        save,
      });
      paidMsg(true, r.message);
      if (save) done("SwiftWin now signs with your App Store Connect key. You can delete the .p8 file (keep a copy somewhere safe).");
    } catch (err) {
      paidMsg(false, err.message);
    }
  };
  // Key ID is in the file name: AuthKey_<KEYID>.p8.
  q("#sg-p8").onchange = () => {
    const m = /AuthKey_([A-Z0-9]+)\.p8$/i.exec(q("#sg-p8").files[0]?.name || "");
    if (m && !q("#sg-key").value) q("#sg-key").value = m[1];
  };
  q("#sg-check").onclick = () => busy(q("#sg-check"), () => asc(false));
  q('[data-f="paid"]').onsubmit = (e) => {
    e.preventDefault();
    busy(q("#sg-save"), () => asc(true));
  };
  return close;
}

let asked = false;
/** After connecting a phone: ask for signing (once per run) if nothing is set up yet, else
 *  make sure WebDriverAgent is on the phone. */
export async function maybeAskForSigning() {
  try {
    const st = await call("signing_status");
    if (st.mode !== "none") return maybeSetUpWda();
    if (asked) return;
    asked = true;
    await signingDialog({ first: true });
  } catch {
    /* status is informational */
  }
}

