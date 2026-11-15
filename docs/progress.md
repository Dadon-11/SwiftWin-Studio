# Progress log

Phase 1: build the complete app without an iPhone. Each milestone ends with CI green on
`windows-latest`, a working `--mock` demo, and docs updated.

## Rename: Tether is now SwiftWin

The project was renamed on 2026-11-04, before the first phone session. Everything older in this log
that says SwiftWin was written as Tether. Changes:

- Crates `tether-*` are now `swiftwin-*`, the command is `swiftwin`, project files are `swiftwin.toml`,
  environment variables are `SWIFTWIN_*` (was `TETHER_*`), generated bundle ids start with `dev.swiftwin.`.
- The data folder is `%LOCALAPPDATA%\SwiftWin\data` (was `%LOCALAPPDATA%\Tether\data`). Move the old one
  once instead of re-importing the 235 MB SDK, from a normal PowerShell, not from a process started by
  the Claude desktop app (see open-questions 23).
- Credentials stored in the Windows Credential Manager under the old name are not found; sign in again.

## Milestone 1 – Foundation

**Works (tested with MockDevice, CI on windows-latest):**
- Cargo workspace: `swiftwin-core` (types, errors, paths), `swiftwin-device` (`Device` + `Controller`
  traits, `MockDevice`), `swiftwin-control` (`Phone` facade + `connect`), `cli` (binary `swiftwin`).
- `MockDevice` is a stateful simulated iPhone (393x852 pt). One layout drives both the rendered PNG
  screenshot (software canvas + 5x7 bitmap font) and the WDA-style UI tree, so taps on what you see
  change what the tree says. It has a home screen with icons, a demo app (`dev.swiftwin.hello`) with a
  counter button, a text field, a scrolling list and a "Crash" button that writes a crash report.
  Logs are kept and can be followed live.
- CLI: `devices, info, apps, install, uninstall, launch, terminate, screenshot, logs, crashes, tap,
  swipe, type, button, home, ui-tree, demo`, global `--mock`, `--udid`, `--json`.
- `swiftwin --mock demo` runs a scripted tour and saves `swiftwin-demo.png`.

**Notes:** each CLI invocation creates a fresh mock phone, so state does not carry over between
separate `swiftwin --mock ...` commands. Long-running hosts (MCP server, desktop app) keep one phone.

## Milestone 2 – Device bridge (RealDevice)

**Works (tested against fakes, CI on windows-latest):**
- Native **usbmuxd client** (`real/usbmux.rs`): plist protocol v1, `ListDevices`, `ReadPairRecord`
  (= "is this PC trusted?"), `Connect` (port in network byte order), and a local **TCP port
  forwarder** (used for WebDriverAgent). Windows uses Apple Mobile Device Service on
  `127.0.0.1:27015`; `USBMUXD_SOCKET_ADDRESS` overrides. Tested against an in-process fake usbmuxd
  that speaks the same wire format.
- Minimal native **lockdown client** (`real/lockdown.rs`): `QueryType`, unauthenticated `GetValue`
  (ProductType, ProductVersion, UniqueDeviceID, DeviceName) – enough to detect a phone before trust.
- **pymobiledevice3 backend** (`real/pmd.rs`), pinned to 11.49.5 whose CLI was inspected for exact
  command names and output formats: lockdown info, developer-mode status, pair, DDI auto-mount, apps
  list/install/uninstall, dvt launch/kill/screenshot, crash pull, `syslog live --format json`,
  `dvt xcuitest`. Errors are classified into *not trusted*, *Developer Mode off*, *no device*.
  Tested with a fake runner that records arguments.
- `RealDevice` implements every `Device` method; background syslog capture into a ring buffer
  (`logbuf.rs`) so "recent logs" and "follow" work like on the mock.
- CLI: `swiftwin devices` lists real phones with trust status; `swiftwin pair`; `swiftwin setup`
  (creates `%LOCALAPPDATA%\SwiftWin\data\venv` and installs pymobiledevice3).

**Decisions:** shell out to pymobiledevice3 for everything that needs TLS lockdown sessions, the
iOS 17+ RemoteXPC tunnel and DVT/instruments, because re-implementing those natively without a
phone to test against is high risk. The native usbmux layer stays (detection, trust, port forwarding).

## Milestone 3 – Control (WebDriverAgent)

**Works (tested, CI on windows-latest):**
- `wda.rs`: WebDriverAgent HTTP client. Routes and payloads verified against
  `appium/WebDriverAgent` sources: `/status`, `POST /session`, `/window/size`, `/wda/tap {x,y}`,
  `/wda/dragfromtoforduration`, `/wda/keys {value:[chars]}`, `/wda/pressButton {name}`,
  `/wda/homescreen`, `/source?format=json`, `/screenshot`. Re-creates the session once when WDA
  reports "invalid session id". JSON source → `UiNode` (short type names normalised).
- `fakewda.rs`: a WDA-compatible HTTP server backed by `MockDevice`, so the whole HTTP path is
  tested end-to-end (tap/type/swipe/home/source/screenshot/session expiry). `swiftwin wda serve`
  runs it for anyone developing a WDA client.
- `controller.rs`: `WdaController` – forwards device port 8100 over usbmux, reuses an
  already-running WDA, otherwise starts the runner via XCTest (`USE_PORT=8100`), waits for
  `/status`, restarts after a dropped connection, and gives a clear error when WDA is missing.
  Tested with a device double whose `start_xctest` boots a fake WDA after a delay.
- `Phone` now carries the `WdaController` on real phones; `Phone::screenshot()` prefers WDA's
  screenshot (faster than DVT) and falls back to the device service.
- CLI: `swiftwin wda status|start|stop|install <ipa>|serve`.

**Not done here:** building/signing WebDriverAgent itself is part of milestone 5 (signer).

## Milestone 4 – SDK + builder

**Works (tested, CI on windows-latest):**
- `swiftwin-sdk`: native `.xip` extraction – XAR reader (zlib TOC), pbzx decoder (XZ chunks via
  pure-Rust `lzma-rs`, raw chunks passed through), cpio *odc* extractor with path sanitising and
  symlink handling (real symlinks where allowed, otherwise copies – Windows without Developer Mode
  can't create symlinks). Only `iPhoneOS.platform/Developer/{SDKs, Library/Frameworks, usr/lib}` is
  extracted, streaming, so the 3 GB archive never needs to fit in memory. Tested end-to-end with a
  synthetic `.xip` built in the test (real formats, mixed chunk types, symlinked `iPhoneOS.sdk`).
- Swift SDK artifact bundle (`swiftwin-ios.artifactbundle`, schema 4.0) written next to the SDK.
- Resumable Xcode download with the user's `ADCDownloadAuth` cookie (`swiftwin sdk download`), tested
  against a local HTTP server that cuts the first transfer short and requires the cookie.
- `swiftwin-builder`: `swiftwin.toml` project model, `swiftwin new` (Hello SwiftUI app with counter,
  text field, list, `os.Logger`, generated app icon, `Brand` color), asset catalog compiler
  (AppIcon → 120/180 px loose icons + `CFBundleIcons`; imagesets → loose `@2x/@3x` files;
  colorsets → generated `extension Color`), Info.plist generation (incl. `[info]` passthrough),
  swiftc invocation for `arm64-apple-ios<target>` with `-use-ld=lld`, `.app` assembly and `.ipa`
  packaging (Mach-O files get mode 0755). `FakeCompiler` lets `--mock` and CI run the whole pipeline.
- CLI: `swiftwin new`, `swiftwin build [--release]`, `swiftwin sdk list|import|download`.

**Not verified (no Mac/Apple ID/phone here):** compiling real SwiftUI against the real SDK with the
Windows Swift toolchain, and whether iOS accepts loose-PNG icons without `Assets.car`.

## Milestone 5 – Signer (+ WebDriverAgent preparation)

**Works (tested, CI on windows-latest):**
- **App Store Connect API client** (`asc.rs`): ES256 JWT (P-256 `.p8` key; tests verify the signature),
  certificates (DEVELOPMENT), bundleIds (exact match over the prefix filter), devices, profiles
  (IOS_APP_DEVELOPMENT; reuse when the active profile already has this device + certificate,
  otherwise delete and recreate).
- **Native RSA-2048 key + PKCS#10 CSR** (`csr.rs`, `der.rs`) – verified with
  `openssl req -verify` (“self-signature verify OK”).
- **Provisioning profile parsing** (`profile.rs`) – plist inside the CMS blob; team id, devices,
  wildcard coverage, concrete entitlements (application-identifier, team-identifier,
  get-task-allow, keychain-access-groups).
- **Secrets** (`secrets.rs`): Windows Credential Manager via `keyring` (values longer than 1000 chars
  are split across several credentials because a generic credential blob holds at most 2560
  bytes); in-memory store for tests; 0600 files only on non-Windows dev machines.
- **Code signing** (`codesign.rs`): `rcodesign sign --pem-file key --pem-file cert
  --entitlements-xml-file ent App.app` (flags checked against apple-codesign sources), embedded
  profile, temp key files deleted right after. `MockSigner` for `--mock`.
- **WebDriverAgent**: appium's unsigned real-device build (`WebDriverAgentRunner-Runner.zip`,
  confirmed in release v16.15.3) is unpacked, its runner/xctest bundle ids renamed to
  `dev.swiftwin.<team>.WebDriverAgentRunner[.xctrunner]`, old signatures stripped, then signed like any app.
- Full flow tested against a **fake App Store Connect** server that checks every JWT: first run creates
  certificate/App ID/device/profile, second run reuses all of them, a new device recreates the profile.
- New crate **`swiftwin-workflow`**: `Workspace::{build, sign, install_run, prepare_wda}` shared by the
  CLI, MCP server and app. `swiftwin --mock run` builds, mock-signs, packages, installs and launches.
- CLI: `swiftwin run`, `swiftwin sign`, `swiftwin signing setup|status`, `swiftwin wda prepare`.

**Decision:** use the official App Store Connect API (paid Apple Developer Program) rather than
re-implementing Xcode's private Apple ID developer services, which are undocumented and change often.

## Milestone 6 – Mirroring

**Works (tested, CI on windows-latest):**
- `swiftwin-mirror` crate. `Mirror` runs three small threads: a **USB screenshot poller** (uses
  `Phone::screenshot`, i.e. WDA's fast screenshot with DVT fallback; publishes only when the picture
  changed), an **AirPlay frame consumer**, and a **policy ticker**.
- `policy::Policy` – pure, clock-injected source selection: `Auto` prefers AirPlay while frames keep
  arriving (stale after 1.5 s), falls back to USB, and returns to AirPlay only after 1 s of steady
  frames (no flapping); `UsbOnly`/`AirPlayOnly` fixed modes. Unit-tested with synthetic timelines.
- AirPlay prototype: UxPlay as a child process with a GStreamer sink that re-encodes frames to JPEG
  and pushes them over local TCP; `MjpegParser` splits the stream on JPEG SOI/EOI (works with or
  without multipart headers, tolerant of split reads). Tested over a real local socket.
- End-to-end test: mock phone over USB, fake AirPlay feed → switches to AirPlay, feed stops → back to
  USB, feed resumes → AirPlay again; switch history recorded with reasons.
- `Mirror::status()` (active source, mode, health, frame count, USB errors, switch log) for the UI.
- CLI: `swiftwin mirror [--seconds N] [--mode auto|usb|airplay] [--airplay] [--out dir] [--fps N]`.

**Claude's view vs the user's view:** the MCP `screenshot` tool (M7) always takes an exact USB/WDA
screenshot, never an AirPlay video frame.

## Milestone 7 – MCP server

**Works (tested, CI on windows-latest):**
- `swiftwin-mcp`: MCP stdio server (newline-delimited JSON-RPC 2.0) with `initialize` (version
  negotiation: 2025-11-25, 2025-06-18, 2025-03-26, 2024-11-05), `notifications/initialized`, `ping`,
  `tools/list`, `tools/call`, empty `resources/list`/`prompts/list`, batches, parse errors (-32700),
  unknown method (-32601), unknown tool (-32602). Tool failures come back as `isError: true` with the
  message so the model can react.
- Tools: **build, install_run, screenshot** (MCP image content, downscaled to ≤600 px wide, with the
  pixel→point factor), **ui_tree** (text or JSON), **tap** (by label or x/y), **swipe** (coordinates
  or direction), **type, logs** (process/text/level filters), **release** (wired in M9), plus
  **press_button** and **device_info**.
- `swiftwin mcp [--project dir]` – register with `claude mcp add swiftwin -- swiftwin mcp`
  (`swiftwin --mock mcp` to try without a phone).
- Tested with a full Claude-style session (create project → install_run → ui_tree → tap → type →
  swipe → screenshot → logs → errors) in-process, and over real stdio pipes from the binary.

## Milestone 8 – Desktop app (Tauri)

**Works (tested; Tauri build + UI test on windows-latest CI):**
- `swiftwin-app-core`: the whole app backend as `AppCore::dispatch(cmd, json)` – scan/connect,
  mirror frames (long-poll), tap/swipe as fractions of the displayed image → device points, typing,
  buttons, UI tree, streamed logs, project open/new, file read/write (sandboxed to the project),
  build and run. Unit-tested with the mock phone.
- `app/` – Tauri v2 shell with a **single** command (`core`) forwarding to `AppCore` on a worker
  thread; Vite + vanilla JS front end:
  - **Onboarding**: "Connect your iPhone" with the three steps (USB, Trust, Developer Mode), live list
    of detected phones (polls usbmuxd), "Use this iPhone" enabled once trusted, or "Try with a
    simulated iPhone".
  - **File tree**, **Monaco editor** (Swift highlighting, Ctrl+S save, Ctrl+R run), toolbar
    (Open/New/Save/Build/Run, mirror mode Auto/USB/AirPlay).
  - **Phone window**: live mirror; click = tap, drag = swipe, mouse wheel = scroll, keyboard = typing
    (Enter/Backspace supported), Home button, UI-tree overlay.
  - **Log panel** with filter; **status bar** (device, iOS, mirror source + fps, last action/error).
- `swiftwin ui [--mock] [--port] [--static app/dist]` serves the same UI + backend over 127.0.0.1 for
  browser testing. `app/tests/ui-mock.mjs` (Playwright) drives the real UI against the mock phone:
  onboarding → mirror → click-to-tap → typing → drag-to-swipe → logs → new project → edit in Monaco
  → Run (build/sign/install/launch). It passed locally and runs in CI.
- Found and fixed by the UI test: opening the UI-tree panel used to shift the phone image so later
  clicks landed in the wrong place – it is now an overlay.
- App icon generated (`npx tauri icon`), NSIS installer target.

## Milestone 9 – Cloud release (TestFlight)

**Works (tested against a fake GitHub, CI on windows-latest):**
- `swiftwin-release`. `swiftwin release` uploads the project (skipping `build/`, hidden folders, files
  over 50 MB) **plus three generated files** to the `swiftwin/release` branch of the owner's GitHub repo
  using the Git Data API (blobs → tree → commit → create/force-update ref) – no local git needed:
  - `.github/workflows/swiftwin-release.yml` – `runs-on: macos-26`, selects the newest `Xcode_26*.app`,
    installs XcodeGen, `xcodebuild archive` with automatic signing via the App Store Connect API key
    (`-allowProvisioningUpdates -authenticationKey…`), build number = run number, then
    `xcodebuild -exportArchive` with `destination=upload` → TestFlight. The key file is deleted afterwards.
  - `swiftwin-xcodegen.yml` – Xcode project spec generated from `swiftwin.toml` (sources, real asset
    catalog compiled by actool on the Mac, Info.plist properties incl. `[info]`, orientations).
  - `ExportOptions.plist` – `app-store-connect` / `upload` / automatic signing.
- The push triggers the workflow; SwiftWin finds the run by commit sha and follows it to completion,
  reporting the URL and failing with the conclusion if it isn't `success`.
- `swiftwin release setup --repo owner/name --token …` stores the token in the Credential Manager and
  uploads `SWIFTWIN_ASC_KEY_ID/ISSUER_ID/KEY_P8/TEAM_ID` as **encrypted repository secrets**
  (libsodium sealed box with the repo public key, as GitHub requires; tested by decrypting in the fake).
- MCP `release` tool is wired to the same flow when release is configured.
- `swiftwin release --dry-run` (and `--mock`) writes the generated files to `build/release-preview/`.
- Tests caught two real bugs: the deployment target was emitted as a YAML number (`17.10` → `17.1`),
  and Info.plist strings needed escaping.

## Milestone 10 – Doctor + installer + first-run guide

**Works (tested, CI on windows-latest):**
- `swiftwin-doctor`: 15 ordered checks – Apple USB driver, iPhone visible, trusted, device helper,
  Developer Mode, iOS version, developer tunnel (iOS 17+), developer disk image, screenshot,
  WebDriverAgent, signing, Swift toolchain, iOS SDK, rcodesign, AirPlay (optional). Each prints
  PASS/FAIL/WARN/SKIP with the **exact error** and a concrete fix; steps that depend on a failed one
  are SKIPped so the first FAIL is always the thing to fix; exit code 1 on failure. The check runner
  is pure logic over a `DoctorEnv` trait – tested with a configurable failing fake, the mock env,
  and run for real on the dev box (correctly reports missing usbmuxd/helpers/signing/Swift/SDK).
- `swiftwin doctor --fix`: installs pymobiledevice3 and rcodesign, triggers the Trust prompt, reveals
  the Developer Mode switch, prepares WebDriverAgent (when signing works), then re-checks.
  `--json` output for bug reports. A **Doctor** button in the desktop app shows the same report.
- **Windows installer**: Tauri NSIS bundle (per-user install to `%LOCALAPPDATA%\SwiftWin`, WebView2
  bootstrapper) that also ships the CLI as `bin\swiftwin.exe` (built first in CI, then bundled).
  SwiftWin's data moved to `%LOCALAPPDATA%\SwiftWin\data` so it never mixes with installed files.
- `docs/first-run.md`: exact steps for the first session (install → prerequisites → Apple account →
  SDK → plug in → trust → Developer Mode → doctor → WDA → first app → desktop app → Claude Code →
  optional AirPlay/TestFlight → what to send back when something fails).

## Phase 1 summary

All ten milestones are implemented and green on `windows-latest`. ~100 Rust tests plus a Playwright
UI test run on every push, all against `MockDevice`, fake usbmuxd/lockdown, a fake WebDriverAgent,
fake App Store Connect and fake GitHub servers, synthetic `.xip` archives and a fake compiler/signer.
What has *not* happened yet is any contact with a real iPhone, Apple ID, Swift-for-iOS compile or
TestFlight upload – that list is [untested.md](untested.md), and [first-run.md](first-run.md) walks
through verifying it in order.

### Fix after M10 – phone input ordering (found by CI)
The UI test failed once on windows-latest (it had passed on the previous run): a click on a text field
and the typing that followed were independent requests, so when the tap took longer than the 150 ms
typing debounce, the text reached the phone before the field had focus and was rejected. A real user
clicking a field and typing immediately would have lost keystrokes too. All phone input (tap, swipe,
scroll, type, buttons) now goes through one ordered queue, and pending typing is flushed before a new
tap. `swiftwin ui --latency-ms N` simulates a slow phone link; the UI test passes locally with 300 ms
per call and CI now runs it with 150 ms so this class of race stays covered.

## After Phase 1 – AI mode + UI/UX redesign

**AI mode** (title-bar switch, `Ctrl+Shift+A`, `Esc` to leave): the window shows only the iPhone, large
and centred. When Claude Code works through SwiftWin, every tap appears as an orange ripple with a
label, every swipe as a trail, and a caption under the phone says what Claude just did ("Tapped
“Tap me”", "Swiped up", "Installed and launched …"). Until Claude connects it shows the one command
to run (`claude mcp add swiftwin -- swiftwin mcp`) with a copy button.

**How Claude's actions reach the app – the bridge:** Claude Code starts `swiftwin mcp` as its own
process. The desktop app now serves its backend on 127.0.0.1 (random port) and writes
`<data dir>/app-bridge.json` with the port and a per-run token. `swiftwin mcp` forwards each tool call
to the open app, which runs it on *its* phone session and records it for AI mode; when the app is
closed, has no phone, or uses the other kind of phone (mock vs real), `swiftwin mcp` runs the tool
itself as before. Security: the endpoint only accepts loopback `Host` headers (no DNS rebinding) and
requires the `X-SwiftWin-Token` header (cross-site pages can't send it without a CORS preflight the
server never grants) – tested.

**Fixed along the way:** `swiftwin mcp` used to connect to the phone at startup, so if the iPhone wasn't
plugged in when Claude Code launched it, the MCP server failed to start. It now connects on first use.

**UI/UX redesign:**
- New visual design: consistent dark theme with design tokens, icon set, realistic iPhone frame with
  Dynamic Island, focus ring when the phone has keyboard focus, ripple/trail feedback for your own
  input (blue) and Claude's (orange).
- Onboarding: illustrated card; the three steps tick off live (USB detected → trusted → Developer
  Mode); device cards with a Trusted / Waiting-for-Trust chip and a Connect button.
- Code mode: collapsible file tree (Swift icons, unsaved dots), editor tabs (middle-click to close,
  "save before closing?" dialog that never discards on Esc), welcome screen with New / Open / AI mode
  and recent projects, a proper project dialog instead of browser prompts, Run/Build buttons with a
  spinner that save all files first, toasts for results and errors, `Ctrl+S/R/B` shortcuts,
  resizable phone and bottom panels (remembered).
- Bottom panel: colour-coded log rows (time, process, message) with level filter, text filter,
  auto-scroll toggle and clear; a **Claude activity** tab listing every action.
- Phone bar: device name with connection dot, mirror pill ("USB · live"), mirror mode, save
  screenshot, UI-tree overlay. Doctor results are a dialog with PASS/FAIL/WARN/SKIP badges and fixes.
- UI test rewritten: 27 checks, including **Claude driving the app's phone through a real
  `swiftwin --mock mcp` process** and the AI-mode caption/ripple/trail; runs on windows-latest with
  150 ms simulated latency and uploads its screenshots as the `ui-screenshots` artifact.

## First session on the owner's PC (2026-11-03/04, no phone yet)

**Worked:**
- CI artifact `swiftwin-windows` (run 41653734156) installed silently with `/S`: `%LOCALAPPDATA%\SwiftWin`,
  `bin\swiftwin.exe`, Start-menu shortcut; `swiftwin --version` = 0.1.0. `swiftwin --mock demo` and
  `swiftwin --mock doctor` pass.
- `swiftwin doctor --fix` created the venv and installed pymobiledevice3 11.49.5 (on Python 3.13), and
  downloaded rcodesign 0.29.0 (the Windows asset name was right).
- Prerequisites via winget: Swift 6.2.4, Python 3.12, web iTunes 12.13 (replacing the Apple Devices Store
  app, which Sideloadly-style tools need), Rust 1.99.1 + VS 2022 Build Tools for local builds. LLVM 22.1.8
  was already installed.
- `cargo test --workspace` locally: 130 passed.

**Failed / found:**
- **Claude's desktop app sandboxes AppData** for every process it starts (MSIX write virtualization, see
  open-questions 23). The first install, run from a Claude session, landed only in
  `…\Packages\Claude_*\LocalCache\Local\SwiftWin`. Reinstalled from the Terminal panel (outside the sandbox).
  The doctor now warns when such a copy exists, with the merge command.
- **One GitHub edge IP (185.199.109.133) stalls on this network**; the rcodesign download failed twice with
  os error 10053. `http_get` now retries with backoff and rotates the resolved addresses, and checks the
  rcodesign zip against its `.sha256`.
- **Swift's own `ld64.lld` cannot link iOS** (patched in Swift's LLVM fork). The builder now compiles with
  `swiftc -c`, strips platform load commands (`macho.rs`) and links with upstream `ld64.lld` directly
  (also avoids swiftc's Windows-mangled rpath, wrong SDK version and `dsymutil`). Doctor's Swift check
  now also requires a linker and fails when `swiftc` can't start (it used to PASS on exit 0xC0000135).
- **Device bridge:** pymobiledevice3 errors always reported "Run again with -v" (now the last ERROR line);
  error classification matched class names the CLI no longer prints; `swiftwin pair` could never make
  SwiftWin see the phone as trusted (records saved only in `~/.pymobiledevice3`); usbmuxd needs the dashed
  UDID for ReadPairRecord; XCTest stderr was discarded. Doctor's tunnel step no longer needs the disk
  image (`remote rsd-info` instead of `dvt proclist`). iPhone 17-family screen sizes added.
- **Signer:** rcodesign split `--entitlements-xml-file C:\…` at the drive colon, so apps were signed with
  no entitlements; fixed by passing relative paths. `find_profiles` exact-name match (it could have deleted
  other apps' profiles); `.p8` with BOM/CRLF accepted; JWT `iat` backdated 60 s.
- **WDA/control:** a runner installed under another bundle id (as sideloaders do) is now found; startup
  failures include the runner's last output lines; `/status` has a 5 s timeout; `/source` skips the
  expensive visible/accessible attributes; `wda stop` also terminates the runner app.
- **AI-mode bridge:** a stale `app-bridge.json` after an app crash cost ~2 s per MCP call (refused
  loopback connect); dead bridges are now remembered and skipped.

**Free Apple ID path (owner has no paid membership):** `swiftwin build --ipa` writes an unsigned `.ipa`
next to the `.app`; `swiftwin wda ipa` writes WebDriverAgent as an unsigned `.ipa` (dSYMs removed);
`swiftwin wda use <bundle id>` pins a runner. A sideloader signs and installs them with the free Apple ID
(7-day apps, 3 apps, 10 App IDs/week). Sideloadly needs web iCloud, which won't install on this Windows
build; Impactor (MIT) needs only iTunes.
- **SDK import (checked against unxip 3.3 / Xcode 26, no real `.xip` yet):** hard links came out as empty
  files (now NTFS hard links, with a second pass for data stored under a skipped path); symlinks did not
  resolve on Windows (now `\` targets, then junction or copy for folders); names Windows can't store
  (`aux.h`, `a:b`, trailing dot) are skipped with a warning; pbzx checks each chunk's size, decodes xz
  blocks with SHA-256 checks, and runs on up to 6 threads; the real `iPhoneOS.sdk` folder is found before
  aliases exist; Xcode 26 download names (`Xcode_26.3_Apple_silicon.xip`), default 26.3. Also extracts
  `Toolchain/usr/lib/swift/iphoneos` and `libclang_rt.ios.a` for the linker. 138 tests pass locally.
- **Free Apple ID, automated:** `swiftwin signing apple-id --email …` switches signing to plumesign 2.6.3
  (the Impactor CLI; downloaded and checked against a pinned SHA-256). `swiftwin run` builds an unsigned
  `.ipa`, then `plumesign sign --apple-id --custom-identifier <bundle id> --register-and-install --udid
  <usbmuxd DeviceID>` signs, registers the phone and installs; SwiftWin launches it. `swiftwin wda prepare`
  does the same for WebDriverAgent, then re-signs the runner with plumesign's own certificate via
  rcodesign because plumesign leaves the `.xctest` unsigned. `swiftwin new` defaults to a random
  `dev.swiftwin.u<hex>` bundle-id prefix. Doctor's signing and signing-tool checks follow the mode. Found by
  reading plumesign's source: `--udid` takes the usbmuxd DeviceID, the login prompt echoes the password
  (so the Impactor app is the recommended sign-in), and a certificate-limit error makes it revoke an
  existing certificate.
- **Real Xcode 26.3 import (done in a parallel session):** Apple stores duplicate files as hard links whose
  later entries hold `NULLcanary` + zeros (25,609 of 159,279 entries, nlink 0, data entry always first);
  the extractor wrote them as content (2,416 broken iOS SDK files). Fixed in `cpio.rs`; the re-import is
  clean: `iPhoneOS26.2`, 9,161 files, 235 MB, 56 s, with `libclang_rt.ios.a` and the `swift/iphoneos`
  libraries. Windows swiftc 6.2.4 typechecks a SwiftUI app against it, and `swiftwin build --ipa` builds and
  links one (unsigned `.ipa`; Mach-O ios 17.0 / sdk 26.2, LC_MAIN, rpaths `/usr/lib/swift` and
  `@executable_path/Frameworks`). Not yet run on the phone.
- **Impactor sign-in:** winget's Impactor 2.6.3 failed with "Authentication SRP error -80045: This action
  cannot be completed at this time." Since 2026-10-12 Apple refuses the sign-in of older sideloaders
  (AltStore, SideStore, iloader, iPASide reported the same). Impactor 2.7.0–2.7.2 fix the GrandSlam
  request headers; installed 2.7.2 from GitHub and moved SwiftWin's pinned plumesign to 2.7.2 (same CLI
  flags, same DeviceID and `.xctest` behaviour; default anisette server now `mac-ani.jkcoxson.com`).

## First phone session (2026-11-04, iPhone18,3 with iOS 27.0.1, free Apple ID)

**Worked on the phone:**
- usbmuxd (web iTunes' Apple Mobile Device Service) lists the phone with the dashed UDID
  `00008150-…`; the pair record is found; `swiftwin devices` / `info` show model, iOS version, Developer
  Mode state and the right screen size (402x874 pt @3x from the table).
- pymobiledevice3 11.49.5: `lockdown info`, `amfi developer-mode-status`, `amfi reveal-developer-mode`
  (`doctor --fix` made the Developer Mode switch appear), `mounter auto-mount` (personalized disk image on
  iOS 27), `remote rsd-info` over the userspace tunnel (no admin, ~3 s), `developer dvt screenshot`
  (1206x2622 PNG).
- Impactor 2.7.2 sign-in, then `swiftwin signing apple-id`; `swiftwin wda prepare`: plumesign registered
  `dev.swiftwin.u…WebDriverAgentRunner.xctrunner`, signed and installed it; the rcodesign re-sign with
  plumesign's key was accepted by the phone's installer.
- The Tether → SwiftWin data folder move (one-off script): SDK manifest and Swift SDK bundle renamed,
  pip's `pymobiledevice3.exe` launcher regenerated (it holds the venv's absolute path).

**Failed / fixed:**
- Doctor's tunnel check passed `--udid` to `remote rsd-info`, which has no such option; the device now
  goes in `PYMOBILEDEVICE3_UDID`, which pymobiledevice3's tunnel code reads.
- Usage errors showed only the bottom border of pymobiledevice3's error box; the box text is now used.
- plumesign's first run failed with os error 10053 reaching the anisette server (this network drops
  connections now and then); dropped connections are now retried twice, and the "bundle id not
  available" hint is no longer shown for them.
- `cli_mock` tests read the developer's real config (free-Apple-ID mode changed the default bundle id);
  they now run with an empty `SWIFTWIN_HOME`.

**End to end on the phone (build 70ec9a0):** after trusting the developer on the phone, `swiftwin doctor` gives
14 PASS + the optional AirPlay WARN; `swiftwin wda start` brings WDA up; `swiftwin run` builds Hello on Windows,
signs it with the free Apple ID (plumesign), installs and launches it, and the SwiftUI app runs on iOS 27.0.1;
`swiftwin tap --label "Tap me"` taps the button (count 0 → 1, about 1 s once WDA runs); `swiftwin logs --process
Hello --follow` shows the app's `Button tapped, count = 2`; `swiftwin screenshot` saves 1206x2622 PNGs. Removed the
`UNTESTED-ON-DEVICE` markers and untested.md rows for everything that ran (see untested.md, "Confirmed").

**MCP against the phone:** `claude mcp add --scope user swiftwin -- …\swiftwin.exe mcp --project …\Hello` shows
Connected. Driving `swiftwin mcp` over stdio like Claude Code: `device_info`, `tap` by label, `ui_tree` (real
accessibility tree with frames), `swipe`, `type` into the text field and `screenshot` (MCP image + point
conversion) all work. `logs` missed the app's line for a tap made just before: the server only started reading
the phone's log on the first `logs` call (the live log has no history), and the phone logs ~2,600 lines/s,
almost all DEBUG (each WDA UI query adds thousands of accessibility lines), so a 20,000-entry ring held under
8 s. Now the MCP server starts the log stream when it connects (`Device::keep_logs`), and DEBUG has its own ring
(20,000) next to the other levels (60,000): a tap's notice was still there 32 s and four `ui_tree` calls later.

**Desktop app + AI mode with the phone:** onboarding lists the iPhone as Trusted (the Developer Mode step stays
"current" because the device list only knows trust; cosmetic), Connect shows the live USB mirror (2–3 fps) and the
log panel. In AI mode, the installed `swiftwin mcp` sends Claude's tap and swipe through the app: "Tapped 'Tap me'"
bubble and caption, swipe trail, status bar "Claude: …". The first try showed nothing: the app had been started
twice, the second instance overwrote `app-bridge.json`, and after it closed `swiftwin mcp` fell back to the phone
directly (correctly, but invisibly). The app is now single-instance (`tauri-plugin-single-instance`) and publishes
the bridge in `setup`, after the plugin has sent a second launch back to the open window.

## AI mode mascot (2026-11-05)

Claude has a face in AI mode: RetroBot, the owner's avatar (`app/src/assets/retrobot.avatar.json`, an
`avatar-definition` v1 file: an orange box body with two arms and four legs, pill eyes, 28 expressions and 23
looping animations). `app/src/mascot.js` renders it as CSS 3D boxes (no WebGL, ~40 small elements) and plays its
expressions with the file's own hold/transition times, blinks, motions (shake, slow drift) and color overrides.
It sits next to the caption, which is now a speech bubble, and its mood follows Claude:

| Claude | Mascot |
|---|---|
| nothing yet / quiet | `idle`, after 3 min `drowsy`, after 7 min `sleeping` (wakes with `surprised`) |
| entering AI mode | `waking` |
| running build / install_run / release | `thinking`, then `celebrate` and `happy` on success, `sad` on failure |
| reading the screen (screenshot, ui_tree, logs) | `searching`, then `curious` |
| tap / swipe / type / button | `working`, then `listening`, glancing toward where Claude touched |
| any failed tool | `confused` |

`activity_since` now also reports `busy` (the tool Claude is running), so a long `install_run` shows `thinking`
while it runs. Respects `prefers-reduced-motion`; paused outside AI mode. The mock UI test checks idle, listening,
curious and confused.

## Liquid Glass UI (2026-11-05)

The desktop app now looks like iOS 26, using [glisse](https://github.com/0kzh/glisse) (MIT). glisse ships React
components; SwiftWin's UI is plain JavaScript, so its framework-free core (lens maps, refraction, lit rim, press
springs, styles) is vendored in `app/src/vendor/glisse/`, with its two hooks rewritten as `glass(el)` /
`autoGlass()` for `[data-glass]` elements. WebView2 is Chromium, so glass gets glisse's full live-backdrop
refraction, not just the frost other browsers get.

- A deep ambient background (blue, violet, Claude orange) that the glass frosts and bends.
- Floating rounded glass panels (explorer, editor, logs, phone) with lit rims; the title bar has no bar, its
  controls are glass capsules (Code / AI mode switch, project, Save, Build, Run, Doctor) that spring when pressed.
- Monaco on a transparent background over its glass card; capsule tabs, inputs and log tools.
- Glass toasts (bending what is under them), dialogs (frosted sheets), onboarding card and device cards.
- AI mode: the phone floats over the ambient light with no card around it; Claude's caption is a glass capsule next
  to the mascot. The UI test passes unchanged; screenshots in its CI artifact show every screen.

**Display name and logo:** "SwiftWin Studio" with the owner's logo (window title, brand, installer description,
app icons). productName, the install folder, the CLI and the data paths are unchanged.

**Installing over a running `swiftwin mcp`:** once `swiftwin` is registered with Claude Code, every Claude Code
session runs `bin\swiftwin.exe mcp`, which keeps the file open. The silent installer then could not replace it
and kept the old CLI without a word (the app itself was updated). The installer now renames a CLI in use to
`swiftwin.old-<tick>.exe` before copying (Windows allows renaming a running .exe; the running servers keep using
it) and deletes old copies on the next update and on uninstall (`app/src-tauri/windows/hooks.nsh`). Confirmed on
the owner's PC: two launches of the app leave one process, which owns `app-bridge.json`; installing build 559f9d5
while this Claude Code session's `swiftwin mcp` ran renamed the CLI in use to `swiftwin.old-<tick>.exe`, wrote
the new one (same SHA-256 as CI's), deleted an old copy no longer in use, and the running server kept answering.

## AI mode as a Claude Code workspace, and a realistic iPhone (2026-11-05)

Asked for by the owner with two references: an Xcode-style iPhone 17 Pro frame, and Claude's own device view
(chat on the left, device on the right).

- **iPhone frame:** a thin titanium rim, a black bezel and the screen, with the Action button, volume buttons and
  side button; Dynamic Island sized as on iPhone 17. All sizes are fractions of the screen, so it scales.
- **AI mode** is now a chat with Claude Code next to the device panel. The app runs the owner's own `claude` CLI
  headless in the open project (`claude -p --output-format stream-json --verbose`, the prompt on stdin, follow-ups
  with `--resume <session>`, `--permission-mode acceptEdits`, allowed tools `mcp__swiftwin,Read,Glob,Grep`, no
  shell) and streams its text, tool calls and results into the transcript (`crates/swiftwin-app-core/src/claude.rs`).
  Claude reaches the phone through the registered `swiftwin` MCP server, which forwards to the open app, so its
  taps also show on the phone. npm's `claude.cmd` is resolved to the native `claude.exe` behind it. Actions from a
  Claude Code session outside the app show in the same transcript, tagged "Claude Code".
- **Device panel:** "Claude is using this device" (a white pill over the phone while Claude works, else its last
  action), Claude's orange cursor gliding to each tap and along each swipe, and a toolbar (Home, screenshot, run,
  back to code). RetroBot sits in the chat header and thinks between Claude's tool calls.
- Checked on the owner's PC: `claude -p --output-format stream-json --verbose` with Claude Code 2.1.235 prints
  `system/init` (model, session id, `swiftwin` MCP connected), `assistant`, `result`. The UI test drives the chat
  with a recorded run (`app/tests/fake-claude.mjs`, `SWIFTWIN_CLAUDE`), including a follow-up that resumes.

**From a live Claude Code session, with the phone:** `install_run` (Hello, about 18 s once cached) went through the
open SwiftWin Studio app in AI mode; RetroBot showed `thinking` while it ran, then `celebrate` with the caption
"Installed and launched dev.swiftwin.u…hello"; `ui_tree` returned the app's live accessibility tree.

## Phone mirror at ~23 fps: WebDriverAgent's video stream over USB (2026-11-05)

The owner asked for more than the mirror's ~3 fps. Each frame was a full-size PNG from WDA `/screenshot` (1206x2622,
~300 ms). WebDriverAgent also runs an MJPEG server (device port 9100). Measured on the owner's iPhone (iOS 27.0.1,
WDA v16.15.3) through `pymobiledevice3 usbmux forward`, 6 s per setting:

| `mjpegServerFramerate` / scale / quality | Frame | Received |
|---|---|---|
| 10 (default) / 100 / 25 | 1206x2622, ~146 KB | 9 fps |
| 30 / 50 / 50 | 603x1311, ~222 KB | 23 fps |
| 60 / 50 / 25 | 603x1311, ~205 KB | 25 fps |
| 60 / 40 / 35 | 482x1049, ~147 KB | 23.5 fps |
| 60 / 100 / 25 | 1206x2622, ~146 KB | 23 fps |

Smaller frames barely help: the phone's capture rate (~25 fps) is the limit, not USB. The stream sends frames
whether or not the screen changes. Settings need a WDA session (`POST /session/:id/appium/settings`).

- **Mirror:** a video source (`Video { kind, feed }`) next to the USB screenshots. `Feed::Mjpeg` reads an
  MJPEG-over-HTTP server and reopens it when it ends (0.5 s, doubling to 30 s while it keeps failing);
  `Feed::Frames` is the AirPlay channel as before. The policy is now "video vs. screenshots" (`Mode::VideoOnly`
  replaces `AirPlayOnly`). Repeated JPEGs of a static screen are not re-published. Fixed a race found by the new
  test: a slow screenshot that finished just after the switch to video replaced the video frame; frames are now
  published only by the active source.
- **Phone:** `Phone::video_stream(fps)` starts WDA if needed, sets frame rate 30, full size, quality 25, and
  forwards device port 9100 over usbmux.
- **App:** real phones mirror from the stream (pill "Video · N fps"), with USB screenshots ("Screenshots") while
  WDA starts or if the stream stops; the menu offers Auto / Video / Screenshots. `swiftwin mirror` uses the
  stream too (`--airplay` swaps in AirPlay).
- **Duplicate logs:** every Connect started another log follower and another `syslog live` process and never
  stopped the old ones (the owner's app had three). A reconnect now ends the old follower, and dropping a
  `RealDevice` kills its `syslog live` process.

**On the owner's iPhone with the installed build (c2d097a):** after Connect the mirror showed USB screenshots while
WebDriverAgent started (~30 s), then switched by itself ("the USB video stream is running; switching to smooth
video"); WDA reported `mjpegServerFramerate` 30, scale 100, quality 25. With the Clock stopwatch running, the app's
`frame` API delivered 236 frames in 10 s (23.5 fps, 1206x2622 JPEG, ~80 KB each) and the window's pill read
"Video · 24 fps". A static home screen sends almost nothing (identical frames are skipped). Two reconnects left one
`syslog live` process (the newest) and reused the running WDA.

## Motion polish (2026-11-06)

The owner asked to apply a list of 20 animation rules across the app, keeping its design. `app/src/motion.js` holds
the helpers and `style.css` the tokens (`--ease-out`, `--ease-in-out`, `--dur-in` 220 ms, `--dur-out` 176 ms):

- Entrances ease out, moves ease in-out, exits are 20% shorter, and nothing bounces (modals and toasts lost their
  overshoot). Everything collapses when Windows asks for reduced motion.
- Presses: non-glass buttons, tabs and tree rows scale to 0.97 at once and ease back on release; glass buttons keep
  glisse's press.
- Modals start at 0.95, grow out of the control that opened them and shrink back into it; their fields stagger in.
  Toasts rise from the corner at 0.95.
- Code / AI mode: one sliding indicator moves between the two items and stretches 12% on the way; the switch is a
  view transition where the phone panel glides to its new place as the same element, old and new layouts overlap,
  and the new one is revealed in a circle from the pressed button. The Logs / Claude activity tabs share the same
  indicator behaviour.
- Lists stagger 30 ms per row: new file-tree rows (project opened, folder expanded), Doctor checks, device cards,
  onboarding steps, welcome tiles. New chat rows ease in.
- Claude's cursor travels on a gentle arc; Run and Build pulse on success; Copy crossfades to a check with a blur.
- The phone and log resize handles rubber-band past their limits and spring back.

The UI test checks that both indicators sit behind their active item after switching (including when the Logs
count changes the tab's width). `app/tests/motion-frames.mjs` captures frames mid-animation for review.

## Security review (2026-11-05)

Full review before other people use SwiftWin; threat model, protections and known limits are in
[security.md](security.md). Findings, most severe first, all fixed with tests:

- **Critical: a project could run commands through AI mode.** Headless Claude Code skips the
  folder-trust prompt, so a project's `.claude/settings.json` hooks ran on every message (confirmed with
  Claude Code 2.1.235). AI mode now passes `--setting-sources user` and disallows `Bash`, `WebFetch`,
  `WebSearch` and `mcp__swiftwin__release`.
- **High: the desktop app had no CSP** (`csp: null`): HTML injected through a device name, log line or
  file name would run with full backend access. Now a strict CSP, the same one the browser UI server
  sends; the UI test passes under it with no violations.
- **High: the app's bridge handed out its token** at `GET /token` to anything on `127.0.0.1`, including
  other Windows accounts. Now only `swiftwin ui` serves it.
- **High: `zip` 2.6** had four advisories (one critical). Now `zip` 9; `.ipa` entries are written as
  Unix entries so the executable bit survives.
- **Medium: `cmd.exe` ran programs from the project** (`node.bat` next to the sources) when npm's
  `claude.cmd` starts `node`. Fixed with `NoDefaultCurrentDirectoryInExePath=1`.
- **Medium: links and Windows path tricks.** `read_file`/`write_file` followed junctions out of the
  project and accepted device names and alternate data streams; `walk` followed links (file tree, build,
  release upload). `swiftwin.toml` folders could point outside the project.
- **Medium: unpinned downloads.** WebDriverAgent was not checked; rcodesign only against a checksum
  from the same host. Both are pinned now. `find_*` skip relative `PATH` entries.
- **Medium: the cloud release workflow** gave the App Store Connect key to every step (including
  `brew install`) with the default token permissions. Now `contents: read`, key only where used,
  pinned checkout. `[info]` keys can no longer inject YAML into the XcodeGen spec.
- **Low:** constant-time token compare, Origin / `Sec-Fetch-Site` checks, request size and connection
  limits, read timeout, graceful close; rcodesign's key folder is new and random; CI has read-only
  permissions, pinned actions, `cargo audit` and `npm audit`.

Tried: `freezePrototype` in Tauri's security config; left out because Monaco and the glass effect were
not tested with it.
## Desktop app works at every window size

The owner asked for an app that "is capable in every size I choose" with the current design kept. A new check,
`app/tests/sizes.mjs`, tests onboarding, welcome, Code mode, AI mode (with a chat) and Doctor at 800x560,
1024x700, 1280x800, 1440x900, 1920x1080, 2560x1440, 3840x2160, 900x1200 and 1700x620. At each size it checks
that no button, pill, tab or chip text overflows or wraps, that no control is off-screen or cut off by its panel,
that panels don't overlap or collapse, and that the iPhone frame fits its panel without being stretched.
`ui-mock.mjs` runs it and saves a screenshot per state and size, so CI's `ui-screenshots` artifact has them all.

**Before:** 170 problems. Below ~1000 px, Build, Run, Doctor, the mirror menu and the phone buttons were
off-screen and the page scrolled sideways. The log header cut off its tools, "Claude activity" wrapped and
log lines wrapped one word per line. The status message ran off the window. In AI mode the device name was
squeezed to nothing. At 2560 px and up, and in tall windows, the iPhone frame was stretched (`max-width`
broke the aspect ratio) and its screen was cropped.
**After:** none (comparisons in `docs/responsive/*.jpg`). The look at 1440x900 is unchanged. What changed:

- The iPhone frame is sized with container units (`min(100cqh, 100cqw / ratio)`), so it always fits and keeps
  its shape. The phone column grows on big screens (`clamp(430px, 22vw, 640px)`) and never takes the editor's
  last 360 px. The resize handle starts from the panel's real width and still works at small sizes.
- At 1000 px wide or less, the file tree is a drawer opened from a folder button in the title bar. The window
  name is hidden, and at 860 px or less Build and Run keep only their icons, with tooltips.
- Container queries hide the least useful bits as the phone panel narrows (mirror pill, iOS version, tap hint)
  and as the log panel narrows ("Claude activity" becomes "Activity"; the time column, then the level menu, go).
- Long labels (project name, device, status message, Claude's last action, chat tool rows) are cut with "…"
  and show the full text in a tooltip. In very wide windows the chat is centred at 960 px.
- Short windows: the log panel takes at most 30% of the height, and the welcome tiles stack when the editor is
  narrow. The Tauri window's minimum size is now 800x560 (was 1100x700).

Not checked on the installed app on the owner's PC yet (Windows display scaling changes the CSS size: a 4K screen
at 150% is a 2560x1440 viewport).

## Accounts and GitHub upload

**Works (tested against in-process fakes, `--mock`, UI test):**
- New crate `swiftwin-account`: Supabase Auth over its REST API (no SDK, no SwiftWin server). Email +
  password with 6-digit email codes for sign-up and password reset; Google and GitHub sign-in in the system
  browser with PKCE (S256) and a loopback redirect on a random port; sign-out ends only this session
  (`scope=local`). The refresh token is kept in the Credential Manager, the access token only in memory, and
  `account.json` keeps the public profile so the app can start offline. `Session`'s `Debug` hides tokens.
- Settings are public values only (project URL, publishable key, GitHub client id): built in, or
  `auth.json`, or environment. A `service_role` / `sb_secret_` key or plain-http URL is refused.
- Connect GitHub with the OAuth device flow (no client secret in the app), `repo` scope, token in the
  Credential Manager. Signing out also disconnects GitHub.
- Upload: the first upload creates a private repository named after the project (`auto_init`), then every
  upload is one commit making the default branch match the folder (Git Data API, fast-forward only, never
  forced). Unchanged files aren't sent again (git blob ids), and nothing is committed when nothing changed.
  Skips `.gitignore`d files, build output, bundles, SDKs, signing files and secrets, and files over 50 MB;
  stops if a file contains a private key.
- App: the sign-in screen comes before onboarding when signed out (same card as onboarding); an Account
  dialog (who, plan, GitHub, sign out); an Upload button and dialog with progress and the repository link.
  The account, GitHub and upload commands are refused by the MCP bridge, so Claude Code can't use them.
- CLI: `swiftwin login [--email | --google | --github]`, `logout`, `whoami`, `upload`.
- Plan hook for selling later: `app_metadata.plan` (server-set only), shown in the Account dialog.
- Tests: 14 in `swiftwin-account` (fake Supabase + GitHub: every flow, refresh-token reuse, a stolen code
  without the verifier, offline start, upload create/update/no-op/name clash/private key), an AppCore test
  through `dispatch`, the bridge refusing the private commands, and the UI test (sign-in screen, wrong
  password, sign-up with code, Upload with Connect GitHub, sign out, Continue with GitHub).

**Not done yet:** the owner's Supabase project, Google client and GitHub OAuth app don't exist yet, so none
of this has run against the real services (untested.md, open-questions 27–31).

**Email links instead of codes (2026-11-06):** Supabase no longer lets a project edit its email templates
without custom SMTP, so the emails can't show a 6-digit code yet. Sign-up and password reset now use the
email's link: the app listens on a loopback port (PKCE, like Google/GitHub sign-in), and opening the link
on the same PC signs in (and, for a reset, asks for the new password). The code entry stays as a fallback.
The owner's Supabase project exists (URL and publishable key built in).

**Services set up (2026-11-06, with the owner):** the Supabase project (email on, confirm email
on, min password 8, Site URL `http://127.0.0.1`, redirect `http://127.0.0.1:*/auth/callback`); GitHub OAuth app
"SwiftWin" (device flow on, non-expiring tokens) enabled as Supabase's GitHub
provider; Google Cloud project "SwiftWin" with an External consent screen in *Testing* (the owner is the only test
user) and a Web client enabled as Supabase's Google provider. Checked: Supabase reports both providers on and
`/authorize` redirects to GitHub/Google with the app's loopback address. Not yet tried in the installed app.
Before selling: publish the Google app (Branding needs a home page and privacy policy), custom SMTP for email.

**Start-up flow redesigned (owner's request, 2026-11-06):** the sign-in card no longer has the iPhone
illustration (one clean card). After sign-in the app opens on **Home** ("What do you want to build?":
New project / Open project, recent projects) instead of "Connect your iPhone". Connecting comes once a
project is open, as "Let's run <project> on your iPhone" with the three steps as cards, "Try a simulated
iPhone", and "Skip for now"; when skipped, the phone window shows a **Connect iPhone** button. The title-bar
logo goes back to Home. Email sign-up was confirmed on the owner's PC: the link signed in by itself.

**Inspo (2026-11-06):** take design inspiration from an app already on the phone. New crate
`swiftwin-inspo`: `user_apps` (the phone's user apps minus the WebDriverAgent runner), `capture` launches
the chosen app and explores it safely (only tab-bar buttons are tapped, found inside `XCUIElementTypeTabBar`
in a fresh UI tree before every tap; each tab is scrolled up to twice; distinct screens by their texts and
positions; at most 12; never captures another app; cancel keeps what was captured), then returns to the
user's app or Home. `style.rs` extracts the palette (colour clusters over all screenshots; background =
the colour at the side edges, text = the most used low-saturation colour with 4.5:1 contrast, surface,
accent), the type scale (text frame height / 1.2), spacing (padding, stack gap, row height), corner radius
(measured on the screenshot along the corner diagonal of filled buttons/cells) and component patterns,
and writes `<project>/.inspo/<app>/` with `NN.png`/`NN.json`, `inspo.json`, `INSPO.md` and
`InspoTheme.swift` (plain values, no macros); `.inspo/.gitignore` ignores it all. The guardrail (inspiration
only, no logos/icons/artwork/brand/exact screens) is in the theme, `INSPO.md`, the dialog and Claude's prompt.
- App: **Inspo** button → app list → progress with Stop → screens grid, palette chips, type scale, spacing,
  patterns; **Add theme to my project** (copies the theme into the sources folder) and **Apply with Claude**
  (AI mode with a pre-filled restyle message). Commands `inspo_apps/start/status/cancel/image/add_theme`.
- MCP: `inspo_capture {bundle_id}` (without it, lists the apps), forwarded through the open app like the
  other tools.
- Mock phone: new **Tunes** app (`fm.tunes.player`, dark, three tabs, a list, and a "Buy Premium" button
  the tests check is never tapped).
- Tests: 8 in `swiftwin-inspo` (fixture-image palette/type/spacing/corners, the Swift theme, exploring Tunes,
  cancel, an app without tab bar), an AppCore flow, and the UI test (list, capture, grid, palette, add theme,
  Apply with Claude). Not tried on the real phone yet (untested.md).

**Inspo on the real phone (2026-11-06, Spotify, iPhone18,3 / iOS 27.0.1):** the app list showed the phone's ~90
user apps (Apple apps and the WebDriverAgent runner hidden). First run found no tabs: Spotify draws its own
tab bar (`TabBarContainer.stackView` with `TabBar.Item.Home` buttons, no `XCUIElementTypeTabBar`). Fixed:
a wide strip in the bottom quarter named like a tab bar counts too, nested buttons are de-duplicated by
label, and action tabs ("Create", "New", "Add", "Post", "Buy"…) are never tapped. Second run: 10 screens
over Home, Search and Your Library in about a minute, "Create" skipped, then back to the user's app (Pulse).
Palette: background `#121212`, surface `#292929`, text `#FEFEFE`; type 30/15/11 pt; padding 16. The accent
first came out as the blue of a big promo card, so it is now the vivid colour seen on the most screens
(green `#19E58C` on that capture). Limit: artwork is not in WDA's tree, so in artwork-heavy apps the accent
can still be an artwork colour (a later run that opened on Your Library picked the blue again); the dialog's
"most used" swatches show the alternatives. Re-run the extraction on a capture with
`INSPO_DIR=<project>/.inspo/<app> cargo test -p swiftwin-inspo real_capture -- --ignored --nocapture`.
Note: piping `swiftwin mcp` in a shell can hang after the reply because the WebDriverAgent runner it starts
keeps the pipe open; redirect to a file instead.

**Inspo deep capture (2026-11-06, owner's request):** a "Deep capture" option (dialog checkbox, MCP `deep: true`)
also opens every button and row on each tab screen and scroll position, captures what it shows, and comes back
(Back/Close/Cancel button, edge swipe, sheet swipe down, re-tapping the tab, finally restarting the app). Up to
40 screens and 120 taps. Buttons whose label suggests a change are never tapped, in either mode: buy, pay,
subscribe, premium, send, post, share, delete, remove, sign in/out, follow, like, add, create, new, save, edit,
download, allow, confirm… (`is_risky`, whole words). Mock: Tunes rows open a detail screen with Back and a
"Delete Playlist" button; the test checks deep mode opens rows on several tabs and never deletes or buys.

**Deep capture on the real phone (2026-11-06, Spotify, from the app):** finished by itself: 40 screens, accent
Spotify's green `#1DD660`, nothing bought/changed. But all 40 went to Your Library and Home (Search never got a
turn) and several were near-copies (97% the same labels), and the type scale lost the large titles (rare sizes
dropped). Fixed: each tab gets an equal share of what is left of the budget, a screen whose labels are 85% the same
as a captured one is skipped, and the largest size seen twice is always kept.

**Phone-only window + Inspo fixes (2026-11-06, owner's feedback):**
- Phone-only window: `swiftwin-app --phone` (or `swiftwin phone`, or Claude Code's new `show_phone` MCP tool)
  opens a small window with just the iPhone mirror, its bar, Home and Claude's caption/ripples, so the owner can
  work in the Claude Code app or a terminal instead of SwiftWin Studio. No sign-in or project; it connects to the
  first trusted iPhone by itself. If the app is open, the single-instance hook adds the window there (same phone
  session and bridge, so Claude's actions show in it). Windows are now created in `setup` (`create: false` for
  main), and the capability covers both windows.
- Inspo dialog lag: the ~90 app cards no longer use Liquid Glass (refraction on each was the lag); a search box
  filters them (Enter picks the first match).
- Claude only looked at 2 of 40 screenshots: each capture now also writes `overview.png` (all screens in one
  grid), and the Apply-with-Claude prompt is a step list: overview first, then every NN.png, use the theme in
  every view, then install_run and compare screens until it holds up.
- Model switching in AI mode: not added; the owner prefers the Claude Code app (which has it) plus this window.

**The iPhone inside the Claude desktop app (2026-11-06, owner's request):**
- The owner wanted the phone in the Claude Code tab's right-hand panel instead of a separate window. A Claude
  Code "mod" pane was ruled out: on the desktop surface a mod pane can draw text, buttons and SVG but no image
  or web page, so no 23 fps mirror. The app's built-in Browser pane sits in the same place and shows a real page.
- The desktop app's bridge now also serves its embedded web UI (`http::Ui::Embedded`, Tauri's asset resolver).
  `show_phone` run from the Claude desktop app (`CLAUDE_CODE_DESKTOP_APP_VERSION` set) gets a one-time link
  (`/api/phone_link`, `/pair`; see security.md) and tells Claude to open it in the Browser pane. It starts the
  app minimized (`--pane`) when it isn't running. Elsewhere `show_phone` still opens the phone window.
- The phone page shows "SwiftWin isn't running" when the app that served it is gone, and "this phone link has
  expired" for a spent code. Tests: `bridge_phone_page_pairs_once` (Rust), the pairing step in ui-mock.mjs.

## Owner-mode improvement round (2026-11-10)

The owner asked for a full pass as owner, engineer, designer, security reviewer and QA. The working list is
[IMPROVEMENTS.md](../IMPROVEMENTS.md); security findings are in [SECURITY.md](../SECURITY.md).

- **Security (code added after the 2026-11-05 review):** `inspo_capture` wrote into any folder and through a
  linked `.inspo`; AI mode could start a capture of any app. All fixed with tests. Pairing, accounts on the
  bridge, secrets in source and history, `cargo audit` and `npm audit` re-checked: nothing found.
- **Claude's phone tools** (from Claude's own 8/10 test report): `tap` refuses off-screen matches instead of
  tapping (0, 0) and can press alert buttons; `launch_app` / `terminate_app`; `logs app_only`; a compact
  `ui_tree` that lists an open alert first; `screenshot` waits for animations to end.
- **Product:** each new project gets an icon coloured from its name instead of the same blue "T".
- **Docs:** README (status, local build), CLAUDE.md (local only), SECURITY.md.
- **Not done on the phone:** the iPhone was not connected, so the tool changes ran only on the simulated
  iPhone (rows added to untested.md).

**From Claude's Water-tracker test (2026-11-10):** `install_run` now says how long it took (21.6 s on the
owner's iPhone); the compact `ui_tree` shows a SwiftUI progress bar once instead of three nested copies.
Claude saw one `app_only` log line missing; re-run on the phone (+1 three times), all three lines were there
with and without the filter, so it was not reproduced. The reset alert's "Reset" button was pressed by label
while a button with the same name was on screen behind it: the alert's was the one tapped.

## The phone appears in Claude by itself, no window (2026-11-10)

The owner wanted the iPhone in the Claude desktop app to work like the iOS Simulator beside Claude on a Mac.

- **Panel:** a Claude Code mod pane was checked again against Claude Code 2.1.321's plugin API. On the desktop
  surface a pane draws `Box`, `Text`, `Button`, `Input`, `Select`, `Link`, `Code`, `Markdown`, `Svg` and
  `Client`; `Image` and `Raster` are terminal-only, an `Svg` is drawn as a picture (131,072 characters at
  most, so no live JPEG stream), presses carry no position (no click-to-tap) and a `Client` region can't draw
  pictures. These are Claude app limits. So the Browser pane stays the panel; its address bar is the app's.
- **Starts by itself:** new Claude plugin `claude-plugin/swiftwin-phone` (local marketplace `swiftwin`). Its
  `tool.call` hook, on the first `mcp__swiftwin__*` call of a session, calls `show_phone` through
  `$.mcp.call` and opens the link with `$.mcp.call('Claude_Browser', 'preview_start')`, beside the tool
  call. If that fails, the tool result tells Claude to open it. The MCP instructions now tell Claude to call
  `show_phone` unasked, so it also works without the plugin. Tests: `claude plugin test claude-plugin/swiftwin-phone`.
- **No window:** `swiftwin-app --pane` opens no window, adds a tray icon (Open SwiftWin Studio, Quit) and
  keeps running when the Studio closes (`ExitRequested` prevented in background mode). Same process,
  single-instance lock and `app-bridge.json`, so only one app owns the phone.

## Account system polish (2026-11-10)

The owner found sign-in and account setup "a bit cheap". Kept the Liquid Glass look; added:

- **Sign-in screens:** show/hide password, live field checks (only after a field was left, so nothing
  shifts under a click), a strength meter for new passwords, a Caps Lock hint, clearer errors (no
  "invalid input:" prefix), a success state on the button, and the last email remembered ("Welcome back").
- **First sign-in:** a short welcome (create a project, connect the iPhone once, run with Claude), once
  per account on this PC, then Home.
- **Avatar menu:** the title bar and Home show the Google / GitHub picture or coloured initials; it opens
  a menu (who is signed in, Account settings, GitHub, Sign out).
- **Account dialog:** profile (picture, name editable, email, plan), sign-in method, email verified,
  member since, change password (email accounts), connected services (GitHub), sign out.
- Backend: `Accounts::update_name`, `change_password`, `avatar`; `User` gained `avatar_url`,
  `created_at`, `email_confirmed` (old `account.json` files still load). Fake Supabase maps access
  tokens to users and serves a picture.
- Not built: **delete account**. Supabase needs the service-role key for that, which can't ship in the
  app. Next step: a Supabase Edge Function (or a `security definer` SQL function) that deletes the
  caller's own user, then a "Delete account" button with a typed confirmation.

## Ready for real users and open source (2026-11-10)

- **Delete account:** `public.delete_my_account()` (`security definer`, `search_path ''`, deletes only
  `auth.uid()`, executable by `authenticated` only), called as `POST /rest/v1/rpc/delete_my_account`.
  Chosen over an Edge Function: one paste in the SQL editor, nothing to deploy, no secret key anywhere.
  Account page → Delete account, typed email confirmation; afterwards tokens, profile and the GitHub
  link are forgotten and the sign-in screen shows. A missing function (PostgREST `PGRST202`) gets a
  clear message. Tests: `delete_my_account` (fake Supabase), UI test.
- **Expired 7-day signing:** launch errors matching the 0xe8008018 family, "invalid code signature,
  inadequate entitlements or its profile has not been explicitly trusted", "could not be verified" and
  similar now say "This app's 7-day signing expired: run it again…"; WebDriverAgent's runner says to
  re-sign it with `swiftwin wda prepare`. Untested on the phone (needs an app older than 7 days).
- **Signed automatic updates** with `tauri-plugin-updater`: key pair outside the repo, public key in
  `tauri.conf.json`, URL from `SWIFTWIN_UPDATE_URL` / `config.json` / config endpoints (none yet),
  `tauri.release.conf.json` turns on update artifacts. docs/releasing.md covers latest.json, hosting,
  SmartScreen and code-signing options.
- **Docs for the owner:** docs/accounts-setup.md (SQL, Resend SMTP with DNS, publishing Google sign-in),
  PRIVACY.md / TERMS.md drafts (linked from the sign-in screen), docs/testing-checklist.md,
  docs/publishing.md (secrets and personal-data scan of files and history).
- **Open source:** MIT LICENSE, THIRD-PARTY-NOTICES.md (all linked crates are permissive or MPL-2.0;
  GPL tools only run as separate processes), CONTRIBUTING.md, a README for the GitHub page with fresh
  screenshots of the app against the mock.
- Verified: no analytics or telemetry in the code; external hosts are only those of features
  (Supabase, GitHub, Apple, picture hosts, tool downloads).

- **Signing setup in the Studio:** the logic of `swiftwin signing` moved to `swiftwin-signer::setup`
  (CLI and app share it). Signing dialog after the first real-phone connection and from the avatar menu:
  free Apple ID (limits explained, Impactor or plumesign console sign-in) or paid account (ASC key with a
  live PASS/FAIL check). Paid-team detection: plumesign never prints Apple's `xcodeFreeOnly`, so SwiftWin
  looks at the profile plumesign made for WebDriverAgent (7 days = free, a year = paid) and suggests the
  paid mode. Tests: `setup.rs`, `signing.rs`, `profile.rs`, UI test. Untested against real accounts.

## An account is required everywhere (2026-11-11)

The owner's rule: nobody uses SwiftWin without a signed-in account, also once it's open source. The app stops
on its sign-in screen (or, in a copy built without the account service, on "An account is required"), and the
CLI refuses every real command, `swiftwin mcp` included, until `swiftwin login` or a sign-in in the app. Free:
`login`, `logout`, `whoami`, `doctor`, `setup`, `ui`, `demo` and anything on the simulated iPhone (`--mock`), so
development and the tests still work. Debug builds skip the check when `SWIFTWIN_TEST_NO_ACCOUNT` is set (the
CLI's tests); release builds ignore it. Limit: in open source anyone can remove the check from the code; only
features that run on a server can be truly enforced.

- **One download, no Impactor:** the app signs in with a free Apple ID itself: Apple ID, password and
  2FA code in the Signing dialog, passed to plumesign's `account login` through stdin (prompts checked
  against plumesign 2.7.2's source; tested with a fake plumesign). plumesign stays a pinned download on
  first use (docs/dependencies.md says why). New **Phone control** step: explains WebDriverAgent,
  installs it with progress, shows how to trust the developer and checks it starts; it opens after an
  Apple sign-in, on connecting a phone without it, and when a tap fails because it is missing or expired.
## Account optional: only for cloud features (2026-11-11)

The owner changed the rule above: SwiftWin stays MIT and an account is needed **only** for the cloud
features, cloud TestFlight builds (`release`, the MCP `release` tool) and Upload to GitHub. Building and
running on the iPhone, the phone in Claude, Inspo and AI mode work without one.

- App: the sign-in screen has **Continue without an account** (remembered on this PC; cleared when someone
  signs in); a copy without the account service opens Home; signed out, the avatar menu offers **Sign in**
  and Signing; Upload asks to sign in first, with **Not now**.
- CLI: `require_account` is gone; `require_account_for_cloud` runs only for `release` and `upload` and in
  the MCP `release` backend (`CloudRelease`). `SWIFTWIN_TEST_NO_ACCOUNT` was removed (no test needs it).
  Test: `only_cloud_commands_need_an_account`; UI test steps for skip, remembered choice and Sign in later.
- Sign-in methods follow Supabase's public `/auth/v1/settings`: with the Email provider off the screen
  shows only Google and GitHub (owner's choice, 2026-11-11: no email sign-up for now).

- **On the owner's iPhone (2026-11-11, installed build):** through the app backend, `signing_apple_id` with
  plumesign's saved session, then `connect`, `wda_install` (download, plumesign sign + install, rcodesign
  re-sign) and `wda_check` all passed; `swiftwin run` installed the Hello app and two taps on "+1" counted
  2 glasses. Not yet tried: the password + 2FA screens (a session existed) and the dialog's own buttons.

## AI mode: connect your own agent (Claude Code or Codex) (2026-11-11)

The owner asked to drop AI mode's built-in chat and keep only the "connect" box, for Claude Code and
OpenAI Codex.

- Removed: the chat (header, transcript, composer), RetroBot (`mascot.js`, `retrobot.avatar.json`), the
  `claude_*` app commands and `swiftwin-app-core/src/claude.rs`, `app/tests/fake-claude.mjs`.
- AI mode's left panel now shows two command pills with copy buttons:
  `claude mcp add swiftwin -- swiftwin mcp` and `codex mcp add swiftwin -- swiftwin mcp` (syntax from
  OpenAI's Codex MCP docs, 2026-11-11; Codex isn't installed on this PC, so the command itself is unverified).
- The phone's pill, status bar and activity tab say "AI agent" (the bridge can't tell which agent called).
- Inspo's "Apply with Claude" became "Copy prompt for your AI agent" (copies the prompt, opens AI mode).
- UI test: chat and mascot checks out; checks for both commands and both copy buttons in.

## Set up this PC: one button for the prerequisites (2026-11-11)

A friend's PC worked once they had installed the prerequisites by hand (first-run A4, A5, B4). Now:

- `swiftwin-doctor/src/setup.rs`: six steps (Apple's USB service, Python, Swift 6.2.4, LLVM 22.1.8,
  helper tools, iOS SDK), each probed first so reruns skip what is there. winget installs, pinned to the
  versions on the owner's PC (`Apple.iTunes`, `Python.Python.3.12`, `Swift.Toolchain 6.2.4`, `LLVM.LLVM 22.1.8`;
  all checked with `winget show` on 2026-11-11; the iTunes manifest downloads from `secure-appldnld.apple.com`).
  No winget, failed installs and "installed but needs a restart" each get a message with the fix. Tests use
  `FakeSetupEnv`.
- iOS SDK: Apple's own sign-in page in an incognito Tauri window; its Xcode 26.3 download is cancelled and
  done by SwiftWin with the window's cookies (in memory only): free-space check (30 GB), resumable
  `.part`, `import_xip`, then the `.xip` is deleted. Apple's signed-out redirect was checked; the signed-in
  part is UNTESTED-ON-DEVICE (open question 32).
- App: **Set up this PC** dialog (`app/src/setup.js`) opens at start when something is missing (not with
  the simulated iPhone), and from the avatar menu and the Doctor dialog. `setup_*` is refused on the MCP
  bridge. CLI: `swiftwin setup` (`--check`) replaces the old "install pymobiledevice3" command.
- `swiftwin setup --check` on the owner's PC: all six DONE. UI test: the mock PC installs and "downloads".
- Installer: unchanged. Its finish page starts the app, which opens the setup; a silent `/S` install doesn't start it.

## Phone page toolbar (2026-11-12)

The phone-only page (phone window and Claude's Browser pane) has a toolbar in the dock style: Home, Run,
Screenshot, Inspo, Signing (warning dot from `wda_status` / `signing_status`), SwiftWin Studio and Account.
Where account commands answer (phone window), Account opens the avatar menu and Signing the Signing
dialog in the page. On the MCP bridge they are refused as before, so both call the new `show_studio`
command, which the desktop app fills in to focus its Studio window and open that page (unknown pages are
refused). The account menu opens above a button near the bottom. UI test: every button, tooltips, the
avatar, menu placement, the warning dot, Signing / Inspo dialogs, the Studio hint, the 300 px fit, and
bridge mode (account commands refused) where Account goes to the Studio.
UNTESTED-ON-DEVICE: `show_studio` focusing the real Studio window (needs the installed app; a Studio
window opened just then may miss the page).

## MCP fixes from the Tempo report (2026-11-12)

Claude Code built a full app (Tempo) through the MCP tools and reported problems. Fixed:

- **App logs came back empty.** When checked, `syslog live` was running and `app_only` found the app's
  subsystem as before (Tempo logs with `Logger(subsystem: "<bundle id>")`, which the prefix match covers,
  also for an id the free-account signer extended). What matches the report (old lines with a frozen
  timestamp) is a stream that went silent while its process stayed alive: nothing restarted it, and the
  desktop app's log follower waited forever. Now a supervisor restarts `syslog live` when it ends or sends
  nothing for 20 s (a phone sends thousands of lines a second), writes a `swiftwin` error line into the log
  (so the app's log panel shows it) and passes pymobiledevice3's own error lines through. Every `logs` reply
  starts with `since=N` (pass it back to read only newer lines) and warns about silence or restarts.
- **`screenshot` ignored `scale`:** `scale` (0.1-1) works, `max_width` is documented.
- **Slow first build (293 s vs 42 s):** each project had its own 150 MB clang/Swift module cache
  (`build/ModuleCache`), so every new project compiled SwiftUI, UIKit, Foundation… again. All projects now
  share `%LOCALAPPDATA%\SwiftWin\data\ModuleCache\<SDK version>`. The first build with a new SDK is still
  slow once and says so; it is warmed by that build (old per-project caches are not copied over, to be safe: clang modules may
  remember their cache path). `install_run` returns the compiler output and compile / sign / install /
  launch times; the app's AI-mode pill shows the current step ("Compiling Tempo…").
- **`swiftwin new Tempo --dir .`** in a folder named Tempo made `Tempo/Tempo` (moving it up left
  `.gitignore` behind). A folder already named like the project now gets the project itself; a folder with
  `Sources/` is refused. Test: `info_keys_and_errors`.

New tools: `wait_for` (label or text, appear or disappear; `timeout=0` is an assertion), `crash_reports`
(list and read, only inside `build/crashes`), `appearance` (dark/light via WDA), `launch_app text_size`
(Dynamic Type for one launch, the phone's setting stays). `type` reports the focused element and types
nothing without one; `tap` refuses a label behind the keyboard and warns for points on the keyboard or
outside the safe area (fixed insets of Face ID iPhones). The compact `ui_tree` shows a NavigationLink card
(a Button holding a same-frame Button) once. Tests: `tempo_report_tools` (mock MCP session),
`a_stalled_log_stream_is_restarted`, `since_reads_only_newer_entries`,
`compact_collapses_a_button_in_a_same_frame_button`. The phone was unplugged during this work: the new
WDA calls are UNTESTED-ON-DEVICE (docs/untested.md, open questions 33-34).

## Quality pass: small bugs (2026-11-12)

Clicked through every screen against the mock (`swiftwin --mock ui` + the Browser pane), read the
UI code, every CLI `--help` and the MCP tool list. Found and fixed:

- **Lost edits, project switch:** opening or creating another project dropped unsaved edits; worse,
  New project switched the backend first, so "Save" wrote the old project's edits into the new
  project's files. `keepEdits()` now asks before both (UI test covers it).
- **Lost edits, closing the window:** the desktop window closed without asking (no `beforeunload`
  in a Tauri window). It now asks through `onCloseRequested` (capability `core:window:allow-destroy`).
- **Toasts covered the iPhone** and its toolbar (bottom right). Now bottom left; top in the phone window.
- **Phone page after a SwiftWin restart:** the page kept its old token and every call (several a
  second) failed with "bad or missing X-SwiftWin-Token". It now fetches a fresh token once.
- **Builds without accounts** (the open-source build): the avatar jumped straight into Signing and
  "Set up this PC" was unreachable from it. It now opens the menu without "Sign in".
- Inspo "Add theme" never pulsed (`ev.currentTarget` read after an `await`); a Phone-control error
  showed literal line breaks.
- CLI: every argument has help text; `signing` mentions the free Apple ID; `mcp` names Codex too;
  `sdk download --xcode` example is 26.3.
- MCP: `show_phone` and `inspo_capture` descriptions had runs of 17 spaces; parameter descriptions for
  `release`, `tap` label/x/y, `crash_reports` file/all, `logs` app_only.
- Docs: untested.md still mentioned the RetroBot mascot; security.md lists the new capability.

Second round (Rust audit, real window):

- **Two builds at once:** the app's Run and Claude's `install_run` share one `Workspace` and both
  wrote the project's build folder. `Workspace` now refuses a second build / install_run /
  WebDriverAgent setup while one runs ("another build is running (Compiling…)"). Test added.
- **Config could be wiped:** `config.json` was rewritten in place while `swiftwin mcp` reads it on
  every build; a half-written file loaded as the default config and could be saved back that way,
  losing the signing setup. New `paths::write_atomic` (temp file + rename), also used for the
  upload repo links and the editor's Save (a crash mid-save truncated the source file). Test added.
- Offline: the GitHub block in Account / Upload stayed empty (Upload stopped half-way) when
  `github_status` failed; it now shows the error with Connect.
- Checked in the installed window (WebView2 debugging port, `--mock`): closing with an unsaved
  edit asks "Save your changes before closing SwiftWin?", Save writes the file and the window
  closes; without edits it closes at once; two launches leave one process.
- Audited and found sound: the local HTTP server (token, host/origin checks, limits, one-time
  pairing), project paths (`join_inside`), SDK extraction (cpio paths and links stay inside the
  SDK), the upload secret filter, the Apple ID password (plumesign stdin only), setup commands,
  MCP forwarding and tool dispatch.
- Not fixed (small): the log panel's index can skip a few hundred lines once after a reconnect or
  when the 20,000-line buffer is trimmed. Not audited line by line: the real-device code
  (`swiftwin-device/src/real`, all UNTESTED-ON-DEVICE), the SDK download and the signer's Apple
  API calls.
