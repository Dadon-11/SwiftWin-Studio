# Improvements

A working list, ordered by impact. Security comes first. Status: `[x]` done, `[ ]` open.
Findings and fixes for the security items are in [SECURITY.md](SECURITY.md).

## What SwiftWin is today

A Windows app that builds, signs and runs SwiftUI apps on a real iPhone with no Mac, and lets Claude
Code do the same through an MCP server. It works end to end on the owner's iPhone: build, install,
~23 fps mirror, taps, logs, AI-mode chat, accounts, Inspo, and the phone beside the chat in the Claude
app. One full security review was done on 2026-11-05; accounts, Inspo and the Browser-pane pairing
were added after it.

## 1. Security and privacy (code added after the last review)

- [x] **Inspo can write outside a project** (medium). The MCP tool `inspo_capture` accepts any
  `project` folder, even one without `swiftwin.toml`, and a project's `.inspo` could be a link to
  another folder. A prompt-injected Claude could drop screenshots anywhere the user can write.
- [x] **Inspo lets Claude open any app on the phone** (medium, by design). Captures can show private
  data (messages, mail). AI mode has no web or shell, so they can't leave the PC that way, but an
  outside Claude Code session could read them. Keep it out of AI mode's allowed tools and say so.
- [x] Re-check the pairing flow (`/pair`, `phone_link`) and the accounts commands on the bridge.
- [x] Re-run `cargo audit` and `npm audit`.

## 2. Claude's tools on the phone (from Claude's own test report, score 8/10)

- [x] `tap` with a label that is off-screen taps (0, 0) and says it worked. It must fail loudly.
- [x] `tap` cannot find buttons in an alert. Use WebDriverAgent's alert commands.
- [x] No way to open or close an app; Claude swipes through the user's home screen to find it.
  Add `launch_app` and `terminate_app`.
- [x] `logs` are flooded by system lines. Add an app-only filter (the app's own `Logger` subsystem),
  and make the template log values as public so they aren't shown as `<private>`.
- [x] `ui_tree` is long (many empty containers) and lists off-screen elements. Make the default
  compact; keep the full tree available.
- [x] `screenshot` can catch an animation half-way. Add a short settle wait.

## 3. Product and UX

- [x] New projects get the template's blue "T" icon. Generate an icon from the app's name.
- [x] Review error messages a first-time user hits (no phone, not trusted, Developer Mode off,
  expired 7-day signing) for a clear next step. Reviewed: each names the next step; the expired-signing
  message could not be checked without the phone.
- [x] Leftover working folders and branches from earlier sessions (cleanup needs the owner's OK).

## 4. Code quality, tests, docs

- [x] README says what the app is and how to build, test and install it locally (no GitHub).
- [x] CLAUDE.md still tells sessions to push and use GitHub CI; the owner turned that off.
- [x] Tests for every fix above; `cargo test --workspace` and the UI test stay green.

## 5. Second pass

- [x] Re-read every change with an attacker's eye, rebuild, install, and try it on the iPhone.

## 6. Ready for real users and open source (2026-11-10)

- [x] Delete account (SQL function + Account page), tested against the fake.
- [x] Clear message for expired 7-day signing (apps and WebDriverAgent), from recorded errors.
- [x] Signed automatic updates (Tauri updater), configurable URL; release and code-signing guide.
- [x] Draft PRIVACY.md and TERMS.md, linked from sign-in and README.
- [x] LICENSE (MIT), third-party notices, CONTRIBUTING, README with screenshots.
- [x] Secret and personal-data scan of files and history (docs/publishing.md).
- [ ] Owner: SQL function, Resend SMTP, publish Google sign-in (docs/accounts-setup.md).
- [ ] Owner: review the legal drafts; decide the publishing items (docs/publishing.md).
- [ ] Run docs/testing-checklist.md on another PC and iPhone; see the expired-signing message on the phone.
