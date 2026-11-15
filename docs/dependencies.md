# External dependencies

Tools SwiftWin shells out to (prototype shortcuts allowed by the ground rules) and notable crates.
Each entry: what it is used for, license, and the plan (keep or replace).

## External tools

| Tool | Used for | License | Plan |
|---|---|---|---|
| Apple Devices app (Microsoft Store) or iTunes | Apple USB driver + Apple Mobile Device Service (usbmuxd on 127.0.0.1:27015) | Apple proprietary, free | **Keep** – required on Windows; the user installs it, SwiftWin never redistributes it. Users who sideload need the **web** iTunes (`Apple.iTunes`) instead of the Store app |
| [LLVM](https://github.com/llvm/llvm-project) `ld64.lld` (upstream, e.g. 22.1.8 via `winget install LLVM.LLVM`) | linking iOS executables; Swift's own `ld64.lld` refuses Apple-platform objects | Apache-2.0 with LLVM exception (separate executable, user-installed) | **Keep** – found via `SWIFTWIN_LD64`, `%ProgramFiles%\LLVM\bin`, PATH |
| [Impactor](https://github.com/CLARATION/Impactor) or [Sideloadly](https://sideloadly.io) | signing + installing with a **free** Apple ID by hand (unsigned `.ipa` files from `swiftwin build --ipa`, `swiftwin wda ipa`) | Impactor MIT; Sideloadly proprietary freeware | **Optional, user-installed.** Not needed any more: SwiftWin signs in and installs through plumesign itself |
| plumesign 2.7.2 (`plumesign-windows-x86_64.exe` from the Impactor release) | `swiftwin run` / `swiftwin wda prepare` with a free Apple ID: Apple developer services login session, free certificate + profile, device registration, install over usbmuxd | MIT (separate executable, downloaded by `swiftwin signing apple-id` / `doctor --fix`, pinned SHA-256 `b965e51a…2fe0`) | **Prototype.** 2.7.0+ is required since Apple's 2026-10-12 sign-in change (older versions fail with SRP error -80045). Uses the remote anisette server `mac-ani.jkcoxson.com` for Apple's device-provisioning headers unless a local StoreServicesCore library is present. Could become a library dependency (its crates are Rust). **Downloaded on first use, not bundled** in the installer: the sign-in needs the internet anyway, it is pinned by SHA-256 like rcodesign and WebDriverAgent, the installer stays small, and a forced update after an Apple change (like -80045) only needs a new pin in an app update either way. The app drives `account login` through stdin (password and 2FA code from its own dialog) |
| [pymobiledevice3](https://github.com/doronz88/pymobiledevice3) 11.49.5 (Python 3.11+) | authenticated lockdown, iOS 17+ tunnel, DDI mount, install, launch, screenshot, syslog, crash reports, XCTest (WDA) | GPL-3.0 (run as a separate process, not linked) | **Prototype.** Replace step by step with native Rust (usbmux is already native). Highest value to replace first: screenshot + syslog (used by mirroring). Installed into SwiftWin's own venv by `swiftwin setup`/`swiftwin doctor --fix`. |

## Rust crates (notable)

| Crate | Used for | License |
|---|---|---|
| serde / serde_json | serialization | MIT/Apache-2.0 |
| clap | CLI parsing | MIT/Apache-2.0 |
| plist | Info.plist / usbmuxd / lockdown plists | MIT |
| png | encoding mock screenshots | MIT/Apache-2.0 |
| zip | reading/writing .ipa files | MIT |

## Signing tools

| Tool | Used for | License | Plan |
|---|---|---|---|
| [rcodesign / apple-codesign](https://github.com/indygreg/apple-platform-rs) 0.29.0 | Mach-O + bundle code signing on Windows | MPL-2.0 (separate executable, downloaded by `swiftwin doctor --fix`, pinned SHA-256) | **Keep** for now; could be linked as a library later (large dependency tree) |
| [WebDriverAgent](https://github.com/appium/WebDriverAgent) v16.15.3 `WebDriverAgentRunner-Runner.zip` | on-phone UI automation server | BSD-3-Clause (download pinned by SHA-256) | **Keep** (re-signed per user; building WDA from source needs Xcode) |
| App Store Connect API | certificates, App IDs, devices, profiles | Apple service (paid program) | **Keep** |

Rust crates added: `p256` (ES256 JWT), `rsa` + `sha2` + `rand` (key/CSR), `keyring` (Windows only,
Credential Manager) – all MIT/Apache-2.0.

## Mirroring tools

| Tool | Used for | License | Plan |
|---|---|---|---|
| [UxPlay](https://github.com/FDH2/UxPlay) | AirPlay mirroring receiver (smooth video view) | GPL-3.0 (separate process) | **Prototype.** Optional – USB mirroring works without it. Needs Bonjour (Apple "Bonjour Print Services" or the Bonjour that comes with iTunes) and GStreamer (MSYS2 build). Replace later with a native receiver if worth it (FairPlay handshake is the hard part). Located via `SWIFTWIN_UXPLAY`, `%LOCALAPPDATA%\SwiftWin\data\tools\uxplay\uxplay.exe`, or PATH. |

## Desktop app

| Package | Used for | License |
|---|---|---|
| Tauri 2 (`tauri`, `@tauri-apps/cli`, `@tauri-apps/api`) | desktop shell, NSIS installer | MIT/Apache-2.0 |
| `tauri-plugin-updater` 2 | signed automatic updates (docs/releasing.md) | MIT/Apache-2.0 |
| `tauri-plugin-single-instance` 2 | one app per user, so a second launch can't take over the MCP bridge | MIT/Apache-2.0 |
| [glisse](https://github.com/0kzh/glisse) 0.5.0 (commit `46be5ba`), vendored in `app/src/vendor/glisse/` | Liquid Glass look: frost, lit rims, refraction through SVG displacement maps (full in WebView2/Chromium), iOS press springs | MIT; only its React-free core is vendored, its two hooks rewritten as plain functions (see the folder's README) |
| Vite 6 | front-end build | MIT |
| monaco-editor 0.52 | code editor | MIT |
| Microsoft Edge WebView2 runtime | renders the UI (preinstalled on Windows 10/11; the NSIS installer bootstraps it if missing) | Microsoft |
| Playwright (CI/dev only, not shipped) | UI test against the mock phone | Apache-2.0 |

## Accounts and GitHub upload

| Service / crate | Used for | License / cost | Plan |
|---|---|---|---|
| [Supabase Auth](https://supabase.com/docs/guides/auth) (hosted GoTrue, [source](https://github.com/supabase/auth)) | accounts: email + password with email codes, Google and GitHub sign-in (PKCE), refresh tokens | GoTrue MIT; hosted service, free plan | **Keep.** No SwiftWin server. Talked to over its REST API (`crates/swiftwin-account/src/supabase.rs`), no SDK. Needs custom SMTP before real users |
| GitHub OAuth app + [device flow](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow) | "Continue with GitHub" (through Supabase) and the `repo` token for uploads | free | **Keep.** A GitHub App with per-repository access would ask for less than `repo`; worth it once SwiftWin has many users |
| Google OAuth client (Google Auth Platform) | "Continue with Google" (through Supabase) | free | **Keep** |
| Gmail SMTP (a dedicated SwiftWin Gmail account), or [Resend](https://resend.com) with an own domain later | sign-up codes, confirmation and password-reset emails sent by Supabase Auth | free | **Gmail chosen** (docs/accounts-setup.md 2-free); configured in Supabase only, SwiftWin doesn't talk to it |
| `ignore` 0.4 | reading `.gitignore` rules when collecting files to upload | MIT/Unlicense | Keep |
| `sha1` 0.10 | git blob ids, so unchanged files aren't uploaded again | MIT/Apache-2.0 | Keep |
| `open` 5 | opening the system browser (ShellExecute) | MIT | Keep |
| `url` 2 | building and parsing sign-in URLs | MIT/Apache-2.0 | Keep |
| `rpassword` 7 | `swiftwin login` password prompt without echo | Apache-2.0 | Keep |

## Cloud release

| Tool / service | Used for | License | Plan |
|---|---|---|---|
| GitHub Actions macOS runner (`macos-26`) | Xcode 26 archive + TestFlight upload | GitHub service (macOS minutes are billed 10x on private repos) | **Keep** – Apple requires Xcode for uploads |
| [XcodeGen](https://github.com/yonaskolb/XcodeGen) (installed on the runner with Homebrew) | generate the .xcodeproj from `swiftwin.toml` | MIT | **Keep** |
| `crypto_box` crate (sealed boxes) | encrypt repository secrets | MIT/Apache-2.0 | Keep |
