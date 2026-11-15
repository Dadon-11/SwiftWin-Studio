# SwiftWin

SwiftWin is a Windows desktop app that lets a developer (and Claude Code) build, run and test SwiftUI iOS apps without a Mac, using a real iPhone connected over USB as the test device.


## Final goal

1. User opens SwiftWin on Windows. It asks them to connect their iPhone.
2. Once connected, SwiftWin shows a live, controllable mirror of the phone (click = tap, scroll = swipe, keyboard = typing), next to a code editor and logs.
3. SwiftWin builds, signs and installs SwiftUI apps on the phone with no Mac.
4. Claude Code gets the same powers through SwiftWin's MCP server: build, screenshot, ui_tree, tap, swipe, type, logs, release.
5. A cloud Mac is used only for App Store / TestFlight builds (Apple requires Xcode 26+ for uploads).

## Architecture

- **SwiftWin core (Rust):**
  - `sdk` ג€“ downloads Xcode.xip with the user's Apple ID, extracts only the iOS SDK, packages it as a Swift SDK. Never bundle or redistribute Apple's SDK.
  - `builder` ג€“ compiles Swift with the open-source swift.org toolchain targeting `arm64-apple-ios`, assembles the .app bundle (Info.plist, entitlements, resources, asset catalogs).
  - `signer` ג€“ Apple developer services: certificates, App IDs, provisioning profiles; code-signs the bundle.
  - `device` ג€“ device bridge: talks to the iPhone over USB (usbmuxd / lockdown, iOS 17+ tunnel), installs, launches, screenshots, syslog, crash reports.
  - `control` ג€“ drives WebDriverAgent on the phone: tap, swipe, type, Home, UI element tree.
  - `mirror` ג€“ hybrid mirroring: AirPlay receiver for smooth video (user view), USB screenshots for exact frames (Claude's view), automatic fallback between them.
- **Desktop app:** Tauri + web UI, Monaco editor, phone window, log panel.
- **MCP server:** exposes the core to Claude Code.

## Milestones (Phase 1: build all of it, no phone)

1. **Foundation:** Rust workspace, `swiftwin` CLI, `Device` trait with `MockDevice`, CI producing Windows builds.
2. **Device bridge (RealDevice):** detect USB iPhone, trust, device info, install .ipa, launch, screenshot, syslog, crash logs.
3. **Control:** WebDriverAgent install and drive: tap, swipe, type, Home, UI tree.
4. **SDK + builder:** download Xcode.xip with the user's Apple ID, extract the iOS SDK, compile a Hello SwiftUI app to a .app bundle, asset catalogs.
5. **Signer:** certificates, App IDs, provisioning profiles, code signing; also builds WebDriverAgent.
6. **Mirroring:** USB screenshot stream, AirPlay receiver, automatic fallback.
7. **MCP server:** build, install_run, screenshot, ui_tree, tap, swipe, type, logs, release.
8. **Desktop app (Tauri):** onboarding ("connect your iPhone"), file tree, Monaco editor, phone window with click-to-tap, log panel, status bar.
9. **Cloud release:** send a project to a macOS GitHub Actions runner for an Xcode 26 TestFlight build.
10. **Doctor + installer:** `swiftwin doctor`, Windows installer, `docs/first-run.md` for connecting the phone for the first time.

Each milestone ends with: CI green, `--mock` demo working, docs updated.

## Development workflow (important)

> **Since 2026-11-15 SwiftWin is public at https://github.com/Dadon-11/SwiftWin-Studio**, published from
> cleaned snapshots (`scripts/publish-snapshot.ps1`, docs/publishing.md). The owner's working repository
> keeps its private history and has no remote on purpose: never add a remote to it or push it. Public
> updates go out only from the `swiftwin-public` snapshot folder, and only when the owner asks. Where the text below says "push" or "CI", commit locally and run
> `cargo test --workspace` plus the UI test instead; build the installer on this PC (see README,
> "Build, test and install").

Phase 1 is building the **complete app without an iPhone**. The owner will connect a real iPhone only after the app is finished.

- Claude Code works on this GitHub repo from a machine with no iPhone attached. Commit and push often to a branch, with clear commit messages.
- `.github/workflows/ci.yml` builds and tests everything on `windows-latest` on every push. Windows-only code must pass there; check CI results with `gh run list` / `gh run view`.
- CI also builds a **release** of the CLI and the Tauri desktop app and uploads them as the artifact `swiftwin-windows`. The owner downloads that and runs it on their own PC.
- Because no phone is available:
  - Put every device operation behind a `Device` trait in `swiftwin-device`, with two backends: `RealDevice` (USB, the real thing) and `MockDevice` (simulated iPhone that returns sample screenshots, logs, a UI tree, and accepts taps/installs).
  - Run the app with `--mock` (or `SWIFTWIN_MOCK=1`) to use `MockDevice`. All UI, MCP and integration tests use the mock.
  - Write `RealDevice` carefully against the documented protocols and existing open-source implementations. Mark every path that has never run against a real phone with `// UNTESTED-ON-DEVICE` and list them in `docs/untested.md`.
  - Build `swiftwin doctor`: a step-by-step check run on first connection (USB driver, device visible, trust, Developer Mode, iOS version, tunnel, developer disk image, screenshot, WebDriverAgent, signing). Each step prints PASS/FAIL with the exact error, so the first real-phone session is quick to debug.
- Same for Apple accounts: the signer and SDK download need the owner's Apple ID. Build them fully, test the logic with recorded/fake responses, and mark them untested.

## Ground rules

- Target platform is Windows 10/11 (x64). Every feature must work natively on Windows; WSL is allowed only as a temporary prototype shortcut and must be marked as such.
- Prove risky parts with the smallest possible spike before building structure around them; when a spike needs a real phone, write it, mark it untested, and move on.
- Prototypes may shell out to existing open-source tools (pymobiledevice3, go-ios, UxPlay) to de-risk; record each such dependency in `docs/dependencies.md` with the plan to replace or keep it, and its license.
- Verify claims about Apple protocols, crates and tools against their current docs/repos before relying on them. Note anything unverified in `docs/open-questions.md`.
- Keep a `docs/progress.md` log: what works, what was tried, what failed.
- Never commit Apple credentials, certificates, private keys or the extracted SDK. Store secrets in the Windows Credential Manager.

## Repo layout (target)

```
swiftwin/
  crates/
    swiftwin-core/      # shared types, errors
    swiftwin-device/    # milestone 1
    swiftwin-control/
    swiftwin-sdk/
    swiftwin-builder/
    swiftwin-signer/
    swiftwin-mirror/
    swiftwin-mcp/
  cli/                # package `swiftwin-cli`, binary `swiftwin`
  app/                # Tauri desktop app (later)
  docs/
```

## Windows prerequisites

- Rust (stable, MSVC toolchain)
- Apple Devices app from the Microsoft Store (or iTunes) ג€” provides Apple's USB driver and the Apple Mobile Device Service
- Python 3.11+ (only while prototypes use pymobiledevice3)
- Swift 6.2.4 for Windows (matches Xcode 26.3) and LLVM (`winget install LLVM.LLVM`) for the `ld64.lld` linker
- Install SwiftWin from a normal PowerShell, not from a process started by the Claude desktop app: that app sandboxes newly created top-level AppData folders (see docs/open-questions.md, 23)
- iPhone with Developer Mode on (Settings > Privacy & Security > Developer Mode)
