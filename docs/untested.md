# Untested on a real device / Apple ID

Everything in the table is written against documented protocols or existing open-source tools but has
**never run against a real iPhone or a real Apple account**. In code, each such path is marked
`// UNTESTED-ON-DEVICE`. Run `grep -rn "UNTESTED-ON-DEVICE" crates cli app` to find them all.

| Area | What | Where | Why untested |
|---|---|---|---|
| Device bridge | unauthenticated lockdown GetValue (which keys are readable before trust) | `real/lockdown.rs`, `RealDevice::info` | the first phone was already trusted |
| Device bridge | pymobiledevice3 calls not used yet: `lockdown pair`, `apps uninstall`, `developer dvt kill` / `process-id-for-bundle-id` (terminate), `crash pull`, `developer dvt proclist` | `real/pmd.rs`, `real/mod.rs` (`uninstall`, `terminate`, `crash_reports`) | not exercised in the first phone session |
| Device bridge | iOS 17.0–17.3: `remote tunneld` (as Administrator) instead of the userspace tunnel | `real/pmd.rs` | the first phone has iOS 27 |
| Device bridge | pymobiledevice3's own pair records (`~/.pymobiledevice3/<udid>.plist`, written by `swiftwin pair`) | `real/mod.rs::has_pair_record` | the first phone was paired through iTunes (usbmuxd record) |
| Device bridge | screen size table for product types other than iPhone18,3 | `real/models.rs` | fallback only, real size comes from WDA |
| Control | WDA endpoints other than `/status`, `/source` and tap: type, swipe, Home and buttons, element lookups by other attributes | `crates/swiftwin-control/src/wda.rs` | not exercised yet |
| MCP tools | `type` focus check (`GET /session/:id/element/active` + the element's name/rect/label), `appearance` (`POST /wda/device/appearance`), `launch_app text_size` (launch argument `-UIPreferredContentSizeCategoryName`) | `wda.rs` (`focused`, `set_appearance`), `crates/swiftwin-mcp/src/tools.rs` | written from appium WDA's source (FBFindElementCommands, FBCustomCommands, 2026-11-12); the phone was unplugged. If `element/active` fails, `type` still types and says it could not check |
| MCP tools | `crash_reports` on the phone (`crash pull`), file names `<Executable>-<date>.ips` | `tools.rs`, `real/mod.rs::crash_reports` | only the mock's reports |
| Device bridge | Stalled `syslog live` detection (20 s without a line) and restart; pymobiledevice3 stderr shown as `swiftwin` log lines | `real/mod.rs::LogCapture` | fake streams only; the stall seen with Tempo was not reproduced (the stream was live when checked) |
| MCP tools | `screenshot` settle wait on a screen that never stops moving (video, a running clock): it should give up after 2 s | `crates/swiftwin-mcp/src/tools.rs` | only tried on a still screen (0.6 s) |
| Control | Swipe feel: press 0.05 s then drag (`dragfromtoforduration`) as a scroll gesture | `wda.rs::SWIPE_PRESS_S` | not exercised yet |
| SDK | Download from download.developer.apple.com with the `ADCDownloadAuth` cookie | `crates/swiftwin-sdk/src/download.rs` | the owner downloaded the `.xip` in a browser |
| Setup | Apple sign-in window: Apple starting the Xcode download after sign-in, `on_download` + `cookies_for_url` in WebView2, the download with those cookies | `app/src-tauri/src/main.rs::apple_signin`, `swiftwin-doctor/src/setup.rs::get_sdk` | needs an Apple ID sign-in; only the mock ran |
| Setup | winget installs on a clean PC (iTunes UAC prompt and restart, Swift 6.2.4 next to a newer Swift, LLVM 22.1.8), no-winget PCs | `swiftwin-doctor/src/setup.rs::RealSetupEnv` | the owner's PC already has everything; probes were checked there |
| SDK | Swift SDK artifact bundle used by `swift build --swift-sdk` | `crates/swiftwin-sdk/src/swiftsdk.rs` | `swiftwin build` calls `swiftc` directly |
| Builder | Loose-PNG app icons (`CFBundleIconFiles`) and loose imageset files instead of `Assets.car` | `crates/swiftwin-builder/src/assets.rs` | not checked on the home screen yet |
| Signer | App Store Connect API calls (certificates, bundleIds, devices, profiles) with the owner's key; `prepare_wda`'s App Store Connect branch; doctor's App Store Connect probe | `crates/swiftwin-signer/src/asc.rs`, `lib.rs`, `swiftwin-workflow`, `swiftwin-doctor/src/env.rs` | the owner uses a free Apple ID |
| Accounts | Supabase Auth: sign-up, email codes, password sign-in, refresh, Google / GitHub sign-in through the browser (PKCE + loopback), password reset, sign-out; tested against the in-process fake only | `crates/swiftwin-account/src/supabase.rs`, `account.rs`, `loopback.rs` | no Supabase project yet (open-questions 27–30) |
| Accounts | GitHub device flow and the GitHub REST calls of Upload (create repo, git trees/blobs/commits/refs) | `crates/swiftwin-account/src/github.rs`, `upload.rs` | no GitHub OAuth app yet (open-questions 31) |
| Accounts | profile picture fetch (Google / GitHub picture hosts only, 1 MB cap, as a `data:` URL), display name (`user_metadata.display_name`), change password (re-check the current one by signing in, then `PUT /user`) | `account.rs::avatar`, `fetch_image`, `update_name`, `change_password` | fake only; Supabase's "secure password change" setting is assumed satisfied by the fresh sign-in |
| Accounts | opening the system browser (`open` crate, ShellExecute) | `account.rs::system_opener` | only the fake browser runs in tests |
| Signer | Windows Credential Manager storage (incl. chunking of long values) | `crates/swiftwin-signer/src/secrets.rs` | compiled on CI only |
| Free Apple ID | Unsigned `.ipa` from `swiftwin build --ipa` / `swiftwin wda ipa` signed and installed by a GUI sideloader; WDA runner found under the sideloader's bundle id | `swiftwin-builder/src/lib.rs` (`package_unsigned_ipa`), `swiftwin-control/src/controller.rs` (`find_runner`) | `swiftwin run` / `wda prepare` with plumesign were used instead |
| Signing setup | App Store Connect key check (`GET /v1/bundleIds?limit=1`, `seedId` compared with the Team ID) from the app's Signing dialog and `swiftwin signing setup` | `swiftwin-signer/src/setup.rs::check_asc` | fake_asc only |
| Signing setup | Paid-team hint: a plumesign profile valid for more than 8 days means a paid team (plumesign does not show listTeams' `xcodeFreeOnly`) | `profile.rs::is_paid_team`, `swiftwin-workflow` (`prepare_wda`) | only free-account profiles seen |
| Signing setup | Apple ID sign-in from the app with a password and 2FA code (with an existing plumesign session, `signing_apple_id` passed on the owner's PC on 2026-11-11): plumesign 2.7.2's `account login` through stdin (password, then the 2FA code; prompts taken from its source), incl. the SMS-number menu that needs a terminal | `plumesign.rs::LoginSession`, `swiftwin-app-core/src/signing.rs` | a fake plumesign (`.cmd`) only |
| Free Apple ID | How long plumesign's saved session lasts (open-questions 26) | `swiftwin-signer/src/plumesign.rs` | needs a `swiftwin run` days after signing in |
| Mirroring | UxPlay on Windows as AirPlay receiver; `-vs "jpegenc ! multipartmux ! tcpclientsink"` pipeline; iPhone discovering "SwiftWin" | `crates/swiftwin-mirror/src/airplay.rs` | needs UxPlay+Bonjour+GStreamer on Windows |
| Desktop app | Crash view against the real phone | `app/`, `crates/swiftwin-app-core` | no app crashed yet; ▶ Run and the file tree were confirmed |
| Inspo | Tried on Spotify only (see progress.md). Not yet: apps with a standard `XCUIElementTypeTabBar`, apps whose tab bar hides on scroll, apps without any tab bar, slow apps (the 1.5 s settle time) | `crates/swiftwin-inspo/src/lib.rs` (`explore`), `style.rs` | only the mock's Tunes app and fixture images so far |
| Cloud release | Git Data API upload + push-triggered workflow on the owner's repo | `crates/swiftwin-release/src/github.rs` | needs the owner's GitHub token |
| Cloud release | `macos-26` runner, Xcode 26 selection, XcodeGen, `xcodebuild archive`/`-exportArchive` upload to TestFlight | `crates/swiftwin-release/src/templates.rs` | needs the owner's Apple account + an App Store Connect app record |
| Accounts | `delete_my_account` SQL function on the real Supabase project (PostgREST `rpc`, PGRST202 when missing) | `supabase.rs::delete_account`, docs/accounts-setup.md | fake only; the owner hasn't added the function yet |
| Device bridge | Expired / unverifiable signing on launch (0xe8008018 family, "invalid code signature … not explicitly trusted", "could not be verified") turned into "7-day signing expired"; the same for the WDA runner | `swiftwin-device/src/lib.rs::signing_expired`, `real/pmd.rs::classify_error`, `swiftwin-control/src/controller.rs::runner_exited` | strings from Apple's error table and others' reports; recorded text in tests only |
| Desktop app | Automatic updates: check, download, signature check, passive NSIS install, restart | `app/src-tauri/src/main.rs` (`update_check`, `update_install`) | no update feed exists yet; signing of update artifacts checked on the owner's PC |
| Installer | NSIS installer: SmartScreen prompt and WebView2 bootstrapper (silent `/S` install, `bin\swiftwin.exe`, Start-menu shortcut and uninstall keeping `data\` are confirmed on the owner's PC) | `app/src-tauri/tauri.conf.json` | installed only silently, on a PC that already had WebView2 |

## Confirmed on the owner's iPhone (2026-11-04, iPhone18,3, iOS 27.0.1, free Apple ID)

Removed from the table above:
- usbmuxd on Windows (web iTunes' Apple Mobile Device Service): ListDevices, ReadPairRecord (dashed UDID), Connect (lockdown and the WDA port forwarder).
- pymobiledevice3 11.49.5: `lockdown info`, `amfi developer-mode-status` / `reveal-developer-mode`, `mounter auto-mount` (personalized image), `remote rsd-info` over the userspace tunnel (no admin), `apps list` / `install`, `developer dvt launch` / `screenshot` / `xcuitest`, `syslog live`.
- XCTest launch of the WDA runner and port 8100 forwarding; WDA `/status`, `/source` and tap (`swiftwin tap --label`).
- A SwiftUI app compiled and linked on Windows (iOS 26.2 SDK, `LC_BUILD_VERSION` stripped, upstream `ld64.lld`) runs on the phone.
- rcodesign signatures accepted by iOS (the re-signed WDA runner); plumesign 2.7.2 `sign --register-and-install` for the app and WDA; plumesign's key at `%APPDATA%\PlumeImpactor\keys\<TEAM>\key.pem`.
- `swiftwin doctor` real probes (all but App Store Connect): 14 PASS, AirPlay WARN.
- MCP (`swiftwin mcp` as registered with `claude mcp add`): `device_info`, `tap`, `ui_tree`, `swipe`, `type`,
  `screenshot`, `logs` against the phone.
- Desktop app with the phone: onboarding lists it as Trusted, Connect, live USB mirror, log panel, and AI mode: the
  MCP server's taps and swipes go through the open app (bridge file, loopback port, token) and show as a ripple /
  trail with a caption.
- A live Claude Code session using the `swiftwin` tools (`install_run`, `ui_tree`) on the phone through the open app;
  AI mode's pill over the phone shows the current build/install step while `install_run` runs.
- Single instance: two launches leave one app process, which owns `app-bridge.json`.
- Installing over a running `swiftwin mcp`: the installer renames the CLI in use and writes the new one; the running
  server keeps answering.
- Mirror from WebDriverAgent's MJPEG stream (device port 9100 through the app's usbmux forwarder, settings through
  `appium/settings`): 23.5 fps at 1206x2622 in the installed app, switching from USB screenshots by itself; a
  reconnect ends the old `syslog live` process and log follower.
- 2026-11-10, through the installed `swiftwin mcp` and the open app, on the Pulse test app: `launch_app` (4 s),
  `terminate_app` (6.5 s), the compact `ui_tree`, `tap` refusing a missing label, the alert routes (a SwiftUI
  alert listed with its buttons, "Cancel" pressed by label), and `logs app_only` (only the app's two
  `os.Logger` lines; pymobiledevice3's `label.subsystem` is there).
- 2026-11-10: `show_phone` from a Claude Code session in the Claude desktop app: the one-time link opened the
  live iPhone in the app's Browser pane beside the chat (seen by the owner), and the
  owner's taps in that pane reached the phone.
- 2026-11-12, by the owner in the installed app with the iPhone: the Signing dialog's "Sign in with your
  Apple ID" (with the existing plumesign session; the password and 2FA prompts were not needed, so that path
  stays in the table), the Phone control dialog (WebDriverAgent found working), and the editor's ▶ Run with
  the file tree on a real project.

## How to work through this list

Follow [first-run.md](first-run.md). `swiftwin doctor` exercises most rows above in dependency order;
`swiftwin run`, `swiftwin wda prepare`, `swiftwin sdk import` and `swiftwin release` cover the rest. When a row
is confirmed on the real phone, remove its `// UNTESTED-ON-DEVICE` marker and delete the row.
