# Open questions / unverified assumptions

Things that could not be verified without a real iPhone, a Mac, or an Apple account. Each one lists
how it will be checked on first connection.

## Answers so far (owner's PC, before the phone was plugged in, 2026-11-03/04)

Checked against the installed tools (pymobiledevice3 11.49.5, Swift 6.2.4, LLVM 22.1.8, rcodesign 0.29.0,
the real Apple Mobile Device Service) and upstream sources. Still needs the phone where noted.

- **q1 narrowed:** pair records on disk are named `XXXXXXXX-XXXXXXXXXXXXXXXX` (uppercase, with dash); SwiftWin tries both spellings.
- **q2 narrowed:** pymobiledevice3 itself reads `UniqueDeviceID`, `ProductType`, `ProductVersion`, `DeviceName` with an unauthenticated GetValue before pairing.
- **q3 resolved:** on iOS 17.4+ 11.49.5 retries `developer` commands over its in-process userspace tunnel: no admin, wintun or tunneld. Only iOS 17.0–17.3 needs `remote tunneld` as Administrator.
- **q4 resolved:** `ReadPairRecord` serves `C:\ProgramData\Apple\Lockdown\<UDID>.plist` under both Apple Devices and web iTunes; it ignores case but needs the dash. `lockdown pair` saves to `%USERPROFILE%\.pymobiledevice3\` instead, so SwiftWin checks both.
- **q6 narrowed:** `developer dvt xcuitest <runner bundle id> --env K=V` exists as SwiftWin calls it. It needs `CFBundleExecutable` ending in `-Runner` and finds `PlugIns/<exe minus -Runner>.xctest` by path; the xctest's bundle id is not used.
- **q7 mostly resolved:** unxip 3.3 extracts Xcode 26 in CI and diffs it against Apple's `xip --expand`: XAR → pbzx (16 MiB chunks, raw when the stored size equals the chunk size) → odc cpio, no Apple Archive. Xcode 26 has ~25k hard links and ~10k symlinks, which SwiftWin now reproduces (NTFS hard links; symlink → junction → copy). Apple DTS: the SDKs for all platforms are in the main Xcode `.xip`; the separate "Platform Support" download is simulators.
- **q9 resolved:** Xcode 26.3 bundles Swift 6.2.4 (the Windows toolchain installed) and the iOS 26.2 SDK, and Windows swiftc 6.2.4 typechecks a SwiftUI `@main` app against the imported SDK (`-target arm64-apple-ios17.0`; the SDK ships only `arm64e-apple-ios.swiftinterface` files, which arm64 builds still find). Xcode 26.4+ ships Swift 6.3 and should not be used with a 6.2 toolchain.
- **q10 resolved:** Swift 6.2.4's `ld64.lld` is patched to refuse every Apple-platform object (swiftlang/llvm-project `lld/MachO/InputFiles.cpp`, "Swift LLVM fork downstream change"). SwiftWin compiles with `swiftc -c`, strips `LC_BUILD_VERSION`, and links with upstream LLVM's `ld64.lld` (`winget install LLVM.LLVM`). iOS 17+ needs no Swift compatibility libraries; `libclang_rt.ios.a` is only needed for C `@available` checks.
- **q11 narrowed:** host plugins ObservationMacros, SwiftMacros, FoundationMacros, TestingMacros exist on Windows; SwiftUIMacros/PreviewsMacros do not.
- **q15 resolved:** send the full PEM CSR (fastlane's `cert` does `csr_content: csr.to_pem` with type `DEVELOPMENT`).
- **q17 narrowed:** rcodesign signs nested frameworks and `.xctest` bundles recursively; entitlements go on the main executable only. appium's v16.15.3 zip has the `.app` at the root, a still Apple-signed fat runner stub, an unsigned xctest/WebDriverAgentLib, a dSYM inside `PlugIns/`, and no `Frameworks/XC*` (they come from the developer disk image).

## Answered on the phone (2026-11-04, iPhone18,3, iOS 27.0.1)

- **q1 resolved:** usbmuxd (web iTunes) lists the serial with the dash (`00008150-…`), the same as lockdown's `UniqueDeviceID`.
- **q2 still open:** the phone was already trusted, so keys readable *before* trust were not checked.
- **q3 resolved:** `developer` commands retry over the in-process userspace tunnel with no admin rights. `remote rsd-info`
  has no `--udid` option; the device goes in `PYMOBILEDEVICE3_UDID`. The tunnel comes up in about 3 s.
- **q4 resolved:** `ReadPairRecord` with the dashed UDID finds the record that iTunes wrote when the user tapped Trust.
- **q6 resolved:** `developer dvt xcuitest` starts appium's WDA v16.15.3 on iOS 27; WDA answers on device port 8100 through
  SwiftWin's usbmux forwarder. The first start right after trusting the developer failed once with
  `initializationForUITestingDidFailWithError` after 60 s; the next start reached the test plan in ~30 s. Later
  commands reuse the running WDA (a tap takes about 1 s).
- **q17 resolved (free Apple ID):** rcodesign re-signing the whole runner (including `PlugIns/*.xctest`) with plumesign's
  certificate is accepted by iOS; XCTest frameworks come from the developer disk image.
- **q24 resolved:** iOS accepts and runs the re-signed runner once the user trusts the developer
  (Settings > General > VPN & Device Management > Apple Development: <Apple ID> > Trust; needs internet). Until then
  SpringBoard logs `signature state: Needs Explicit User Trust`, and the launch fails with `deviceprocesscontrolservice`
  code 2, which SwiftWin's hint already points at.

## Found on the owner's PC

23. **Claude desktop app sandbox.** Processes started from the Claude desktop app (including `swiftwin mcp`
    launched by Claude Code) get MSIX write virtualization: a *new top-level* folder under
    `AppData\Local`/`Roaming` goes to `…\Packages\Claude_*\LocalCache\…`, invisible to the rest of Windows.
    Writes into a folder that already exists for real are real. No documented breakaway works (desktop-app
    policy attributes, job breakaway); only WMI-created processes escape. Install SwiftWin from a normal
    PowerShell or by double-clicking; `swiftwin doctor` warns if a sandboxed copy exists.
24. **Free Apple ID with a sideloader.** Narrowed from Impactor's v2.6.3 and v2.7.2 sources: plumesign/Impactor map
    `.xctest` to an unknown bundle type, so they register no App ID for it (WDA costs one) but also
    **don't sign it**; `WebDriverAgentLib.framework` inside it is signed. SwiftWin's `wda prepare` therefore
    re-signs the whole runner with plumesign's certificate. Open: whether iOS accepts that, and whether
    Sideloadly does better. *Check:* `swiftwin wda prepare` with `signing = apple-id`, then `swiftwin wda start`.
25. **Sideloadly on Windows** needs web iTunes *and* web iCloud; iCloud (Legacy) 7.21 fails its launch
    conditions on this Windows 11 build (MSI 1603), and the Sideloadly installer is unsigned. Impactor
    (MIT) needs only iTunes.
26. **plumesign session lifetime.** The saved `com.apple.gs.xcode.auth` token in
    `%APPDATA%\PlumeImpactor\accounts.json` is never refreshed by the CLI; how long it lasts is not in the
    source. When `swiftwin run` starts failing with an auth error, sign in again in Impactor. *Check:* a
    `swiftwin run` a few days after signing in.

32. **Apple's sign-in → download flow (Set up this PC).** Checked 2026-11-11: signed out,
    `https://developer.apple.com/services-account/download?path=/Developer_Tools/Xcode_26.3/Xcode_26.3_Apple_silicon.xip`
    redirects (302) to `idmsa.apple.com/IDMSWebAuth/signin?...&path=<that page>`, and
    `download.developer.apple.com/...` without cookies redirects to `/unauthorized/`. Not verified: that after
    sign-in Apple returns to that page and starts the download (if it doesn't, the user can click Xcode 26.3
    on developer.apple.com/download/all in the same window; any download of the pinned file is taken),
    which cookies `download.developer.apple.com` really needs (SwiftWin sends all of that URL's cookies),
    and how long they last. *Check:* testing-checklist section 2 on a clean PC.
33. **winget and the Store apps.** Not verified: whether `winget install Apple.iTunes` is refused or
    clashes when the Store's Apple Devices is installed (setup only tells the user to uninstall it), and
    which exit code winget gives for "restart needed" (setup checks the service afterwards instead).

## Accounts and GitHub upload

Checked against Supabase Auth's source and OpenAPI spec (github.com/supabase/auth, `openapi.yaml`,
`internal/utilities/request.go`) and GitHub's OAuth docs on 2026-11-05; none of it has run against the
real services yet.

27. **Loopback redirects on hosted Supabase.** The source allows any loopback IP (and ignores the port for
    localhost) as `redirect_to`; the docs don't say whether `*` matches a port in the allow list. Setting
    the Site URL to `http://127.0.0.1` plus the redirect `http://127.0.0.1:*/auth/callback` should cover
    both. *Check:* "Continue with Google" on the owner's PC; if the browser lands on the Site URL instead
    of "You're signed in", the redirect was refused.
28. **Publishable keys.** SwiftWin sends the key only as the `apikey` header (not as a bearer token);
    `/authorize` is opened in the browser without a key, as supabase-js does. *Check:* the first sign-in.
29. **Email links (PKCE).** Sign-up and password reset send `code_challenge` and `?redirect_to=` (the
    app's loopback address) like supabase-js, so the email's link should come back to the app with
    `?code=`. Supabase now only lets you edit templates with custom SMTP, so the default link-only emails
    are what the owner gets until then. The 6-digit code path (`POST /verify`) stays for templates that
    show `{{ .Token }}`. *Check:* sign up with a new address and open the link on the same PC.
30. **Refresh-token rotation.** Supabase revokes a session when an old refresh token is reused (outside
    a short grace period). The app and the CLI share one stored token, so `swiftwin whoami` never
    refreshes, and a refresh always re-reads the stored token first. If the app and `swiftwin login`
    refresh at the same moment, one of them may be signed out. *Check:* daily use.
31. **GitHub device-flow polling** honours `interval` and `slow_down` (+5 s). OAuth app tokens don't
    expire unless the app opts in to expiring tokens; SwiftWin assumes they don't. *Check:* Connect
    GitHub, then upload a week later.

## Device bridge (M2)

1. **usbmuxd SerialNumber format on Windows.** For newer iPhones the UDID contains a dash
   (`00008110-...`). Apple's Windows service may report it with or without the dash. SwiftWin compares
   UDIDs ignoring dashes and prefers the lockdown `UniqueDeviceID`. *Check:* `swiftwin devices` vs
   `swiftwin info` on first connection.
2. **Which lockdown keys are readable before pairing.** Assumed: ProductType, ProductVersion,
   UniqueDeviceID, DeviceName. If not, `info` falls back to defaults until trust. *Check:* `swiftwin info`
   before tapping Trust.
3. **pymobiledevice3 on Windows with iOS 17+.** Its 11.x help says developer commands "build their own
   no-root tunnel". Not verified on Windows. Fallback: run `pymobiledevice3 remote tunneld` as admin
   and pass `--tunnel ''`. *Check:* `swiftwin doctor` tunnel step.
4. **Pair records on Windows** live in `C:\\ProgramData\\Apple\\Lockdown`; ReadPairRecord via usbmuxd is
   assumed to work the same as on macOS. *Check:* `swiftwin devices` shows "trusted" after tapping Trust.

## Control (M3)

5. **Swipe velocity.** `dragfromtoforduration` uses XCUIGestureVelocityDefault; long lists may need
   `pressAndDragWithVelocity` for fast flicks. *Check:* scroll a long list from the phone window.
6. **WDA on iOS 17+ via pymobiledevice3 `xcuitest`.** Assumed to work like on macOS with Xcode. If
   not, alternative: go-ios `ios runwda` (MIT). *Check:* `swiftwin wda start`.

## SDK + builder (M4)

7. **Xcode .xip internals.** Assumed: XAR with an uncompressed `Content` entry holding pbzx whose
   chunks are XZ (or raw when incompressible) and decode to a cpio *odc* stream, as documented by
   the `unxip`/`xcodes` projects. If Apple switched to Apple Archive (`.aar`/`AA01` magic) for newer
   Xcodes, the extractor reports "not a pbzx stream". *Check:* `swiftwin sdk import` on Xcode 26.
   Fallback: extract with 7-Zip/`xar` on Windows, then point SwiftWin at the folder.
8. **XIP signature** is not verified (it is checked by macOS's `xip` tool). SwiftWin only downloads from
   download.developer.apple.com over HTTPS.
9. **Swift toolchain ↔ SDK versions.** The SDK's `.swiftinterface` files must be readable by the
   Windows swiftc. Use the swift.org toolchain matching Xcode's Swift (Xcode 26 = Swift 6.2).
10. **Linking Mach-O on Windows** with `-use-ld=lld` (ld64.lld from the swift.org toolchain) and
    whether `libclang_rt.ios.a` (needed for `@available`/`#available`) is required/absent.
11. **SwiftUI macros** (`#Preview`, `@Observable`) need compiler plugins built for the host; Windows
    toolchains may lack Apple's SwiftUI macro plugins. The Hello template avoids them.
12. **Asset catalogs without `Assets.car`**: icons via `CFBundleIconFiles`, images via loose files,
    colors via generated Swift. `Color("Brand")` and `AccentColor` will not work – use the generated
    `Color.brand` / `.tint(...)`.
13. **Apple ID sign-in automation**: SwiftWin does not implement Apple's SRP login (fragile and 2FA
    heavy). The desktop app's sign-in window gets the session cookie instead.

## Signer (M5)

14. **Paid vs free Apple ID.** The App Store Connect API needs a paid Apple Developer Program account.
    Free "Personal Team" signing uses Xcode's private developer services API (what AltStore/Sideloadly
    use) – not implemented. *Decision needed from the owner if they only have a free account.*
15. **`csrContent` format**: SwiftWin sends the full PEM text. If Apple wants only the base64 body,
    `create_dev_certificate` needs a one-line change. *Check:* first `swiftwin run`.
16. **Certificate limit**: Apple allows a small number of development certificates; SwiftWin reuses its
    own (id stored in `%LOCALAPPDATA%\SwiftWin\data\signing\certificate.json`). If creation fails with a limit
    error, revoke an old one in the developer portal.
17. **rcodesign + nested WDA bundles**: assumed that signing the runner `.app` recursively signs
    `PlugIns/*.xctest` and `Frameworks/*` correctly without separate entitlements.

## Mirroring (M6)

18. **UxPlay `-vs` with a pipeline fragment.** UxPlay passes `-vs` into its GStreamer launch string;
    assumed that `jpegenc ! multipartmux ! tcpclientsink` works there. If not, alternative: UxPlay's
    `-vrtp` (forward raw H.264 RTP) and decode in SwiftWin, or capture UxPlay's window.
19. **AirPlay on the owner's network**: phone and PC must be on the same Wi-Fi, mDNS allowed by the
    Windows firewall (UDP 5353) plus UxPlay's ports.

## Cloud release (M9)

20. **`macos-26` runner label** and the Xcode 26 path on it (`/Applications/Xcode_26*.app`) – based on
    GitHub's naming scheme for runner images; if the label differs, change `runs_on` in
    `templates.rs` (e.g. `macos-15` with Xcode 26 installed). *Check:* first `swiftwin release`.
21. **App record**: the App Store Connect API cannot create the app itself; the owner creates it once
    (My Apps > + > New App) with the same bundle id.
22. **Export method name** `app-store-connect` (Xcode 15.3+); older name `app-store`.

## MCP tools (Tempo report, 2026-11-12)

33. **`accessibilityHidden` elements in WDA's source.** XCUITest normally leaves them out, so `ui_tree`
    should not show them; Tempo hides one view (`Store.swift`). *Check:* `ui_tree` on Tempo's screen with
    that view, next time the phone is connected.
34. **Dark mode and text size without changing the app.** `appearance` uses WDA's
    `/wda/device/appearance` (sets `XCUIDevice.appearance`; no simulator-only check in WDA's code, but not
    tried on a real phone). WDA has no Dynamic Type route, so `launch_app text_size` passes
    `-UIPreferredContentSizeCategoryName <category>` as a launch argument, which UIKit reads from user
    defaults (assumed to apply to SwiftUI's `dynamicTypeSize` too). *Check:* both on Tempo.
