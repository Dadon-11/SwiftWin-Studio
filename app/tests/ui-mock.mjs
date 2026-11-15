// UI end-to-end test against the real backend (AppCore) with the mock phone.
//
//   cargo build -p swiftwin-cli && (cd app && npm run build)
//   SWIFTWIN_HOME=/tmp/th ./target/debug/swiftwin --mock ui --port 8787 --static app/dist [--latency-ms 150] &
//   SWIFTWIN_HOME=/tmp/th SWIFTWIN_BIN=./target/debug/swiftwin SWIFTWIN_UI_PROJECT_DIR=/tmp/p node app/tests/ui-mock.mjs [shots-dir]
//
// SWIFTWIN_HOME must match the server's so `swiftwin mcp` finds the app bridge (AI mode part).
// Uses Playwright from the environment (not a dependency of the app).
import { createRequire } from "module";
import { spawn } from "child_process";
import path from "path";
import fs from "fs";
import os from "os";
import crypto from "crypto";
import { checkSizes } from "./sizes.mjs";
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_PATH || "playwright");

const base = process.env.SWIFTWIN_UI_URL || "http://127.0.0.1:8787";
const shots = process.argv[2];
const check = (cond, what) => {
  if (!cond) throw new Error("FAILED: " + what);
  console.log("ok -", what);
};
const shot = async (page, name) => shots && page.screenshot({ path: path.join(shots, name) });
// The sliding indicator of a group sits exactly behind its active item once it has settled.
const indicatorOn = (page, group, item) =>
  page
    .waitForFunction(
      ([g, i]) => {
        const ind = document.querySelector(`${g} .slide-ind`)?.getBoundingClientRect();
        const act = document.querySelector(`${g} ${i}.active`)?.getBoundingClientRect();
        return ind && act && Math.abs(ind.left - act.left) < 1.5 && Math.abs(ind.width - act.width) < 1.5 && ind.width > 0;
      },
      [group, item],
      { timeout: 3000 },
    )
    .then(() => true, () => false);
// Window sizes: every state is checked at 800x560 … 3840x2160 (see sizes.mjs); problems are reported at the end.
const sizeProblems = [];
const sizes = async (state) => sizeProblems.push(...(await checkSizes(page, state, { shots })));

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: Number(process.env.SHOT_SCALE || 1), permissions: ["clipboard-read", "clipboard-write"] });
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
await page.goto(base);

// ---------- sign in (mock: fake Supabase + GitHub in the backend) ----------
await page.waitForSelector("#signin:not(.hidden)");
check(
  (await page.isVisible("#auth-email")) && (await page.isVisible("text=Continue with Google")) && (await page.isVisible("text=Continue with GitHub")),
  "signed out: the sign-in screen comes first (email, Google, GitHub)",
);
check(!(await page.isVisible("#onboarding")) && !(await page.isVisible("#home-screen")), "nothing else shows until someone is signed in");
check(!(await page.isVisible(".ob-art")), "the sign-in card is clean (no illustration)");
await page.click('[data-legal="privacy"]');
await page.waitForSelector(".modal .legal");
check((await page.textContent(".modal .legal")).includes("Windows Credential Manager"), "the sign-in screen links the Privacy Policy");
await page.keyboard.press("Escape");
await page.waitForSelector(".modal .legal", { state: "detached" });
await shot(page, "0-signin.png");
await sizes("signin");
await page.click("#auth-submit");
check((await page.textContent('[data-msg="auth-email"]')) === "Enter your email" && (await page.getAttribute("#auth-email", "aria-invalid")) === "true",
  "sending an empty form marks the fields (no round trip)");
await page.fill("#auth-email", "demo@");
check((await page.textContent('[data-msg="auth-email"]')).includes("doesn't look like an email"), "the email is re-checked live while typing");
await page.fill("#auth-email", "demo@swiftwin.dev");
check(!(await page.isVisible('[data-msg="auth-email"]')), "a fixed email clears its message");
await page.fill("#auth-password", "not my password");
await page.click('[data-eye="auth-password"]');
check((await page.getAttribute("#auth-password", "type")) === "text" && (await page.getAttribute('[data-eye="auth-password"]', "aria-pressed")) === "true", "the eye shows the password");
await page.click('[data-eye="auth-password"]');
check((await page.getAttribute("#auth-password", "type")) === "password", "and hides it again");
await page.click("#auth-submit");
await page.waitForFunction(() => document.getElementById("auth-error").textContent.includes("Wrong email or password"));
check(true, "a wrong password shows a clear error");
await page.click('[data-go="signup"]');
check((await page.textContent("#auth-title")) === "Create your account", "Create an account switches the form");
await page.fill("#auth-email", "ada@example.com");
await page.fill("#auth-password", "short");
check((await page.textContent('[data-meter="auth-password"] span')) === "Too short", "the strength meter rates the new password as you type");
await page.click("#auth-submit");
check((await page.textContent('[data-msg="auth-password"]')).includes("at least 8"), "sign-up asks for a password of at least 8 characters");
check((await page.textContent("#auth-title")) === "Create your account", "nothing is sent while a field is wrong");
await page.fill("#auth-password", "a good password");
check((await page.textContent('[data-meter="auth-password"] span')) === "Good", "a longer password rates better");
await page.click("#auth-submit");
await page.waitForFunction(() => document.getElementById("auth-sub").textContent.includes("ada@example.com"));
check((await page.textContent("#auth-title")) === "Check your email", "sign-up asks to open the link emailed to the address");
await page.click(".auth-alt summary");
await sizes("signin-code");
await page.fill("#auth-code", "000000");
await page.click("#auth-submit");
await page.waitForFunction(() => !document.getElementById("auth-error").classList.contains("hidden"));
check((await page.textContent("#auth-error")).includes("wrong or has expired"), "a wrong code is refused");
// The person clicks the link in the email (the mock's inbox): the browser comes back to the app.
const link = await page.evaluate(async () => {
  const t = (await (await fetch("/token")).json()).token;
  const r = await fetch("/api/account_status", { method: "POST", headers: { "X-SwiftWin-Token": t }, body: "{}" });
  return (await r.json()).result.mock_email_link;
});
await fetch(link);
await page.waitForSelector("#welcome-go");
check(true, "opening the email's link signs in by itself");
check((await page.textContent("#auth-title")).startsWith("Welcome to SwiftWin"), "the first sign-in shows a short welcome");
await shot(page, "0-welcome.png");
await sizes("welcome");
await page.click("#welcome-go");
await page.waitForSelector("#signin.hidden", { state: "attached" });

// ---------- home → new project → connect the iPhone ----------
const dir = process.env.SWIFTWIN_UI_PROJECT_DIR || fs.mkdtempSync(path.join(os.tmpdir(), "swiftwin-ui-"));
await page.waitForSelector("#home-screen:not(.hidden)");
check(await page.isVisible("text=What do you want to build?"), "after sign-in, Home offers to start a project");
check((await page.textContent("#home-hello")).includes("Welcome"), "Home greets the person");
check(!(await page.isVisible("#onboarding")), "Home comes before connecting an iPhone");
await shot(page, "1-home.png");
await sizes("home");
await page.click('[data-home="new"]');
await page.fill("#np-name", "Demo");
await page.fill("#np-parent", dir);
await page.click("#np-create");
await page.waitForSelector("#onboarding:not(.hidden)");
check((await page.textContent("#ob-project")) === "Demo", "after creating a project, it asks to run it on the iPhone");
check(await page.isVisible("#ob-skip"), "connecting the iPhone can wait (Skip for now)");
check(!(await page.locator(".toast.error").count()), "starting a project without an iPhone shows no errors");
await shot(page, "1b-connect.png");
await sizes("onboarding");
await page.click("#ob-skip");
await page.waitForSelector("#onboarding.hidden", { state: "attached" });
await page.waitForSelector("#connect-phone:not(.hidden)");
check(true, "skipped: the phone window offers Connect iPhone");
await page.click("#connect-phone");
await page.waitForSelector("#onboarding:not(.hidden)");
await page.click("#use-mock");
await page.waitForSelector("#onboarding.hidden", { state: "attached" });
await page.waitForFunction(() => document.getElementById("screen").src.startsWith("data:image/png"));
check(true, "phone window shows the mock screen");
await page.waitForFunction(() => document.getElementById("sb-device").textContent.includes("Mock iPhone"));
check(true, "status bar shows the device");
await page.waitForFunction(() => document.querySelector(".tab.active")?.textContent.includes("ContentView.swift"));
check((await page.textContent("#tree-root")).includes("ContentView.swift"), "the new project is open with its file tree and a tab");
check(!(await page.isVisible("#welcome")), "the editor shows the code, not the welcome screen");

let box;
const refreshBox = async () => (box = await page.locator("#screen").boundingBox());
const at = (x, y) => [box.x + (x / 393) * box.width, box.y + (y / 852) * box.height];

// Read the UI tree through the panel (open, read, close) until `pred` holds.
async function treeUntil(pred, what) {
  for (let i = 0; i < 40; i++) {
    await page.click("#toggle-tree");
    await page.waitForFunction(() => !document.getElementById("tree").classList.contains("hidden"));
    const t = await page.textContent("#tree");
    await page.click("#toggle-tree");
    if (pred(t)) return check(true, what);
    await page.waitForTimeout(250);
  }
  check(false, what);
}

// ---------- phone window: tap, type, swipe ----------
await refreshBox();
let [x, y] = at(334.5, 132); // the "Hello" icon
await page.mouse.click(x, y);
check((await page.locator(".ripple").count()) > 0, "your tap shows a ripple where you clicked");
await treeUntil((t) => t.includes("Tap me"), "click-to-tap launched the app");
await refreshBox();
[x, y] = at(105, 190); // "Tap me"
await page.mouse.click(x, y);
await page.mouse.click(x, y);
[x, y] = at(196, 257); // text field
await page.mouse.click(x, y);
await page.keyboard.type("Ada");
await treeUntil((t) => t.includes("Hello, Ada!") && t.includes("Count: 2"), "taps counted and keyboard typing reached the text field");
[x, y] = at(196, 700);
const [x2, y2] = at(196, 480);
await page.mouse.move(x, y);
await page.mouse.down();
await page.mouse.move(x2, y2, { steps: 5 });
await page.mouse.up();
await treeUntil((t) => !t.includes('"Item 1"'), "drag-to-swipe scrolled the list");
await page.waitForFunction(() => document.getElementById("log-list").textContent.includes("Button tapped, count = 2"));
check(true, "log panel streams device logs");
await page.fill("#log-filter", "springboard");
check(!(await page.textContent("#log-list")).includes("Button tapped"), "log filter narrows the lines");
await page.fill("#log-filter", "");

// ---------- project dialog, tree, tabs, editor, run ----------
if (dir) {
  await page.click('.tree-row[data-file="Sources/DemoApp.swift"]');
  await page.waitForFunction(() => document.querySelectorAll(".tab").length === 2);
  check(true, "clicking a file opens a second tab");
  await page.click(".monaco-editor");
  await page.keyboard.press("Control+End");
  await page.keyboard.type("\n// edited in SwiftWin\n");
  await page.waitForFunction(() => document.querySelector(".tab.active.dirty"));
  check(!(await page.isDisabled("#save")), "editing marks the tab unsaved and enables Save");
  await page.click("#run");
  await page.waitForFunction(() => document.getElementById("sb-msg").textContent.includes("Launched com.example.demo"), null, { timeout: 60000 });
  check(!(await page.locator(".tab.dirty").count()), "Run saves all files, then builds, signs, installs and launches");
  await shot(page, "2-code-mode.png");
  await sizes("code");

  // The phone / editor resize handle works, and the phone column always leaves the editor room.
  const phoneW = async () => (await page.locator("#phone-pane").boundingBox()).width;
  const drag = async (dx) => {
    const h = await page.locator("#resize-phone").boundingBox();
    await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2);
    await page.mouse.down();
    await page.mouse.move(h.x + h.width / 2 + dx, h.y + h.height / 2, { steps: 5 });
    await page.mouse.up();
  };
  const w0 = await phoneW();
  await drag(-120);
  check(Math.abs((await phoneW()) - (w0 + 120)) < 4, "dragging the handle widens the phone panel");
  await page.setViewportSize({ width: 900, height: 700 });
  await page.waitForTimeout(200);
  const edit = (await page.locator("#editor-card").boundingBox()).width;
  check(edit >= 340, `a small window keeps the editor usable (${Math.round(edit)}px) even with a wide phone panel`);
  const w1 = await phoneW();
  await drag(60);
  check(Math.abs((await phoneW()) - (w1 - 60)) < 4, "the handle still works in a small window");
  // Small windows: the file tree is a drawer.
  check(!(await page.isVisible("#sidebar")) && (await page.isVisible("#explorer-btn")), "small window: file tree folds into a drawer button");
  await page.click("#explorer-btn");
  check(await page.isVisible("#sidebar"), "the drawer button opens the file tree");
  await page.click('.tree-row[data-file="Sources/ContentView.swift"]');
  check(!(await page.isVisible("#sidebar")), "opening a file closes the drawer");
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.waitForTimeout(200);
  await drag(60);
  check(await page.isVisible("#sidebar") && !(await page.isVisible("#explorer-btn")), "large window: the file tree is back in its column");

  // Unsaved edits stay with their project: a new project asks first, and Save writes into the old one.
  await page.click(".monaco-editor");
  await page.keyboard.press("Control+End");
  await page.keyboard.type("\n// kept in Demo\n");
  await page.click("#project-btn");
  await page.click('.modal .seg[data-t="new"]');
  await page.fill("#np-name", "Second");
  await page.fill("#np-parent", dir);
  await page.click("#np-create");
  await page.waitForSelector('.modal h2:text("Unsaved changes")');
  await page.click('.modal [data-v="save"]');
  await page.waitForFunction(() => document.title.startsWith("Second"));
  const demoSrc = fs.readFileSync(path.join(dir, "Demo", "Sources", "ContentView.swift"), "utf8");
  const secondSrc = fs.readFileSync(path.join(dir, "Second", "Sources", "ContentView.swift"), "utf8");
  check(demoSrc.includes("// kept in Demo") && !secondSrc.includes("// kept in Demo"), "switching projects asks to save, and saves into the old project");
  await page.click("#project-btn");
  await page.fill("#op-path", path.join(dir, "Demo"));
  await page.click("#op-open");
  await page.waitForFunction(() => document.title.startsWith("Demo"));
}

// ---------- Inspo: capture another app's look, add its theme, hand it to Claude ----------
if (dir) {
  await page.click("#inspo-btn");
  await page.waitForSelector(".inspo-app");
  const names = await page.$$eval(".inspo-app b", (b) => b.map((x) => x.textContent));
  check(names.includes("Tunes") && !names.includes("Settings"), "Inspo lists the phone's own apps, not system apps");
  check((await page.textContent(".inspo")).includes("personal data"), "Inspo says the captures may show personal data and stay on this PC");
  await shot(page, "7-inspo-pick.png");
  await sizes("inspo-pick");
  check(await page.isVisible("#inspo-deep"), "Inspo offers a deep capture that opens every safe button");
  await page.click('.inspo-app[data-id="fm.tunes.player"]');
  await page.waitForSelector(".inspo-result", { timeout: 30000 });
  await page.waitForFunction(() => document.querySelectorAll(".inspo-shot img").length >= 6);
  check(true, "Inspo explores the app by itself and shows the captured screens in a grid");
  check((await page.textContent(".inspo-chips")).includes("#FF5A5F"), "the palette shows the app's accent colour");
  check((await page.locator(".inspo-type > div").count()) > 0, "the type scale is shown");
  check((await page.textContent(".inspo")).includes("Never copy the other app's logo"), "the dialog says it is inspiration, not copying");
  await shot(page, "7-inspo.png");
  await sizes("inspo");
  await page.click("#inspo-theme");
  await page.waitForSelector('.tree-row[data-file="Sources/InspoTheme.swift"]');
  check(true, "Add theme to my project writes Sources/InspoTheme.swift");
  await page.click("#inspo-claude");
  await page.waitForFunction(() => document.body.dataset.mode === "ai");
  check((await page.evaluate(() => navigator.clipboard.readText())).includes("Do not copy the other app's logo"), "Copy prompt for your AI agent copies the restyle prompt with the guardrail");
  await page.click("#mode-code");
  await page.waitForFunction(() => document.body.dataset.mode === "code");
}

// ---------- AI mode: Claude drives the phone through `swiftwin mcp` ----------
const bin = process.env.SWIFTWIN_BIN;
if (bin) {
  check(await indicatorOn(page, ".segmented", ".seg"), "the mode indicator sits behind Code");
  await page.click("#mode-ai");
  await page.waitForFunction(() => document.body.dataset.mode === "ai");
  check(await indicatorOn(page, ".segmented", ".seg"), "the mode indicator slides to AI mode");
  check(!(await page.isVisible("#editor-wrap")) && !(await page.isVisible("#sidebar")), "AI mode hides the editor");
  check((await page.isVisible("#ai-connect")) && (await page.isVisible("#device")), "AI mode shows the connect panel next to the iPhone");
  check((await page.locator("#chat-input, #mascot").count()) === 0, "the built-in chat and mascot are gone");
  check((await page.textContent("#cmd-claude")) === "claude mcp add swiftwin -- swiftwin mcp", "the panel shows the Claude Code command");
  check((await page.textContent("#cmd-codex")) === "codex mcp add swiftwin -- swiftwin mcp", "the panel shows the Codex command");
  await page.click('[data-copy="cmd-codex"]');
  check((await page.evaluate(() => navigator.clipboard.readText())) === "codex mcp add swiftwin -- swiftwin mcp", "Copy puts the Codex command on the clipboard");
  await page.click('[data-copy="cmd-claude"]');
  check((await page.evaluate(() => navigator.clipboard.readText())) === "claude mcp add swiftwin -- swiftwin mcp", "Copy puts the Claude Code command on the clipboard");
  await shot(page, "3-ai-waiting.png");
  await sizes("ai");

  const mcp = spawn(bin, ["--mock", "mcp"], { env: process.env, stdio: ["pipe", "pipe", "inherit"] });
  let out = "";
  const replies = new Map();
  mcp.stdout.on("data", (d) => {
    out += d;
    let i;
    while ((i = out.indexOf("\n")) >= 0) {
      const line = out.slice(0, i);
      out = out.slice(i + 1);
      if (line.trim()) {
        const m = JSON.parse(line);
        replies.set(m.id, m);
      }
    }
  });
  let id = 0;
  const rpc = async (method, params) => {
    const my = ++id;
    mcp.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: my, method, params }) + "\n");
    for (let i = 0; i < 300 && !replies.has(my); i++) await new Promise((r) => setTimeout(r, 100));
    return replies.get(my);
  };
  const tool = (name, args = {}) => rpc("tools/call", { name, arguments: args });

  await rpc("initialize", { protocolVersion: "2025-06-18" });
  await tool("press_button", { button: "home" });
  await tool("tap", { label: "Demo" });
  const t = await tool("tap", { label: "Tap me" });
  check(t.result && !t.result.isError, "Claude's tap (via swiftwin mcp → app bridge) succeeds");
  await page.waitForFunction(() => document.getElementById("ai-text").textContent.includes("Tapped “Tap me”"));
  check(true, "AI mode captions Claude's action");
  check((await page.locator(".ripple.claude").count()) > 0, "Claude's tap shows an orange ripple on the phone");
  check(!(await page.locator("#claude-cursor").getAttribute("class")).includes("off"), "Claude's cursor shows where it tapped");
  await shot(page, "4-ai-tap.png");
  await tool("swipe", { direction: "up" });
  await page.waitForFunction(() => document.getElementById("ai-text").textContent === "Swiped up");
  check((await page.locator(".trail.claude").count()) > 0, "Claude's swipe draws a trail");
  const tree = await tool("ui_tree");
  check(tree.result.content[0].text.includes("Count: 1"), "the app's phone (not a separate mock) received Claude's tap");
  const nope = await tool("tap", { label: "No such button" });
  check(nope.result.isError, "a tap on a missing element fails");
  mcp.stdin.end();

  await page.keyboard.press("Escape");
  await page.waitForFunction(() => document.body.dataset.mode === "code");
  check(true, "Esc returns to code mode");
  await page.click('.ptab[data-panel="activity"]');
  check((await page.textContent("#activity-list")).includes("Swiped up"), "AI activity tab lists the actions");
  check(await indicatorOn(page, ".tabs-head", ".ptab"), "the tab indicator slides to AI activity");
  await page.click('.ptab[data-panel="logs"]');
  check(await indicatorOn(page, ".tabs-head", ".ptab"), "the tab indicator follows Logs, whose count changes its width");
}

// ---------- account + GitHub + Upload ----------
check((await page.textContent("#account-btn .avatar")) === "A", "the title bar shows the person's initials");
await page.click("#account-btn");
await page.waitForSelector(".umenu.show");
check((await page.textContent(".um-head")).includes("ada@example.com"), "the avatar opens a menu with who is signed in");
await page.keyboard.press("Escape");
await page.waitForSelector(".umenu", { state: "detached" });
check(true, "Escape closes the menu");
await page.click("#account-btn");
await page.click("#um-account");
await page.waitForSelector("#acct-email");
check((await page.textContent("#acct-email")) === "ada@example.com", "Account shows who is signed in");
check((await page.textContent("#acct-plan")).includes("Free plan"), "Account shows the plan (paid plans come later)");
check((await page.textContent("#acct-method")).includes("Email") && (await page.textContent("#acct-verified")).includes("Verified") && (await page.textContent("#acct-since")).includes("2026"),
  "Account shows the sign-in method, email verification and member since");
await page.click("#acct-edit-name");
await page.fill("#acct-name-input", "");
await page.type("#acct-name-input", "Ada Lovelace"); // real key presses, not fill(): a cancelled keydown once blocked typing here
check((await page.inputValue("#acct-name-input")) === "Ada Lovelace", "the name field takes typed keys");
await page.click("#acct-name-save");
await page.waitForFunction(() => document.getElementById("acct-name")?.textContent === "Ada Lovelace");
check((await page.textContent("#acct-avatar .avatar")) === "AL", "the name can be changed, and the initials follow");
await page.click("#acct-pw-open");
await page.fill("#acct-current", "wrong password");
await page.fill("#acct-new", "an even better one");
await page.click("#acct-pw-save");
await page.waitForFunction(() => document.querySelector('[data-msg="acct-current"]')?.textContent.includes("current password is wrong"));
check(true, "changing the password checks the current one");
await page.fill("#acct-current", "a good password");
await page.click("#acct-pw-save");
await page.waitForSelector("#acct-pw-open");
check(true, "the password can be changed");
await shot(page, "6-account.png");
await sizes("account");
await page.keyboard.press("Escape");
if (dir) {
  await page.click("#upload-btn");
  await page.waitForSelector("#gh-connect");
  check(await page.isDisabled("#up-go"), "Upload waits until GitHub is connected");
  check(await page.isChecked("#up-private"), "new repositories are private by default");
  check((await page.inputValue("#up-name")) === "Demo", "the repository is named after the project");
  await page.click("#gh-connect");
  await page.waitForFunction(() => document.querySelector("#up-gh")?.textContent.includes("@octo"));
  check(true, "Connect GitHub (device flow) connects the account");
  check(!(await page.isDisabled("#up-go")), "Upload is enabled once GitHub is connected");
  await shot(page, "6-upload.png");
  await sizes("upload");
  await page.fill("#up-msg", "First upload");
  await page.click("#up-go");
  await page.waitForSelector(".up-done");
  check((await page.textContent(".up-done")).includes("Created octo/Demo"), "Upload creates the repository and shows it");
  check((await page.textContent(".up-done")).includes("View on GitHub"), "the result links to the repository");
  await page.click("#up-go");
  await page.waitForFunction(() => document.querySelector(".up-done")?.textContent.includes("Already up to date"));
  check(true, "uploading again with no changes makes no commit");
  await page.keyboard.press("Escape");
  await page.click("#upload-btn");
  await page.waitForFunction(() => document.querySelector("#up-form")?.textContent.includes("octo/Demo"));
  check(!(await page.locator("#up-name").count()), "later uploads go to the same repository");
  await page.keyboard.press("Escape");
}
await page.click("#account-btn");
await page.click("#um-signout");
await page.waitForSelector("#signin:not(.hidden)");
check(true, "Sign out returns to the sign-in screen");
check((await page.inputValue("#auth-email")) === "ada@example.com" && (await page.textContent("#auth-title")) === "Welcome back", "the sign-in screen remembers the last email");
await page.fill("#auth-password", "an even better one");
await page.click("#auth-submit");
await page.waitForSelector("#signin.hidden", { state: "attached" });
check(true, "the new password works, and the welcome shows only once");
await page.waitForSelector("#home-screen:not(.hidden)");
check((await page.textContent("#home-hello")).includes("Ada"), "Home greets with the new name");
await page.click("#home-account");
await page.click("#um-signout");
await page.waitForSelector("#signin:not(.hidden)");
await page.click('[data-provider="github"]');
await page.waitForSelector("#welcome-go");
await page.click("#welcome-go");
await page.waitForSelector("#signin.hidden", { state: "attached" });
check(true, "Continue with GitHub signs in through the browser (PKCE, loopback)");
await page.waitForSelector("#home-screen:not(.hidden)");
check((await page.textContent("#home-recent")).includes("Demo"), "signing in again opens Home with the recent project");
await page.waitForSelector("#home-account .avatar img");
check(true, "the GitHub profile picture shows on Home");
await page.click("#home-account");
await page.click("#um-account");
await page.waitForSelector("#acct-email");
check((await page.textContent("#acct-method")).includes("GitHub") && (await page.textContent("#acct-security")).includes("managed by GitHub"), "the account shows the GitHub sign-in (no password to change)");
await page.waitForFunction(() => document.getElementById("acct-gh").textContent.trim().length > 0);
check((await page.textContent("#acct-gh")).includes("Connect GitHub"), "signing out disconnected GitHub uploads");
await page.click("#acct-del-open");
await page.fill("#acct-del-confirm", "someone@else.com");
check(await page.isDisabled("#acct-del-go"), "Delete account stays off until the account's email is typed");
await page.fill("#acct-del-confirm", "github-user@example.com");
await shot(page, "6-account-delete.png");
await page.click("#acct-del-go");
await page.waitForSelector("#signin:not(.hidden)");
check(true, "Delete account deletes it, signs out and shows the sign-in screen");
check((await page.textContent("#auth-skip")) === "Continue without an account", "the sign-in screen offers to continue without an account");
await page.click("#auth-skip");
await page.waitForSelector("#home-screen:not(.hidden)");
check(await page.isHidden("#signin"), "without an account SwiftWin opens Home (only cloud features need one)");
await page.reload();
await page.waitForSelector("#home-screen:not(.hidden)");
check(await page.isHidden("#signin"), "the choice is remembered: no sign-in screen at the next start");
await page.click("#home-account");
await page.waitForSelector("#um-signin");
check((await page.textContent(".umenu")).includes("Not signed in"), "signed out, the avatar menu offers Sign in");
await page.click("#um-signin");
await page.waitForSelector("#signin:not(.hidden)");
check((await page.textContent("#auth-skip")) === "Not now", "signing in later can still be put off");
await page.fill("#auth-email", "ada@example.com");
await page.fill("#auth-password", "an even better one");
await page.click("#auth-submit");
await page.waitForSelector("#home-screen:not(.hidden)");
check(true, "other accounts are untouched (Ada signs in again)");
await page.click(".home-recent-item");
await page.waitForSelector("#home-screen.hidden", { state: "attached" });
check(true, "a recent project opens from Home");

// ---------- phone-only window (swiftwin-app --phone / show_phone) ----------
{
  const phone = await browser.newPage({ viewport: { width: 430, height: 900 } });
  phone.on("pageerror", (e) => errors.push("phone view: " + e.message));
  await phone.goto(base + "/#phone");
  await phone.waitForFunction(() => document.body.dataset.view === "phone");
  await phone.waitForFunction(() => document.getElementById("screen").src.startsWith("data:image"));
  check(true, "the phone-only window connects by itself and mirrors the iPhone");
  check(!(await phone.isVisible(".titlebar")) && !(await phone.isVisible("#signin")) && !(await phone.isVisible("#statusbar")), "the phone-only window shows only the iPhone");
  const sb = await phone.locator("#device").boundingBox();
  check(sb && sb.width > 250 && sb.y + sb.height <= 900, "the iPhone fills the small window");
  await shot(phone, "8-phone-window.png");
  // Its toolbar: Home, Run, Screenshot, Inspo, Signing, Studio, Account.
  const bar = ["#home", "#tb-run", "#tb-snap", "#pv-inspo", "#pv-setup", "#pv-studio", "#pv-account"];
  check((await Promise.all(bar.map((b) => phone.isVisible(b)))).every(Boolean), "the phone page's toolbar has Home, Run, Screenshot, Inspo, Signing, Studio and Account");
  check((await Promise.all(bar.map((b) => phone.getAttribute(b, "title")))).every((t) => t && t.length > 3), "every toolbar button has a tooltip");
  await phone.waitForSelector("#pv-account .avatar");
  check((await phone.textContent("#pv-account .avatar")) === (await page.textContent("#account-btn .avatar")), "Account shows the signed-in person's avatar, as in the Studio");
  await phone.click("#pv-account");
  await phone.waitForSelector(".umenu.show");
  const um = await phone.locator(".umenu").boundingBox();
  check(um.y >= 0 && um.y + um.height <= 900, "the account menu opens above the toolbar, on screen");
  await phone.keyboard.press("Escape");
  await phone.waitForSelector(".umenu", { state: "detached" });
  await phone.waitForFunction(() => document.getElementById("pv-setup").title.startsWith("Signing:"));
  check((await phone.getAttribute("#pv-setup", "class")).includes("warn"), "Signing shows a warning dot while signing isn't set up");
  await phone.click("#pv-setup");
  await phone.waitForSelector("#sg-now");
  check(true, "Signing opens the Signing dialog in the phone page");
  await phone.keyboard.press("Escape");
  await phone.click("#pv-inspo");
  await phone.waitForSelector(".inspo");
  check(true, "Inspo opens in the phone page");
  await phone.keyboard.press("Escape");
  await phone.click("#pv-studio");
  await phone.waitForSelector(".toast >> text=Open SwiftWin Studio from the Start menu");
  check(true, "Studio asks the desktop app to show its window (none in the browser UI: a hint)");
  await phone.setViewportSize({ width: 300, height: 640 });
  const tb = await phone.locator(".phone-foot .dev-toolbar").boundingBox();
  check(tb.x >= 0 && tb.x + tb.width <= 300, "the toolbar fits the smallest phone window (300 px)");
  await phone.close();
  // On the MCP bridge (Claude's Browser pane) account and signing commands are refused: Account and
  // Signing then bring the Studio forward instead of opening here.
  const bridged = await browser.newPage({ viewport: { width: 400, height: 800 } });
  bridged.on("pageerror", (e) => errors.push("bridged phone view: " + e.message));
  await bridged.route(/\/api\/(account_|signing_)/, (r) => r.fulfill({ contentType: "application/json", body: JSON.stringify({ ok: false, error: "not available on the bridge" }) }));
  await bridged.goto(base + "/#phone");
  await bridged.waitForFunction(() => document.getElementById("screen").src.startsWith("data:image"));
  check(!(await bridged.locator("#pv-account .avatar").count()), "on the bridge the Account button shows no account details");
  await bridged.click("#pv-account");
  await bridged.waitForSelector(".toast >> text=Open SwiftWin Studio from the Start menu");
  check(!(await bridged.locator(".umenu").count()), "on the bridge Account goes to SwiftWin Studio instead");
  await bridged.close();
  // The Claude app's Browser pane opens `#phone&pair=<one-time code>`; a spent or wrong code says so.
  const pane = await browser.newPage({ viewport: { width: 400, height: 800 } });
  pane.on("pageerror", (e) => errors.push("phone pane: " + e.message));
  await pane.goto(base + "/#phone&pair=0000");
  await pane.waitForFunction(() => document.getElementById("no-screen-text").textContent.includes("expired"));
  check(new URL(pane.url()).hash === "#phone", "a phone link's pairing code is used once and taken out of the address");
  await pane.close();
}

// ---------- signing (mock backend: settings in memory, App Store Connect not called) ----------
await page.click("#account-btn");
await page.waitForSelector("#um-signing");
check((await page.textContent("#um-signing")).includes("Not set up"), "the account menu shows the signing mode");
await page.click("#um-signing");
await page.waitForSelector(".signing #sg-now");
check((await page.textContent(".signing")).includes("7 days") && (await page.textContent(".signing")).includes("3 apps"), "Signing explains the free Apple ID limits");
await shot(page, "6-signing-free.png");
await sizes("signing-free");
await page.fill("#sg-email", "not an email");
await page.fill("#sg-password", "x");
await page.click("#sg-use-free");
await page.waitForSelector("#sg-free-msg.error");
check(true, "a bad Apple ID is refused");
await page.click('.signing .seg[data-t="paid"]');
await page.click("#sg-check");
check((await page.textContent("#sg-paid-msg")).includes("FAIL"), "Check key without a .p8 says FAIL");
const p8 = crypto.generateKeyPairSync("ec", { namedCurve: "prime256v1" }).privateKey.export({ type: "pkcs8", format: "pem" });
await page.setInputFiles("#sg-p8", { name: "AuthKey_ABC123XYZ9.p8", mimeType: "application/octet-stream", buffer: Buffer.from(p8) });
check((await page.inputValue("#sg-key")) === "ABC123XYZ9", "the key id is taken from the file name");
await page.fill("#sg-issuer", "00000000-0000-0000-0000-000000000000");
await page.fill("#sg-team", "BAD");
await page.click("#sg-check");
await page.waitForFunction(() => document.getElementById("sg-paid-msg").textContent.includes("Team ID"));
check((await page.textContent("#sg-paid-msg")).includes("FAIL"), "a wrong Team ID is a FAIL with the reason");
await page.fill("#sg-team", "ABCDE12345");
await page.click("#sg-check");
await page.waitForFunction(() => document.getElementById("sg-paid-msg").textContent.includes("PASS"));
check(true, "a valid key is a PASS");
await shot(page, "6-signing-paid.png");
await sizes("signing-paid");
await page.click("#sg-save");
// A new signing setup goes straight on to WebDriverAgent (the mock phone is connected).
await page.waitForSelector(".wda #wda-install");
check((await page.textContent(".wda")).includes("control your iPhone") && (await page.textContent(".wda")).includes("3 apps"),
  "after signing, the app explains WebDriverAgent and the free-account limits");
await page.click("#wda-install");
await page.waitForSelector("#wda-trust:not(.hidden)");
check((await page.textContent("#wda-trust")).includes("VPN & Device Management"), "after installing it, the app shows how to trust the developer");
await page.click("#wda-check");
await page.waitForFunction(() => document.getElementById("wda-msg").textContent.includes("WebDriverAgent started"));
check(true, "Check it starts: PASS");
await shot(page, "6-wda.png");
await page.waitForSelector(".wda", { state: "detached" });
await page.click("#account-btn");
check((await page.textContent("#um-signing")).includes("Paid account (ABCDE12345)"), "the saved key switches to the paid mode");
await page.click("#um-signing");
await page.waitForSelector(".signing #sg-now");
check(!(await page.content()).includes("PRIVATE KEY"), "the page never gets the key back");
await page.click('.signing .seg[data-t="free"]');
await page.fill("#sg-email", "paid@example.com");
await page.fill("#sg-password", "wrong");
await page.click("#sg-use-free");
await page.waitForFunction(() => document.getElementById("sg-free-msg").textContent.includes("incorrect"));
check(!(await page.isVisible("#sg-code-field")), "a wrong password is reported, and no code is asked");
await page.fill("#sg-password", "my password");
await page.click("#sg-use-free");
await page.waitForSelector("#sg-code-field:not(.hidden)");
check((await page.inputValue("#sg-password")) === "", "the password field is cleared once sent; the 2FA code is asked in the app");
await shot(page, "6-signing-code.png");
await page.fill("#sg-code", "123456");
await page.click("#sg-use-free");
await page.waitForSelector(".signing", { state: "detached" });
await page.waitForSelector(".wda #wda-install");
check(true, "a new Apple sign-in offers to re-sign WebDriverAgent");
await page.keyboard.press("Escape");
await page.waitForSelector(".wda", { state: "detached" });
await page.click("#account-btn");
await page.click("#um-signing");
await page.waitForSelector("#sg-hint");
check((await page.textContent("#sg-now")).includes("paid@example.com") && (await page.textContent("#sg-hint")).includes("paid"),
  "switching back to the free Apple ID works, and a paid team is pointed out");
await page.keyboard.press("Escape");
await page.waitForSelector(".signing", { state: "detached" });

// ---------- doctor ----------
await page.click("#doctor-btn");
await page.waitForFunction(() => document.getElementById("doctor-list")?.textContent.includes("PASS"));
check((await page.textContent("#doctor-list")).includes("No failures"), "Doctor dialog shows PASS/FAIL checks");
await shot(page, "5-doctor.png");
await sizes("doctor");

// ---------- set up this PC (mock: a pretend PC with Apple's service and Python; the rest "installs") ----------
await page.click("#doctor-setup");
await page.waitForSelector(".setup .check");
check((await page.textContent("#setup-list")).includes("TO DO") && (await page.isVisible("#setup-signin")),
  "Set up this PC lists what is missing, with an Apple sign-in for the SDK");
await page.click("#setup-run");
await page.waitForFunction(() => document.getElementById("setup-run")?.textContent.includes("All set"), null, { timeout: 20000 });
check([...(await page.$$eval("#setup-list .badge", (b) => b.map((x) => x.textContent)))].every((t) => t === "DONE"),
  "Set up everything installs each item and then downloads the iOS SDK after the Apple sign-in");
await shot(page, "7-setup.png");
await sizes("setup");
await page.click("#setup-run");
await page.waitForSelector(".setup", { state: "detached" });
await page.click("#doctor-btn");
await page.waitForFunction(() => document.getElementById("doctor-list")?.textContent.includes("PASS"));
await page.keyboard.press("Escape");

check(!sizeProblems.length, "every size: no text overflows its button, nothing off-screen, iPhone fits\n  " + sizeProblems.join("\n  "));
check(!errors.length, "no JavaScript errors: " + errors.join(" | "));
await browser.close();
console.log("UI TEST PASSED");
