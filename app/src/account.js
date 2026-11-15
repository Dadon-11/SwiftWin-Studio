// Accounts and GitHub upload: the sign-in screen (shown before Home when signed out), the
// first-run welcome, the avatar menu, the Account dialog (profile, password, GitHub, sign out)
// and the Upload dialog. Everything secret stays in the Rust backend; these calls never return tokens.
import { call } from "./api.js";
import { setupDialog } from "./setup.js";
import { icon, googleLogo, hydrateIcons } from "./icons.js";
import { $, esc, toast, modal, store } from "./ui.js";
import { stagger, swapIcon, reduced } from "./motion.js";
import privacyText from "../../PRIVACY.md?raw";
import termsText from "../../TERMS.md?raw";
import { signingDialog, signingLabel } from "./signing.js";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
export const A = { status: null, avatar: null };

/** Backend errors read "invalid input: Wrong email…": show only the human part. */
const nice = (m) =>
  String(m || "")
    .replace(/^(invalid input|authentication required|HTTP error|not supported|timed out): /i, "")
    .replace(/^./, (c) => c.toUpperCase());

export async function refresh() {
  A.status = await call("account_status");
  paintAvatars();
  loadAvatar();
  return A.status;
}

/** An account is optional: only the cloud features (TestFlight release, Upload to GitHub) need one.
 *  At start the sign-in screen offers "Continue without an account", remembered on this PC.
 *  `cloud`: a cloud feature asked, so sign-in is offered even after that choice ("Not now" closes it).
 *  Resolves with the status, signed in or not. */
export async function ensureSignedIn({ cloud = false } = {}) {
  let st = await refresh();
  while (st.loading) {
    await sleep(150);
    st = await refresh();
  }
  if (st.signed_in || !st.configured) return st;
  if (!cloud && store("skipSignIn")) return st;
  return new Promise((resolve) => showSignIn(resolve, cloud));
}

// ===================== Avatar =====================

const PROVIDER = { email: "Email", google: "Google", github: "GitHub" };
const displayName = (u) => u?.name || (u?.email || "").split("@")[0] || "You";
function initials(u) {
  const parts = displayName(u).split(/[\s._-]+/).filter(Boolean);
  return ((parts[0]?.[0] || "?") + (parts.length > 1 ? parts[parts.length - 1][0] : "")).toUpperCase();
}
/** A stable colour per person, for the initials. */
const hue = (s) => [...String(s || "")].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 210);

/** The profile picture (Google / GitHub, as a data: URL from the backend) or initials. */
export function avatar(u, cls = "") {
  if (A.avatar) return `<span class="avatar ${cls}"><img src="${esc(A.avatar)}" alt="" draggable="false" /></span>`;
  return `<span class="avatar ${cls}" style="--av-h:${hue(u?.id || u?.email)}" aria-hidden="true">${esc(initials(u))}</span>`;
}

let avatarFor;
async function loadAvatar() {
  const want = (A.status?.signed_in && A.status.user?.avatar_url) || null;
  if (want === avatarFor) return;
  avatarFor = want;
  A.avatar = null;
  if (want) A.avatar = await call("account_avatar").then((r) => r.avatar, () => null);
  paintAvatars(true);
}

let painted = "";
function paintAvatars(force) {
  const u = A.status?.signed_in ? A.status.user : null;
  const key = u ? `${u.id}|${u.name}|${A.avatar ? A.avatar.length : 0}` : "";
  if (key === painted && !force) return;
  painted = key;
  for (const id of ["account-btn", "home-account", "pv-account"]) {
    const b = $(id);
    if (!b) continue;
    b.innerHTML = u ? avatar(u) : icon("user");
    b.classList.toggle("has-avatar", !!u);
    b.title = u ? `${displayName(u)} · Account` : "Account";
    b.setAttribute("aria-haspopup", "menu");
  }
}

// ===================== Sign-in screen =====================

let done = null; // resolve of ensureSignedIn
let view = "signin";
let email = store("lastEmail") || "";
let pw = ""; // kept only while "Check your email" is open, to send the email again

function showSignIn(resolve, cloud = false) {
  done = resolve;
  const skip = $("auth-skip");
  skip.textContent = cloud ? "Not now" : "Continue without an account";
  skip.onclick = () => {
    if (!cloud) store("skipSignIn", true);
    $("signin").classList.add("hidden");
    done?.(A.status);
    done = null;
  };
  email = store("lastEmail") || email;
  $("signin").classList.remove("hidden");
  render("signin");
}

function authError(msg) {
  const el = $("auth-error");
  el.innerHTML = msg ? `${icon("alert")}<span>${esc(nice(msg))}</span>` : "";
  el.classList.toggle("hidden", !msg);
}

/** Signed in: a moment of success on the button, the welcome the first time, then Home. */
async function signedIn(btn) {
  pw = "";
  await refresh();
  const u = A.status?.user || {};
  store("lastEmail", u.email);
  email = u.email || email;
  if (btn?.isConnected) {
    btn.classList.remove("busy");
    btn.classList.add("success");
    btn.innerHTML = `${icon("check")}<span>Signed in</span>`;
    await sleep(550);
  }
  if (u.id && !store(`welcomed.${u.id}`)) {
    store(`welcomed.${u.id}`, true);
    return render("welcome");
  }
  finish();
}

function finish() {
  store("skipSignIn", null);
  $("signin").classList.add("hidden");
  toast(`Signed in as ${A.status?.user?.email || "you"}`, "success");
  done?.(A.status);
  done = null;
}

/** Run `fn` with `btn` spinning; errors go to the screen's error box. */
async function busy(btn, fn) {
  authError("");
  btn.classList.add("busy");
  btn.disabled = true;
  try {
    return await fn();
  } catch (e) {
    authError(e.message);
    if (!reduced()) $("signin").querySelector(".auth-card")?.animate(
      [{ transform: "translateX(0)" }, { transform: "translateX(-6px)" }, { transform: "translateX(5px)" }, { transform: "translateX(-3px)" }, { transform: "translateX(0)" }],
      { duration: 320, easing: "ease-out" },
    );
  } finally {
    if (!btn.classList.contains("success")) {
      btn.classList.remove("busy");
      btn.disabled = false;
    }
  }
}

// ----- fields: show password, strength, caps lock, live checks (sign-in screen and Account dialog) -----

export const emailOk = (e) => /^[^\s@]+@[^\s@.]+(\.[^\s@.]+)+$/.test(e.trim()) && e.trim().length <= 254;

/** 0 (empty) … 4; anything under 8 characters is 1. */
export function strength(p) {
  if (!p) return 0;
  if (p.length < 8) return 1;
  const s = 1 + (p.length >= 12) + (/[a-z]/.test(p) && /[A-Z]/.test(p)) + (/[\d\W_]/.test(p));
  return Math.min(4, s);
}
const STRENGTH = ["", "Weak", "Fair", "Good", "Strong"];

const emailField = (id = "auth-email") => `
  <div class="field"><label for="${id}">Email</label>
    <input id="${id}" type="email" autocomplete="username" spellcheck="false" placeholder="you@example.com" value="${esc(email)}" data-check="email" />
    <small class="field-msg" data-msg="${id}"></small></div>`;

/** `kind`: "current" (just required) or "new" (8+ characters, strength meter). */
const pwField = (id, label, kind, extra = "") => `
  <div class="field"><div class="label-row"><label for="${id}">${label}</label>${extra}</div>
    <div class="pw-wrap"><input id="${id}" type="password" autocomplete="${kind === "new" ? "new-password" : "current-password"}" data-check="${kind}" />
      <button type="button" class="icon-btn pw-eye" data-eye="${id}" title="Show password" aria-label="Show password" aria-pressed="false">${icon("eye")}</button></div>
    ${kind === "new" ? `<div class="pw-meter" data-meter="${id}" data-score="0"><i></i><i></i><i></i><i></i><span></span></div>` : ""}
    <small class="field-msg" data-msg="${id}"></small></div>`;

function fieldMsg(input, text, kind = "error") {
  const m = input.closest(".field")?.querySelector(".field-msg");
  if (!m) return;
  m.textContent = text || "";
  m.className = `field-msg ${text ? kind : ""}`;
  input.classList.toggle("invalid", !!text && kind === "error");
  input.setAttribute("aria-invalid", text && kind === "error" ? "true" : "false");
}

function problem(input) {
  const v = input.value;
  switch (input.dataset.check) {
    case "email":
      return !v.trim() ? "Enter your email" : emailOk(v) ? "" : "That doesn't look like an email address";
    case "new":
      return !v ? "Choose a password" : v.length < 8 ? "Use at least 8 characters" : v.length > 72 ? "Use at most 72 characters" : "";
    case "current":
      return v ? "" : "Enter your password";
    case "code":
      return /^\d{6,10}$/.test(v.replace(/[\s-]/g, "")) ? "" : "Enter the 6-digit code from the email";
    case "name":
      return v.trim() ? "" : "Enter a name";
    default:
      return "";
  }
}

/** Check every field of `form`; show what's wrong and focus the first problem. */
function validate(form) {
  let first = null;
  for (const i of form.querySelectorAll("input[data-check]")) {
    const p = problem(i);
    i.dataset.touched = "1";
    fieldMsg(i, p);
    if (p && !first) first = i;
  }
  first?.focus();
  return !first;
}

function wireFields(root) {
  root.querySelectorAll("[data-eye]").forEach((b) => {
    b.onclick = () => {
      const i = root.querySelector("#" + b.dataset.eye);
      const show = i.type === "password";
      i.type = show ? "text" : "password";
      b.setAttribute("aria-pressed", String(show));
      b.title = b.ariaLabel = show ? "Hide password" : "Show password";
      swapIcon(b, icon(show ? "eyeOff" : "eye"));
      i.focus();
    };
  });
  root.querySelectorAll("input[data-check]").forEach((i) => {
    const meter = root.querySelector(`[data-meter="${i.id}"]`);
    // Live: once a field was left (or the form was sent), it re-checks as you type.
    i.addEventListener("blur", (e) => {
      // Leaving for another screen (or the eye button) is not "done typing": don't shift the layout under the click.
      if ((!i.value && !i.dataset.touched) || e.relatedTarget?.closest?.("[data-go], [data-provider], .pw-eye, #acct-pw-cancel")) return;
      i.dataset.touched = "1";
      fieldMsg(i, problem(i));
    });
    i.addEventListener("input", () => {
      if (meter) {
        const s = strength(i.value);
        meter.dataset.score = s;
        meter.querySelector("span").textContent = i.value.length && i.value.length < 8 ? "Too short" : STRENGTH[s];
      }
      if (i.dataset.touched) fieldMsg(i, problem(i));
    });
    if (i.type === "password") {
      const caps = (e) => {
        if (i.classList.contains("invalid") || !e.getModifierState) return;
        fieldMsg(i, e.getModifierState("CapsLock") ? "Caps Lock is on" : "", "warn");
      };
      i.addEventListener("keydown", caps);
      i.addEventListener("keyup", caps);
    }
  });
}

const providers = `
  <div class="auth-or"><span>or</span></div>
  <div class="auth-providers">
    <button type="button" class="btn lg-glass auth-provider" data-provider="google" data-glass="bend press">${googleLogo}<span>Continue with Google</span></button>
    <button type="button" class="btn lg-glass auth-provider" data-provider="github" data-glass="bend press">${icon("github")}<span>Continue with GitHub</span></button>
  </div>`;

const codeField = `<div class="field"><label for="auth-code">Code</label><input id="auth-code" class="code-input" inputmode="numeric" autocomplete="one-time-code" maxlength="10" spellcheck="false" placeholder="123456" data-check="code" />
  <small class="field-msg" data-msg="auth-code"></small></div>`;

/** The owner turned email sign-in off at the service: Google and GitHub only. */
const emailOff = () => A.status?.email === false;
const providersOnly = `
  <div class="auth-providers only">
    <button type="button" class="btn lg-glass auth-provider" data-provider="google" data-glass="bend press">${googleLogo}<span>Continue with Google</span></button>
    <button type="button" class="btn lg-glass auth-provider" data-provider="github" data-glass="bend press">${icon("github")}<span>Continue with GitHub</span></button>
  </div>
  <p class="auth-foot muted">New here? The same buttons create your account.</p>`;

const VIEWS = {
  signin: {
    title: () => (email && !emailOff() ? "Welcome back" : "Sign in"),
    sub: () =>
      emailOff()
        ? "Sign in with your Google or GitHub account to build and test SwiftUI apps on your iPhone."
        : email ? "Sign in to keep building on your iPhone." : "Sign in to build and test SwiftUI apps on your iPhone.",
    html: () => emailOff() ? providersOnly : `
      <form class="form" id="auth-form" novalidate>
        ${emailField()}
        ${pwField("auth-password", "Password", "current", `<button type="button" class="link-btn" data-go="forgot">Forgot password?</button>`)}
        <button type="submit" class="btn primary auth-submit" id="auth-submit"><span>Sign in</span></button>
      </form>${providers}
      <p class="auth-foot muted">New to SwiftWin? <button class="link-btn" data-go="signup">Create an account</button></p>`,
    submit: async (btn) => {
      email = $("auth-email").value.trim();
      await call("account_sign_in", { email, password: $("auth-password").value });
      await signedIn(btn);
    },
  },
  signup: {
    title: "Create your account",
    sub: "Free. Build, run and test SwiftUI apps on your iPhone, no Mac needed.",
    html: () => `
      <form class="form" id="auth-form" novalidate>
        ${emailField()}
        ${pwField("auth-password", "Password", "new")}
        <button type="submit" class="btn primary auth-submit" id="auth-submit"><span>Create account</span></button>
      </form>${providers}
      <p class="auth-foot muted">Already have an account? <button class="link-btn" data-go="signin">Sign in</button></p>`,
    submit: async (btn) => {
      email = $("auth-email").value.trim();
      pw = $("auth-password").value;
      const r = await call("account_sign_up", { email, password: pw });
      if (r.signed_in) return signedIn(btn);
      render("code");
      waitForLink("code");
    },
  },
  code: {
    title: "Check your email",
    sub: () => `We sent a confirmation link to ${email}. Open it on this PC and SwiftWin signs you in by itself.`,
    html: () => `
      <div class="auth-mail">${icon("mail")}<div class="meta"><b>${esc(email)}</b><span class="scanning-inline"><span class="spinner"></span> Waiting for you to open the link…</span></div></div>
      <details class="auth-alt"><summary>Got a 6-digit code instead?</summary>
      <form class="form" id="auth-form" novalidate>
        ${codeField}
        <button type="submit" class="btn primary auth-submit" id="auth-submit"><span>Confirm</span></button>
      </form></details>
      <p class="auth-foot muted">Opened it on another device? Your email is confirmed: <button class="link-btn" data-go="signin">sign in</button>.
        No email? <button type="button" class="link-btn" id="auth-resend">Send it again</button></p>`,
    submit: async (btn) => {
      await call("account_confirm", { email, code: $("auth-code").value });
      await signedIn(btn);
    },
  },
  forgot: {
    title: "Reset your password",
    sub: "Enter your account's email and we'll send you a link to choose a new password.",
    html: () => `
      <form class="form" id="auth-form" novalidate>
        ${emailField()}
        <button type="submit" class="btn primary auth-submit" id="auth-submit"><span>Send reset link</span></button>
      </form>
      <p class="auth-foot muted"><button class="link-btn" data-go="signin">${icon("arrowLeft")} Back to sign in</button></p>`,
    submit: async () => {
      email = $("auth-email").value.trim();
      await call("account_forgot", { email });
      render("reset");
      waitForLink("reset");
    },
  },
  reset: {
    title: "Check your email",
    sub: () => `If ${email} has a SwiftWin account, a password-reset link is on its way. Open it on this PC to choose a new password.`,
    html: () => `
      <div class="auth-mail">${icon("mail")}<div class="meta"><b>${esc(email)}</b><span class="scanning-inline"><span class="spinner"></span> Waiting for you to open the link…</span></div></div>
      <details class="auth-alt"><summary>Got a 6-digit code instead?</summary>
      <form class="form" id="auth-form" novalidate>
        ${codeField}
        ${pwField("auth-password", "New password", "new")}
        <button type="submit" class="btn primary auth-submit" id="auth-submit"><span>Set password and sign in</span></button>
      </form></details>
      <p class="auth-foot muted"><button class="link-btn" data-go="signin">${icon("arrowLeft")} Back to sign in</button></p>`,
    submit: async (btn) => {
      await call("account_reset", { email, code: $("auth-code").value, password: $("auth-password").value });
      await signedIn(btn);
    },
  },
  newpass: {
    title: "Choose a new password",
    sub: () => `For ${email || "your account"}.`,
    html: () => `
      <form class="form" id="auth-form" novalidate>
        ${pwField("auth-password", "New password", "new")}
        <button type="submit" class="btn primary auth-submit" id="auth-submit"><span>Save and continue</span></button>
      </form>`,
    submit: async (btn) => {
      await call("account_set_password", { password: $("auth-password").value });
      await signedIn(btn);
    },
  },
  browser: {
    title: "Continue in your browser",
    sub: "Finish signing in in the browser window that just opened, then come back here.",
    html: () => `
      <div class="scanning"><span class="spinner"></span> Waiting for the browser…</div>
      <div class="auth-row"><button type="button" class="link-btn" id="auth-copy">${icon("copy")} Didn't open? Copy the link</button><span class="grow"></span>
        <button type="button" class="btn" id="auth-cancel">Cancel</button></div>`,
  },
  welcome: {
    title: () => `Welcome to SwiftWin, ${displayName(A.status?.user).split(" ")[0]}`,
    sub: "You're all set. Here's how building an app works:",
    html: () => `
      <ol class="welcome-steps">
        <li><span class="step-dot">${icon("plus")}</span><div><b>Create a project</b><span>Start from the SwiftUI template, or open a folder you have.</span></div></li>
        <li><span class="step-dot">${icon("phone")}</span><div><b>Connect your iPhone, once</b><span>Plug it in, tap Trust, and turn on Developer Mode.</span></div></li>
        <li><span class="step-dot">${icon("play")}</span><div><b>Run it, and let Claude help</b><span>One click builds, signs and installs. Claude can see and tap the phone too.</span></div></li>
      </ol>
      <button type="button" class="btn primary auth-submit" id="welcome-go"><span>Get started</span>${icon("chevron")}</button>`,
  },
};

function render(name) {
  view = name;
  const v = VIEWS[name];
  const val = (x) => (typeof x === "function" ? x() : x);
  $("auth-title").textContent = val(v.title);
  $("auth-sub").textContent = val(v.sub);
  authError("");
  const body = $("auth-body");
  body.innerHTML = v.html();
  hydrateIcons(body);
  wireFields(body);
  stagger(body.querySelectorAll(".field, .auth-submit, .auth-or, .auth-providers, .auth-foot, .welcome-steps li, .auth-mail"), { step: 25 });
  body.querySelectorAll("[data-go]").forEach((b) => (b.onclick = () => render(b.dataset.go)));
  body.querySelectorAll("[data-provider]").forEach((b) => (b.onclick = () => browserSignIn(b.dataset.provider, b)));
  const form = $("auth-form");
  if (form) {
    form.onsubmit = (e) => {
      e.preventDefault();
      if (validate(form)) busy($("auth-submit"), () => v.submit($("auth-submit")));
    };
    const inputs = [...form.querySelectorAll("input")];
    (inputs.find((i) => !i.value) || inputs[0])?.focus();
  }
  const go = $("welcome-go");
  if (go) {
    go.onclick = finish;
    go.focus();
  }
  const resend = $("auth-resend");
  if (resend)
    resend.onclick = () =>
      busy(resend, async () => {
        if (!pw) return render("signup");
        await call("account_sign_up", { email, password: pw });
        toast("Sent it again", "success");
        waitForLink("code");
      });
}

/** Terms / Privacy Policy, shown in the app (the repo has no public page yet). */
export function legalDialog(which) {
  const body = document.createElement("div");
  body.className = "legal";
  body.textContent = which === "terms" ? termsText : privacyText;
  modal({ title: which === "terms" ? "Terms of Use" : "Privacy Policy", body, wide: true });
}
document.querySelectorAll("[data-legal]").forEach((b) => (b.onclick = () => legalDialog(b.dataset.legal)));

/** While "Check your email" is open: the email's link comes back to the app and signs in. */
async function waitForLink(name) {
  while (view === name) {
    const st = await refresh();
    if (st.signed_in) {
      pw = "";
      if (st.recovered) return render("newpass");
      return signedIn();
    }
    if (!st.pending) {
      if (st.error) authError(st.error);
      return;
    }
    await sleep(700);
  }
}

async function browserSignIn(provider, btn) {
  const r = await busy(btn, () => call("account_browser", { provider }));
  if (!r) return;
  render("browser");
  $("auth-copy").onclick = async () => {
    try {
      await navigator.clipboard.writeText(r.url);
      toast("Link copied: paste it into your browser", "success");
    } catch {
      authError(r.url);
    }
  };
  $("auth-cancel").onclick = async () => {
    await call("account_cancel");
    render("signin");
  };
  while (view === "browser") {
    const st = await refresh();
    if (st.signed_in) return signedIn();
    if (!st.pending) {
      render("signin");
      return authError(st.error || "Sign-in didn't finish. Try again.");
    }
    await sleep(500);
  }
}

// ===================== GitHub connection (device flow) =====================

/** Fill `box` with the GitHub connection state; `onChange(connected)` after each change. */
async function githubBlock(box, onChange) {
  // Offline or a failed check: show the error with Connect, instead of an empty block.
  let gh = await call("github_status").catch((e) => ({ configured: true, error: nice(e.message) }));
  const draw = () => {
    if (!gh.configured) {
      box.innerHTML = `<div class="callout">${icon("info")} Uploading to GitHub isn't set up in this build of SwiftWin.</div>`;
    } else if (gh.connected) {
      box.innerHTML = `<div class="acct-row">${icon("github")}<div class="meta"><b>Connected as @${esc(gh.login)}</b><span>SwiftWin can create repositories and upload your projects.</span></div>
        <button class="btn small ghost" id="gh-disconnect">Disconnect</button></div>`;
      box.querySelector("#gh-disconnect").onclick = async () => {
        await call("github_disconnect");
        gh = await call("github_status");
        draw();
        onChange?.(false);
        toast("GitHub disconnected. To revoke access too: GitHub › Settings › Applications.", "info", 7000);
      };
    } else if (gh.pending) {
      box.innerHTML = `<div class="gh-code">
          <span class="muted">On GitHub, enter this code:</span>
          <div class="cmd" data-glass="bend"><code id="gh-user-code">${esc(gh.pending.user_code)}</code><button class="icon-btn" id="gh-copy" title="Copy">${icon("copy")}</button></div>
          <div class="auth-row"><button class="link-btn" id="gh-open">${icon("external")} Open ${esc(gh.pending.verification_uri.replace(/^https:\/\//, ""))}</button><span class="grow"></span>
            <span class="scanning-inline"><span class="spinner"></span> Waiting for GitHub…</span>
            <button class="btn small" id="gh-cancel">Cancel</button></div></div>`;
      box.querySelector("#gh-copy").onclick = () => navigator.clipboard?.writeText(gh.pending.user_code).then(() => toast("Code copied", "success"));
      box.querySelector("#gh-open").onclick = () => call("open_url", { url: gh.pending.verification_uri }).catch((e) => toast(e.message, "error"));
      box.querySelector("#gh-cancel").onclick = async () => {
        await call("github_cancel");
        gh = await call("github_status");
        draw();
      };
    } else {
      box.innerHTML = `${gh.error ? `<div class="callout error">${esc(gh.error)}</div>` : ""}
        <div class="acct-row">${icon("github")}<div class="meta"><b>Connect GitHub</b><span>Needed to upload projects. New repositories are private.</span></div>
        <button class="btn small primary" id="gh-connect">Connect GitHub</button></div>`;
      box.querySelector("#gh-connect").onclick = async (e) => {
        e.target.classList.add("busy");
        try {
          await call("github_connect");
        } catch (err) {
          toast(err.message, "error");
        }
        gh = await call("github_status");
        draw();
        while (gh.pending && box.isConnected) {
          await sleep(1000);
          gh = await call("github_status");
          if (!gh.pending) {
            draw();
            onChange?.(gh.connected);
            if (gh.connected) toast(`Connected to GitHub as @${gh.login}`, "success");
          }
        }
      };
    }
  };
  draw();
  return gh;
}

// ===================== Avatar menu =====================

let menuEl = null;

/** Under the button, or above it when it sits low (the phone page's toolbar). */
function placeMenu(m, btn) {
  const r = btn.getBoundingClientRect();
  if (r.bottom + 8 + m.offsetHeight > window.innerHeight) m.style.bottom = `${Math.round(window.innerHeight - r.top + 8)}px`;
  else m.style.top = `${Math.round(r.bottom + 8)}px`;
  m.style.right = `${Math.max(8, Math.round(window.innerWidth - r.right))}px`;
}

/** The menu under the avatar in the title bar / on Home: who is signed in, Account, Sign out. */
export async function accountMenu(btn) {
  if (menuEl) return closeMenu();
  const st = A.status || (await refresh());
  if (!st.signed_in) return signedOutMenu(btn, st.configured);
  const u = st.user || {};
  const gh = st.github || {};
  const sg = await call("signing_status").catch(() => null);
  const m = document.createElement("div");
  m.className = "umenu";
  m.setAttribute("role", "menu");
  m.dataset.glass = "";
  m.innerHTML = `
    <div class="um-head">${avatar(u, "md")}<div class="meta"><b>${esc(displayName(u))}</b><span>${esc(u.email)}</span></div></div>
    <div class="um-sep"></div>
    <button class="um-item" role="menuitem" id="um-account">${icon("settings")}<span>Account settings</span></button>
    <button class="um-item" role="menuitem" id="um-signing">${icon("shield")}<span>Signing: ${esc(signingLabel(sg))}</span></button>
    <button class="um-item" role="menuitem" id="um-setup">${icon("settings")}<span>Set up this PC</span></button>
    <button class="um-item" role="menuitem" id="um-github">${icon("github")}<span>${gh.connected ? `GitHub: @${esc(gh.login)}` : "Connect GitHub"}</span>${gh.connected ? '<i class="um-dot"></i>' : ""}</button>
    <div class="um-sep"></div>
    <button class="um-item" role="menuitem" id="um-signout">${icon("logout")}<span>Sign out</span></button>`;
  document.body.appendChild(m);
  placeMenu(m, btn);
  menuEl = m;
  btn.setAttribute("aria-expanded", "true");
  const items = [...m.querySelectorAll(".um-item")];
  const onDown = (e) => !m.contains(e.target) && !btn.contains(e.target) && closeMenu();
  const onKey = (e) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      closeMenu();
      btn.focus();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const i = items.indexOf(document.activeElement);
      items[(i + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length].focus();
    }
  };
  document.addEventListener("mousedown", onDown, true);
  document.addEventListener("keydown", onKey, true);
  m._cleanup = () => {
    document.removeEventListener("mousedown", onDown, true);
    document.removeEventListener("keydown", onKey, true);
    btn.setAttribute("aria-expanded", "false");
  };
  requestAnimationFrame(() => m.classList.add("show"));
  items[0].focus({ preventScroll: true });
  m.querySelector("#um-account").onclick = () => (closeMenu(), accountDialog());
  m.querySelector("#um-signing").onclick = () => (closeMenu(), signingDialog());
  m.querySelector("#um-setup").onclick = () => (closeMenu(), setupDialog());
  m.querySelector("#um-github").onclick = () => (closeMenu(), accountDialog());
  m.querySelector("#um-signout").onclick = () => (closeMenu(), signOut());
}

function closeMenu() {
  const m = menuEl;
  if (!m) return;
  menuEl = null;
  m._cleanup();
  m.classList.remove("show");
  m.classList.add("closing");
  setTimeout(() => m.remove(), 200);
}

/** Signed out (or a build without accounts, `configured` false): sign in, signing, Set up this PC. */
async function signedOutMenu(btn, configured) {
  const sg = await call("signing_status").catch(() => null);
  const m = document.createElement("div");
  m.className = "umenu";
  m.setAttribute("role", "menu");
  m.dataset.glass = "";
  m.innerHTML = `
    ${configured ? `<div class="um-head"><div class="meta"><b>Not signed in</b><span>Sign in for cloud builds and GitHub upload.</span></div></div>
    <div class="um-sep"></div>
    <button class="um-item" role="menuitem" id="um-signin">${icon("user")}<span>Sign in</span></button>` : ""}
    <button class="um-item" role="menuitem" id="um-signing">${icon("shield")}<span>Signing: ${esc(signingLabel(sg))}</span></button>
    <button class="um-item" role="menuitem" id="um-setup">${icon("settings")}<span>Set up this PC</span></button>`;
  document.body.appendChild(m);
  placeMenu(m, btn);
  menuEl = m;
  btn.setAttribute("aria-expanded", "true");
  const onDown = (e) => !m.contains(e.target) && !btn.contains(e.target) && closeMenu();
  const onKey = (e) => e.key === "Escape" && (e.stopPropagation(), closeMenu(), btn.focus());
  document.addEventListener("mousedown", onDown, true);
  document.addEventListener("keydown", onKey, true);
  m._cleanup = () => {
    document.removeEventListener("mousedown", onDown, true);
    document.removeEventListener("keydown", onKey, true);
    btn.setAttribute("aria-expanded", "false");
  };
  requestAnimationFrame(() => m.classList.add("show"));
  m.querySelector(".um-item").focus({ preventScroll: true });
  if (configured) m.querySelector("#um-signin").onclick = async () => {
    closeMenu();
    const st = await ensureSignedIn({ cloud: true });
    if (st.signed_in) window.dispatchEvent(new CustomEvent("swiftwin:signed-in"));
  };
  m.querySelector("#um-signing").onclick = () => (closeMenu(), signingDialog());
  m.querySelector("#um-setup").onclick = () => (closeMenu(), setupDialog());
}

/** Sign out, then back to the sign-in screen; `before()` runs once signed out (closes a dialog). */
async function signOut(btn, before) {
  btn?.classList.add("busy");
  try {
    await call("account_sign_out");
  } catch (err) {
    btn?.classList.remove("busy");
    return toast(nice(err.message), "error");
  }
  before?.();
  toast("Signed out", "info");
  await ensureSignedIn();
  window.dispatchEvent(new CustomEvent("swiftwin:signed-in"));
}

// ===================== Account dialog =====================

const since = (iso) => {
  const d = iso && new Date(iso);
  return d && !isNaN(d) ? d.toLocaleDateString(undefined, { month: "long", year: "numeric" }) : "—";
};
const providerIcon = (p) => (p === "google" ? googleLogo : icon(p === "github" ? "github" : "mail"));

export async function accountDialog() {
  const st = await refresh();
  const body = document.createElement("div");
  if (!st.configured) {
    body.innerHTML = `<p class="muted">Accounts aren't set up in this build of SwiftWin.</p>`;
    modal({ title: "Account", body });
    return;
  }
  const u = st.user || {};
  const plan = (u.plan || "free").replace(/^./, (c) => c.toUpperCase());
  body.className = "acct";
  body.innerHTML = `
    ${st.offline ? `<div class="callout">${icon("info")} You're offline. Changes to your account need a connection.</div>` : ""}
    <div class="acct-profile">
      <span id="acct-avatar">${avatar(u, "xl")}</span>
      <div class="meta">
        <div class="acct-name-row" id="acct-name-row"><b id="acct-name">${esc(displayName(u))}</b>
          <button class="icon-btn" id="acct-edit-name" title="Edit name" aria-label="Edit name">${icon("pencil")}</button></div>
        <span id="acct-email">${esc(u.email)}</span>
      </div>
      <span class="chip ok" id="acct-plan">${esc(plan)} plan</span>
    </div>
    <div class="acct-facts">
      <div><span>Sign-in method</span><b id="acct-method">${providerIcon(u.provider)}${esc(PROVIDER[u.provider] || u.provider)}</b></div>
      <div><span>Email</span><b id="acct-verified">${u.email_confirmed ? `<i class="dot ok"></i>Verified` : `<i class="dot warn"></i>Not verified`}</b></div>
      <div><span>Member since</span><b id="acct-since">${esc(since(u.created_at))}</b></div>
    </div>
    <div class="acct-section"><div class="acct-label">Security</div><div id="acct-security"></div></div>
    <div class="acct-section"><div class="acct-label">Connected services</div><div id="acct-gh"></div></div>
    <p class="acct-note">${icon("lock")} Your sign-in is kept in Windows Credential Manager on this PC. Your password is never saved here.</p>
    <div class="acct-section"><div class="acct-label">Delete account</div><div id="acct-delete"></div></div>
    <div class="form-actions"><button class="btn" id="acct-signout">${icon("logout")}Sign out</button></div>`;
  const close = modal({ title: "Account", body });
  hydrateIcons(body);
  githubBlock(body.querySelector("#acct-gh"), () => refresh());
  nameEditor(body, u);
  securityBlock(body.querySelector("#acct-security"), u);
  deleteBlock(body.querySelector("#acct-delete"), u, close);
  body.querySelector("#acct-signout").onclick = (e) => signOut(e.currentTarget, close);
}

/** The name with a pencil; editing swaps in a field with Save / Cancel. */
function nameEditor(body, u) {
  const row = body.querySelector("#acct-name-row");
  const show = () => {
    row.innerHTML = `<b id="acct-name">${esc(displayName(A.status?.user || u))}</b>
      <button class="icon-btn" id="acct-edit-name" title="Edit name" aria-label="Edit name">${icon("pencil")}</button>`;
    row.querySelector("#acct-edit-name").onclick = edit;
  };
  const edit = () => {
    row.innerHTML = `<form class="acct-name-form" id="acct-name-form" novalidate>
        <input id="acct-name-input" maxlength="64" spellcheck="false" autocomplete="name" value="${esc(A.status?.user?.name || "")}" placeholder="Your name" data-check="name" aria-label="Display name" />
        <button type="submit" class="btn small primary" id="acct-name-save">Save</button>
        <button type="button" class="btn small ghost" id="acct-name-cancel">Cancel</button></form>`;
    const f = row.querySelector("form");
    const i = f.querySelector("input");
    i.focus();
    i.select();
    i.onkeydown = (e) => e.key === "Escape" && (e.stopPropagation(), show());
    f.querySelector("#acct-name-cancel").onclick = show;
    f.onsubmit = async (e) => {
      e.preventDefault();
      if (!i.value.trim()) return i.focus();
      const b = f.querySelector("#acct-name-save");
      b.classList.add("busy");
      b.disabled = true;
      try {
        await call("account_update_name", { name: i.value });
        await refresh();
        show();
        body.querySelector("#acct-avatar").innerHTML = avatar(A.status.user, "xl");
        toast("Name updated", "success");
        window.dispatchEvent(new CustomEvent("swiftwin:profile"));
      } catch (err) {
        toast(nice(err.message), "error");
        b.classList.remove("busy");
        b.disabled = false;
      }
    };
  };
  show();
}

/** Email accounts: change the password. Google / GitHub accounts: managed there. */
function securityBlock(box, u) {
  if (u.provider !== "email") {
    const p = PROVIDER[u.provider] || u.provider;
    box.innerHTML = `<div class="acct-row">${providerIcon(u.provider)}<div class="meta"><b>Signed in with ${esc(p)}</b>
      <span>Your password and two-step verification are managed by ${esc(p)}.</span></div></div>`;
    return;
  }
  const collapsed = () => {
    box.innerHTML = `<div class="acct-row">${icon("lock")}<div class="meta"><b>Password</b><span>Change the password you sign in with.</span></div>
      <button class="btn small" id="acct-pw-open">Change password</button></div>`;
    box.querySelector("#acct-pw-open").onclick = open;
  };
  const open = () => {
    box.innerHTML = `<form class="form acct-pw" id="acct-pw-form" novalidate>
        ${pwField("acct-current", "Current password", "current")}
        ${pwField("acct-new", "New password", "new")}
        <div class="form-actions"><button type="button" class="btn ghost" id="acct-pw-cancel">Cancel</button>
          <button type="submit" class="btn primary" id="acct-pw-save">Update password</button></div></form>`;
    wireFields(box);
    stagger(box.querySelectorAll(".field, .form-actions"), { step: 25 });
    const f = box.querySelector("form");
    f.querySelector("#acct-current").focus();
    f.querySelector("#acct-pw-cancel").onclick = collapsed;
    f.onsubmit = async (e) => {
      e.preventDefault();
      if (!validate(f)) return;
      const b = f.querySelector("#acct-pw-save");
      b.classList.add("busy");
      b.disabled = true;
      try {
        await call("account_change_password", { current: f.querySelector("#acct-current").value, password: f.querySelector("#acct-new").value });
        collapsed();
        toast("Password changed", "success");
      } catch (err) {
        const m = nice(err.message);
        const cur = f.querySelector("#acct-current");
        if (/current password/i.test(m)) fieldMsg(cur, m), cur.focus();
        else fieldMsg(f.querySelector("#acct-new"), m);
        b.classList.remove("busy");
        b.disabled = false;
      }
    };
  };
  collapsed();
}

/** Delete the account for good, after typing its email. Then back to the sign-in screen. */
function deleteBlock(box, u, close) {
  const collapsed = () => {
    box.innerHTML = `<div class="acct-row">${icon("trash")}<div class="meta"><b>Delete your SwiftWin account</b>
      <span>Removes your account from the sign-in service. Your projects on this PC stay.</span></div>
      <button class="btn small danger" id="acct-del-open">Delete account</button></div>`;
    box.querySelector("#acct-del-open").onclick = open;
  };
  const open = () => {
    box.innerHTML = `<form class="form acct-del" id="acct-del-form" novalidate>
        <div class="callout">${icon("alert")} This can't be undone. You'll be signed out, and GitHub is disconnected on this PC.</div>
        <div class="field"><label for="acct-del-confirm">Type <b>${esc(u.email)}</b> to confirm</label>
          <input id="acct-del-confirm" autocomplete="off" spellcheck="false" />
          <small class="field-msg" data-msg="acct-del-confirm"></small></div>
        <div class="form-actions"><button type="button" class="btn ghost" id="acct-del-cancel">Cancel</button>
          <button type="submit" class="btn danger" id="acct-del-go" disabled>Delete my account</button></div></form>`;
    const f = box.querySelector("form");
    const input = f.querySelector("#acct-del-confirm");
    const go = f.querySelector("#acct-del-go");
    input.focus();
    input.oninput = () => (go.disabled = input.value.trim().toLowerCase() !== (u.email || "").toLowerCase());
    f.querySelector("#acct-del-cancel").onclick = collapsed;
    f.onsubmit = async (e) => {
      e.preventDefault();
      if (go.disabled) return;
      go.classList.add("busy");
      go.disabled = true;
      try {
        await call("account_delete", { confirm: input.value });
      } catch (err) {
        go.classList.remove("busy");
        go.disabled = false;
        return fieldMsg(input, nice(err.message));
      }
      close();
      toast("Your account was deleted", "info");
      await ensureSignedIn();
      window.dispatchEvent(new CustomEvent("swiftwin:signed-in"));
    };
  };
  collapsed();
}

// ===================== Upload dialog =====================

const kb = (n) => (n > 1024 * 1024 ? `${(n / 1024 / 1024).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`);

/** `hasProject` false → ask to open one first. */
export async function uploadDialog(hasProject, saveAll) {
  if (!hasProject) return toast("Open a project to upload it", "info");
  if (A.status?.configured && !A.status?.signed_in) {
    toast("Uploading to GitHub needs a SwiftWin account", "info");
    if (!(await ensureSignedIn({ cloud: true })).signed_in) return;
  }
  if (saveAll && !(await saveAll())) return;
  const body = document.createElement("div");
  body.innerHTML = `<div class="scanning"><span class="spinner"></span> Reading the project…</div>`;
  modal({ title: "Upload to GitHub", body });
  let p;
  try {
    p = await call("upload_preview");
  } catch (e) {
    body.innerHTML = `<div class="callout error">${esc(e.message)}</div>`;
    return;
  }
  const skipped = p.skipped.length
    ? `<details class="up-skipped"><summary>${p.skipped.length} file${p.skipped.length > 1 ? "s" : ""} won't be uploaded</summary>
        <ul>${p.skipped.map((s) => `<li><code>${esc(s.path)}</code> <span class="dim">${esc(s.reason)}</span></li>`).join("")}</ul></details>`
    : "";
  body.innerHTML = `
    <div id="up-gh" class="acct-section"></div>
    <form class="form" id="up-form">
      ${
        p.repo
          ? `<div class="acct-row">${icon("github")}<div class="meta"><b>${esc(p.repo)}</b><span>Uploads update this repository.</span></div>
              <button type="button" class="btn small ghost" id="up-open-repo">${icon("external")}Open</button></div>`
          : `<div class="field"><label for="up-name">New repository</label><input id="up-name" value="${esc(p.default_name)}" spellcheck="false" />
              <small>Created on your GitHub account the first time you upload.</small></div>
             <label class="check-row"><input type="checkbox" id="up-private" checked /> Private repository <span class="dim">(only you can see it)</span></label>`
      }
      <div class="field"><label for="up-msg">Description of the changes</label><input id="up-msg" placeholder="Upload from SwiftWin" spellcheck="true" /></div>
      <div class="up-summary muted">${icon("file")} ${p.files} file${p.files === 1 ? "" : "s"} · ${kb(p.bytes)} <span class="dim">· build output, secrets and signing files are never uploaded</span></div>
      ${skipped}
      <div id="up-error" class="callout error hidden"></div>
      <div id="up-progress" class="up-progress hidden"><div class="up-step" id="up-step"></div><div class="up-bar"><div id="up-fill"></div></div></div>
      <div id="up-done" class="hidden"></div>
      <div class="form-actions"><button type="submit" class="btn primary" id="up-go">${icon("upload")}Upload</button></div>
    </form>`;
  const go = body.querySelector("#up-go");
  const setConnected = (c) => (go.disabled = !c);
  const gh = await githubBlock(body.querySelector("#up-gh"), setConnected);
  setConnected(gh.connected);
  body.querySelector("#up-open-repo")?.addEventListener("click", () => call("open_url", { url: `https://github.com/${p.repo}` }).catch((e) => toast(e.message, "error")));
  body.querySelector("#up-form").onsubmit = async (e) => {
    e.preventDefault();
    const err = body.querySelector("#up-error");
    const prog = body.querySelector("#up-progress");
    const doneBox = body.querySelector("#up-done");
    err.classList.add("hidden");
    doneBox.classList.add("hidden");
    go.classList.add("busy");
    go.disabled = true;
    prog.classList.remove("hidden");
    try {
      await call("upload_start", {
        name: body.querySelector("#up-name")?.value.trim() || "",
        public: body.querySelector("#up-private") ? !body.querySelector("#up-private").checked : false,
        message: body.querySelector("#up-msg").value.trim(),
      });
      let j;
      for (;;) {
        j = await call("upload_status");
        body.querySelector("#up-step").textContent = j.step || "";
        body.querySelector("#up-fill").style.width = j.total ? `${Math.round((100 * j.done) / j.total)}%` : "8%";
        if (!j.running) break;
        await sleep(300);
      }
      if (j.error) throw new Error(j.error);
      const o = j.result;
      prog.classList.add("hidden");
      doneBox.className = "up-done";
      doneBox.innerHTML = `${icon("check")}<div class="meta"><b>${o.commit ? (o.created ? "Created " : "Uploaded to ") : "Already up to date: "}${esc(o.repo)}</b>
        <span>${o.commit ? `${o.uploaded} new or changed file${o.uploaded === 1 ? "" : "s"}` : "Nothing changed since the last upload"}</span></div>
        <button type="button" class="btn small" id="up-view">${icon("external")}View on GitHub</button>`;
      doneBox.querySelector("#up-view").onclick = () => call("open_url", { url: o.url }).catch((e2) => toast(e2.message, "error"));
      toast(o.commit ? `Uploaded to ${o.repo}` : `${o.repo} is up to date`, "success");
      go.innerHTML = `${icon("upload")}Upload again`;
    } catch (ex) {
      prog.classList.add("hidden");
      err.textContent = ex.message;
      err.classList.remove("hidden");
    } finally {
      go.classList.remove("busy");
      go.disabled = false;
    }
  };
}
