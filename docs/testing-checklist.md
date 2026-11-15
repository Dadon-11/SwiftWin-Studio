# Test checklist: another PC, another iPhone

SwiftWin has been confirmed on one PC and one iPhone (iPhone18,3, iOS 27, free Apple ID). Run this on a
**different Windows PC** with a **different iPhone model or iOS version** (iOS 17.0–17.3 and 18 are the
most useful, since they use other paths). Takes about an hour. A friend can do it: nothing here needs
the owner's accounts except the optional parts marked (owner).

Write down for each step: ✅ / ❌, how long it took, and the exact error text or a screenshot. Collect
`swiftwin doctor` output and `%LOCALAPPDATA%\SwiftWin\data\*.log` if something fails.

## 0. Record the setup

- [ ] Windows edition and version (`winver`), x64, normal user or admin
- [ ] iPhone model and iOS version (Settings → General → About)
- [ ] Apple ID type: free / paid developer account
- [ ] Apple Devices app or iTunes installed? Which?

## 1. Install

- [ ] Copy `SwiftWin_<version>_x64-setup.exe` to the PC (USB stick or download).
- [ ] Double-click it. If "Windows protected your PC" shows: **More info → Run anyway** (expected:
      the installer isn't code-signed yet). Note whether it showed.
- [ ] The installer finishes without asking for admin rights.
- [ ] If the PC had no WebView2: it is installed by the installer (needs internet).
- [ ] Start menu → **SwiftWin** opens the app.
- [ ] A new PowerShell: `swiftwin --version` works (the installer's `bin` folder is on PATH, or follow
      first-run A3).

## 2. Doctor, before the phone

- [ ] `swiftwin doctor` prints PASS/FAIL per step; FAILs say what to do.
- [ ] **Best on a clean PC** (no iTunes, Python, Swift or LLVM yet): the app opens **Set up this PC** by
      itself. Note which items it lists as TO DO.
- [ ] **Set up everything**: each item turns DONE; accept the Windows permission prompts (iTunes, Swift,
      LLVM). Note any FAIL text, and whether iTunes asked for a restart. After a restart, open the app
      again: done items are skipped.
- [ ] The Apple sign-in window opens after the other items. Sign in; the Xcode 26.3 download should start
      by itself and show progress in the dialog. Note the time, and whether the 2FA prompt worked there.
- [ ] Close the app mid-download, reopen, press the sign-in button again: the download resumes.
- [ ] The SDK item turns DONE and `%LOCALAPPDATA%\SwiftWin\data\sdk\downloads` is empty (the .xip is gone).
- [ ] `swiftwin setup --check` in a new PowerShell lists everything DONE.

## 3. Sign in

- [ ] The app opens on the sign-in screen. **Terms** and **Privacy Policy** open and are readable.
- [ ] **Continue without an account** works (the app opens Home; everything except cloud builds and
      Upload works).
- [ ] **Continue with Google**: until Google sign-in is published, only Gmail addresses the owner added as
      test users can sign in. Otherwise use GitHub. (Email sign-up is off on purpose.)
- [ ] **Continue with GitHub** works.
- [ ] Account settings: change name, sign out, sign in again.
- [ ] **Delete account** with a throwaway account: back on the sign-in screen; that account can't sign
      in any more. (Needs `delete_my_account`: accounts-setup.md 1.)

## 4. Connect the iPhone

- [ ] Plug in, unlock, tap **Trust**. The app lists the phone as Trusted.
- [ ] Developer Mode: the app/doctor explains how to turn it on; after the restart the phone shows up.
- [ ] `swiftwin doctor` with the phone: all PASS except optional AirPlay (note any FAIL text).
- [ ] iOS 17.0–17.3 only: the tunnel needs `remote tunneld` as Administrator; is the message clear?

## 5. Run an app

- [ ] Home → **New project** (SwiftUI template) → **Run**. It builds, signs with the Apple ID, installs
      and opens on the phone. Note the total time.
- [ ] The app's icon on the home screen is the generated one (not the blue "T").
- [ ] First run on this phone: iOS asks to trust the developer (Settings → General → VPN & Device
      Management). The message in SwiftWin says so.
- [ ] Change the text in `ContentView.swift`, Run again: the phone shows the change.
- [ ] Logs panel shows the app's own log lines; a crash shows in the crash view.

## 6. Mirror and control

- [ ] The live mirror runs (note the fps shown, and whether it says USB video or screenshots).
- [ ] Click = tap, drag/scroll = swipe, typing reaches a text field, Home works.
- [ ] Unplug and replug the cable: the app reconnects by itself or says what to do.

## 7. Claude

- [ ] `claude mcp add swiftwin -- swiftwin mcp`, then in Claude Code: "take a screenshot of my phone",
      "tap the button", "show me the app's logs". Each works.
- [ ] **AI mode** (`Ctrl+Shift+A`): ask Claude to change the app's color and run it. You see its taps
      on the phone, and it builds and installs.
- [ ] **The phone in the Claude desktop app:** install the plugin (first-run, "Working in the Claude Code
      app"), ask Claude to use the phone: it opens beside the chat by itself; your clicks there tap the
      phone. Close SwiftWin's window: the phone stays (SwiftWin in the tray).

## 8. Inspo

- [ ] **Inspo** → pick an app on the phone (not a private one) → capture. Screenshots and a style guide
      appear in the project's `.inspo` folder. Note apps where it gets stuck (tab bars, slow apps).

## 9. Later checks

- [ ] **7 days later** (free Apple ID): open the test app on the phone. It won't open. Run it from
      SwiftWin: the message should be "This app's 7-day signing expired: run it again…", and Run fixes
      it. Same for the mirror/taps (WebDriverAgent): "WebDriverAgent's 7-day signing expired". Copy the
      exact text either way: it confirms the patterns in `swiftwin-device/src/lib.rs::signing_expired`.
- [ ] **Updates** (once a release feed exists, see [releasing.md](releasing.md)): an older install shows
      "SwiftWin x.y is available", the click installs and restarts it.
- [ ] **Uninstall** from Windows Settings → Apps: SwiftWin is removed; `%LOCALAPPDATA%\SwiftWin\data`
      stays (by design) and can be deleted by hand.

## Report

Send the owner: the setup from step 0, the list of ❌ with error text, timings, and doctor output.
Confirmed rows can then be removed from [untested.md](untested.md).
