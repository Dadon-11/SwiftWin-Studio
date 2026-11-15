# Security

How SwiftWin is protected, what was checked and what is left for the owner to decide. The full threat
model and the protections of each part are in [docs/security.md](docs/security.md).

## Reviews

| Date | Scope | Result |
|---|---|---|
| 2026-11-05 | Whole app | 9 groups of findings (1 critical, 3 high, 4 medium, 1 low), all fixed with tests; see docs/progress.md, "Security review" |
| 2026-11-10 | Code added since: accounts on the bridge, Inspo, the Browser-pane pairing, the new phone tools | 3 findings below, all fixed with tests |

## Findings of 2026-11-10

| # | Severity | What | Fix |
|---|---|---|---|
| 1 | Medium | The MCP tool `inspo_capture` accepted any folder as `project`, even without `swiftwin.toml`. A Claude session steered by text it read (prompt injection) could write screenshots and JSON into any folder the user can write to. | It now needs a real project (`Project::load`). Test: `app_tools_and_app_only_logs`. |
| 2 | Medium | A project from someone else could ship `.inspo` as a link (junction) to another folder; captures would be written through it. | The folder goes through `paths::join_inside`, which refuses links that leave the project. Test: `a_linked_inspo_folder_is_refused`. |
| 3 | Medium | In AI mode (no permission prompts), a project's text could make Claude start an Inspo capture of any app on the phone, e.g. a mail or chat app, and then read the screenshots. | `mcp__swiftwin__inspo_capture` is disallowed in AI mode; the user starts captures from the Inspo dialog. An outside Claude Code session still asks the user before each tool call. |

Also checked, no change needed:

- **Pairing for the phone page** (`POST /api/phone_link`, `POST /pair`): a 128-bit code, single use,
  2 minutes, minted only by a token holder, compared in constant time, same Host and Origin checks as
  the API; the token itself is never served on the bridge.
- **Accounts on the bridge:** sign-in, sign-out, GitHub and upload commands are refused there, so
  `swiftwin mcp` (Claude) can't reach them.
- **Secrets in source and history:** none found. The Supabase URL, its publishable key and the GitHub
  OAuth client id are public by design. Sessions and tokens are in the Windows Credential Manager.
- **Dependencies:** `cargo audit` reports 0 advisories (242 crates); `npm audit` reports none.
- **New phone tools:** `launch_app` / `terminate_app` only open or close apps, which a tap on the home
  screen could already do. `tap` now refuses elements that are not on screen.

## Added since (2026-11-10, later)

- **Delete account** through a `security definer` SQL function that deletes only the caller
  (`auth.uid()`), callable by `authenticated` only; typed confirmation; refused on the MCP bridge.
  Test: `delete_my_account` (account flows), the UI test, `http.rs` bridge test.
- **Signed automatic updates** (Tauri updater): signature checked against the public key in
  `tauri.conf.json`, https only, installed only on a click, not reachable from the bridge. The private
  key lives outside the repo (`%USERPROFILE%\.tauri\`).
- **Account optional** (2026-11-11): only cloud release and GitHub upload check for a signed-in account
  (CLI, MCP `release`, Upload dialog); nothing else depends on accounts.
- **Expired 7-day signing** is reported as such for apps and the WebDriverAgent runner.

- **AI mode's built-in chat removed** (2026-11-11): SwiftWin no longer starts `claude` itself; AI mode
  only shows how to connect your own Claude Code or Codex to `swiftwin mcp`. Finding 3 above applied to
  that chat and no longer has anything to apply to.

## Risks that remain (your call)

1. **Inspo captures can show private data.** They stay in `<project>/.inspo/` on this PC (git-ignored),
   but anything that can read the project can read them. Delete the folder when you no longer need it.
2. **Your AI agent can use the whole phone.** Through `swiftwin mcp`, Claude Code or Codex can tap
   anything on screen, including other apps, with whatever permissions you gave it in its own settings.
   Watch it when the phone shows something sensitive.
3. **The phone page in the Claude app's Browser pane** holds the bridge token for as long as that tab
   is open; it can do what the SwiftWin window can, except account and upload actions. The swiftwin-phone
   Claude plugin opens that page by itself, and only a `127.0.0.1` phone link; SwiftWin may then keep
   running in the tray with no window (Quit from the tray icon).
4. **Signed with a free Apple ID**, apps expire after 7 days and SwiftWin relies on third-party tools
   (plumesign, pymobiledevice3, WebDriverAgent), pinned by version and checksum.
5. **The update signing key** (`%USERPROFILE%\.tauri\swiftwin-updater.key`, no password): back it up
   offline. Lost = no more automatic updates; leaked = someone could sign an update.
6. **Out of scope:** malware already running as your Windows user, and Apple's or GitHub's own security.

## Reporting a problem

Please don't open a public issue. Once the repository is public, use GitHub's private vulnerability
reporting (**Security → Report a vulnerability**); until then, tell the owner directly. Include what you
did, what happened, what you expected and the SwiftWin version.
