# First run: connecting your iPhone to SwiftWin

Everything in SwiftWin was built and tested **without** an iPhone, a Mac, or your Apple ID (see
[untested.md](untested.md)). This guide takes you from a fresh Windows PC to "Claude Code taps around
in my app on my phone", in order. Each part ends with a check, so you know exactly where something
broke. Plan on about an hour, mostly downloads.

**You need:** Windows 10/11 x64 · an iPhone with iOS 17 or newer and a USB **data** cable · a **paid**
Apple Developer Program membership (the free Apple ID can't use the App Store Connect API that
SwiftWin signs with) · about 30 GB of free disk space while the iOS SDK is downloaded and imported.

---

## Part A: Install SwiftWin (no phone yet)

### A1. Get the installer
Download `SwiftWin_<version>_x64-setup.exe` from the project's Releases page, or build it yourself
(README, "Build from source"): it is `app\src-tauri\target\release\bundle\nsis\SwiftWin_<version>_x64-setup.exe`.
It contains the desktop app and the `swiftwin` command-line tool.

### A2. Install the desktop app
Run `SwiftWin_<version>_x64-setup.exe`. The installer isn't code-signed, so Windows SmartScreen will warn you.
Click **More info › Run anyway**. Run it by double-clicking or from a normal PowerShell, **not** from a
terminal inside the Claude desktop app: that app sandboxes new AppData folders, and an install made from
it is invisible to the rest of Windows (`swiftwin doctor` warns if that happened). SwiftWin installs to `%LOCALAPPDATA%\SwiftWin`. The `swiftwin` command is in
`%LOCALAPPDATA%\SwiftWin\bin`, and your data (SDK, tools, settings) goes to `%LOCALAPPDATA%\SwiftWin\data`.

### A3. Put `swiftwin` on your PATH
Open **PowerShell** and run:

```powershell
[Environment]::SetEnvironmentVariable("Path", [Environment]::GetEnvironmentVariable("Path","User") + ";$env:LOCALAPPDATA\SwiftWin\bin", "User")
```

Close PowerShell, open a new one, and check:

```powershell
swiftwin --version
```

### A4. Set up everything (one button)
Open **SwiftWin**. When something is missing, the **Set up this PC** dialog opens by itself (later:
avatar menu › **Set up this PC**, or the Doctor dialog). Press **Set up everything**. Each item shows
DONE, TO DO, FAIL (with the fix) or NEXT (your turn):

| Item | How SwiftWin gets it (official source only) |
|---|---|
| Apple's USB service | `winget install Apple.iTunes` (Apple's own installer from apple.com). Windows asks for permission; **restart the PC** when it says so |
| Python 3.11+ | `winget install Python.Python.3.12` |
| Swift 6.2.4 | `winget install Swift.Toolchain --version 6.2.4` (matches Xcode 26.3) |
| LLVM 22.1.8 | `winget install LLVM.LLVM --version 22.1.8` (the `ld64.lld` linker for iOS) |
| Helper tools | pymobiledevice3 in SwiftWin's own Python environment, plumesign and rcodesign (pinned SHA-256) |
| iOS SDK | your Apple sign-in, see B4 |

Already installed items are skipped, so running it again is safe. The same from a terminal:
`swiftwin setup` (`--check` only lists). No winget (Windows Package Manager)? Install **App Installer**
from the Microsoft Store, or install each item by hand from the link in its FAIL message.

**Use the web iTunes, not the Store apps.** Free Apple ID signing (plumesign) doesn't work with the
Microsoft Store's Apple Devices or iTunes. If one is installed, uninstall it before **Set up everything**.

### A5. (Optional) the same by hand
If you'd rather not use winget: iTunes from https://www.apple.com/itunes/, Python from python.org, Swift
6.2.4 from https://www.swift.org/install/windows/, LLVM from https://github.com/llvm/llvm-project/releases,
then `swiftwin doctor --fix` for the helper tools. Many doctor checks still FAIL at this point (no phone
yet). That's expected.

### A6. Smoke test without the phone

```powershell
swiftwin --mock demo
```

Then open **SwiftWin** from the Start menu and click **Try with a simulated iPhone**. You should see a
fake phone you can click, type into and scroll. Close the app when you're done.

---

## Part B: Apple account setup (still no phone)

### B1. Find your Team ID
Go to https://developer.apple.com/account, then **Membership details**, and copy the **Team ID**
(10 characters, like `ABCDE12345`).

### B2. Create an App Store Connect API key
1. Go to https://appstoreconnect.apple.com, then **Users and Access › Integrations › App Store Connect
   API › Team Keys**, and click **+**.
2. Name: `SwiftWin`. Access: **Admin** (needed to create certificates and profiles).
3. Click **Download API Key**. You get `AuthKey_XXXXXXXXXX.p8`, and **Apple lets you download it only
   once.**
4. Copy the **Key ID** (shown next to the key) and the **Issuer ID** (shown above the list).

### B3. Give the key to SwiftWin

**In the desktop app:** after you connect your iPhone the first time, SwiftWin asks how to sign
(also: avatar menu › **Signing**). Pick **Paid Apple Developer account**, choose the `.p8`, fill in the
Key ID, Issuer ID and Team ID, press **Check key** (PASS/FAIL from App Store Connect), then **Save and use**.
The **Free Apple ID** tab signs in with your Apple ID instead (Part B-free). The menu shows the current mode.

**Or in a terminal:**

```powershell
swiftwin signing setup --key-id XXXXXXXXXX --issuer-id 00000000-0000-0000-0000-000000000000 --p8 "$env:USERPROFILE\Downloads\AuthKey_XXXXXXXXXX.p8" --team-id ABCDE12345
swiftwin signing status
```

The `.p8` is stored in the **Windows Credential Manager**. It is never saved to a file or to the repo.
After `signing status` shows `ASC .p8 stored: yes`, move the downloaded `.p8` somewhere safe, like a
password manager.

### B4. Get the iOS SDK
**In the app (Set up this PC):** after the other items, SwiftWin opens Apple's own sign-in page in a
separate window. Sign in with your Apple ID (a free one works); SwiftWin never sees the password. Apple
then starts the **Xcode 26.3** download (`Xcode_26.3_Apple_silicon.xip`, about 3 GB). SwiftWin takes it
over in the background: it checks for about 30 GB of free disk space, shows progress, resumes a broken
download (press the sign-in button again), extracts **only** the iOS SDK and XCTest frameworks (10–20
minutes) and deletes the `.xip`. Xcode 26.3 bundles Swift 6.2.4, the same as the Windows toolchain;
26.4 or newer ship Swift 6.3, whose SDK interfaces a 6.2 compiler can't read.

**By hand:** sign in at https://developer.apple.com/download/all/, download Xcode 26.3, then:

```powershell
swiftwin sdk import "$env:USERPROFILE\Downloads\Xcode_26.3_Apple_silicon.xip"
swiftwin sdk list
```

Once `sdk list` shows `iPhoneOS26.x`, you can delete the `.xip`.

**Check:** `swiftwin doctor` should now PASS *Signing*, *Swift toolchain*, *iOS SDK* and
*Code signing tool*.

---

## Part B-free: no paid membership (free Apple ID)

Without a paid Apple Developer Program membership, skip B1–B3. SwiftWin signs and installs with your free
Apple ID through **plumesign**, the command-line half of the open-source Impactor (MIT). Apple's limits
for free accounts: apps stop launching after **7 days** (just `swiftwin run` again), at most **3**
sideloaded apps per phone, at most **10** new App IDs per week. SwiftWin reuses its bundle ids, so
re-installing doesn't spend new ones.

Nothing else to install: SwiftWin downloads plumesign 2.7.2 (checked against a pinned SHA-256) and
rcodesign the first time you sign in.

1. **In the desktop app:** connect the iPhone; SwiftWin asks how to sign (also: avatar menu › **Signing**).
   On **Free Apple ID**, enter your Apple ID and password and press **Sign in with your Apple ID**, then
   enter the 6-digit code Apple sends to your devices. The password goes only to plumesign's input, which
   sends it to Apple; SwiftWin doesn't store it. plumesign keeps the session in `%APPDATA%\PlumeImpactor`.
   Start SwiftWin from the Start menu, not from a Claude session, so that folder is created outside
   Claude's sandbox.
2. **Or in a terminal:** `swiftwin signing apple-id --email you@example.com --login` (plumesign asks for the
   password and code there, and shows the password as you type it).

   Either way you get a random bundle-id prefix such as `dev.swiftwin.u1a2b3c4d`; new projects use it,
   because `com.example.*` is usually already taken at Apple.
3. Right after signing in, the app's **Phone control** step installs WebDriverAgent (see C4).
4. Import the SDK as in B4 (a free Apple ID can download Xcode). Then `swiftwin run`, `swiftwin wda prepare`,
   AI mode and the desktop app's ▶ Run work as with a paid account.

Good to know:
- plumesign signs the WebDriverAgent runner but leaves its test bundle unsigned, so `swiftwin wda prepare`
  signs the whole runner again with plumesign's certificate (via rcodesign) and reinstalls it.
- When your account reaches Apple's certificate limit, plumesign revokes an existing development
  certificate and makes a new one; apps signed with the old one (e.g. from AltStore) stop launching.
- plumesign gets Apple's device-provisioning ("anisette") data from `mac-ani.jkcoxson.com` unless a
  local library is present; your password goes only to Apple.
- No automation? `swiftwin build --ipa` and `swiftwin wda ipa` write unsigned `.ipa` files for any sideloader
  (Impactor's window, or Sideloadly, which also needs the web iCloud); then `swiftwin launch <bundle id>`.

---

## Part C: Plug in the iPhone

### C1. Connect and trust
1. Plug the iPhone in with the USB cable and **unlock it**.
2. When it asks **"Trust This Computer?"**, tap **Trust** and enter your passcode.
3. Check:

   ```powershell
   swiftwin devices
   ```

   It should print your UDID and `trusted`. If it says `NOT trusted` and there was no prompt, run
   `swiftwin pair` and watch the phone.

### C2. Turn on Developer Mode
1. On the iPhone, open **Settings › Privacy & Security › Developer Mode** and switch it **On**.
   - If **Developer Mode** isn't in the list, run `swiftwin doctor --fix`. It asks the phone to show the
     option.
2. The phone restarts. After it boots, unlock it, tap **Turn On**, and enter your passcode.

### C3. Run the doctor

```powershell
swiftwin doctor
```

Fix the **first** FAIL, then run it again. Every step prints the exact error and a fix. Here's what to
expect the first time:

| Step | First-time notes |
|---|---|
| Apple USB driver | FAIL → reinstall Apple Devices, restart, and check that *Apple Mobile Device Service* is Running in `services.msc`. |
| iPhone visible | FAIL → use a different cable or port (charge-only cables don't work) and unlock the phone. |
| Trusted | FAIL → tap Trust, or run `swiftwin pair`. |
| Developer Mode | FAIL → see C2. |
| Developer tunnel (iOS 17+) | FAIL → see the fix text. Keep the phone unlocked; on iOS 17.4+ the tunnel needs no admin rights. Only on iOS 17.0–17.3: run `pymobiledevice3 remote tunneld` in an **Administrator** PowerShell (the executable is in `%LOCALAPPDATA%\SwiftWin\data\venv\Scripts`), leave it running, and retry. |
| Developer disk image | The first mount downloads a personalized image from Apple, so the PC needs internet access and the phone must be unlocked. |
| Screenshot | Should PASS once the two steps above do. |
| WebDriverAgent | FAIL is expected the first time. See C4. |

### C4. Install WebDriverAgent (tap/swipe/type)

WebDriverAgent is a small helper app on the iPhone that lets SwiftWin tap, swipe, type and read the
screen. **In the desktop app** the **Phone control** step does all of this: it opens after you sign in
with Apple, when you connect a phone that doesn't have it, and when a tap fails because it is missing or
its 7-day signing expired. Press **Install WebDriverAgent**, trust the developer on the phone
(Settings › General › VPN & Device Management › your Apple ID › Trust), then **Check it starts**. With a
free Apple ID it is one of your 3 apps and uses App IDs from your 10 a week.

**Or in a terminal:**

```powershell
swiftwin wda prepare
swiftwin wda start
swiftwin ui-tree
```

`wda prepare` downloads appium's WebDriverAgent, gives it a bundle id for your team, signs it and
installs it. `wda start` launches it on the phone; the screen may briefly show "Automation Running".
`ui-tree` should print what's on screen.

- If the phone says **Untrusted Developer**, go to **Settings › General › VPN & Device Management**,
  tap your developer certificate, then **Trust**, and run `swiftwin wda start` again.

**Check:** `swiftwin doctor` now has no FAIL (the AirPlay WARN is fine).

---

## Part D: Your first app

```powershell
swiftwin new Hello --dir "$env:USERPROFILE\SwiftWinProjects"
cd "$env:USERPROFILE\SwiftWinProjects\Hello"
swiftwin run
```

`swiftwin run` builds with Swift, signs (the first time, it creates your development certificate, App ID
and provisioning profile), installs and launches the app. **Hello** should appear on the phone with a
counter button, a text field and a list. Then try:

```powershell
swiftwin tap --label "Tap me"
swiftwin logs --process Hello --limit 20
swiftwin screenshot -o shot.png
```

### The desktop app
Open **SwiftWin** from the Start menu and click **Use this iPhone**. Open
`%USERPROFILE%\SwiftWinProjects\Hello`, edit `ContentView.swift`, then press **▶ Run** (Ctrl+R).

In the phone window, a click is a tap, a drag is a swipe, the mouse wheel scrolls, and typing goes to
the focused field. The **Doctor** button runs the same checks as `swiftwin doctor`.

### Claude Code

```powershell
claude.cmd mcp add --scope user swiftwin -- "$env:LOCALAPPDATA\SwiftWin\bin\swiftwin.exe" mcp --project "$env:USERPROFILE\SwiftWinProjects\Hello"
```

- `claude.cmd`, not `claude`: when PowerShell runs npm's `claude.ps1` it drops the `--`, so `--project`
  would go to Claude instead of SwiftWin.
- `--scope user` makes it available in every folder (the default scope is per folder).
- The full path to `swiftwin.exe` matters because the Claude desktop app keeps the PATH it started with.

Start a **new** Claude Code session (`claude.cmd mcp list` should show `swiftwin`), then ask, for example: *"Use swiftwin: run the app, take a screenshot, tap 'Tap me'
three times and check the logs."* Claude gets these tools: `build, install_run, screenshot, ui_tree,
tap, swipe, type, press_button, logs, device_info, release`.

### OpenAI Codex

```powershell
codex.cmd mcp add swiftwin -- "$env:LOCALAPPDATA\SwiftWin\bin\swiftwin.exe" mcp --project "$env:USERPROFILE\SwiftWinProjects\Hello"
```

This writes a `[mcp_servers.swiftwin]` entry (`command`, `args`) to `~\.codex\config.toml`, which every
Codex session uses; you can also add that entry by hand. `codex mcp list` should show `swiftwin`. Use
`codex.cmd` for the same reason as `claude.cmd` above if Codex came from npm. Then start a new Codex
session and ask the same way; it gets the same tools.

**Watch your agent work (AI mode):** keep the SwiftWin app open and connected, then click **AI mode** in
the title bar (or press `Ctrl+Shift+A`). The window shows the iPhone next to the two commands above
(short form, each with a copy button). `swiftwin mcp` sends the agent's actions through the open app, so
you see each tap as an orange ripple with a cursor, each swipe as a trail, "AI agent is using this
device" while it works, and a caption with what it just did. Press `Esc` to go back to the editor. If
the app is closed, the agent still works; you just don't see it live.

---

### Working in the Claude Code app with a phone window

If you prefer the Claude Code app or a terminal to SwiftWin Studio, open just the phone: run `swiftwin phone`
(or start `swiftwin-app.exe --phone`), or ask Claude to call `show_phone`. A small window shows the iPhone
live, connects by itself, and shows every tap and swipe Claude makes. Click to tap, drag to swipe, type to
type, as in the Studio.

Under the phone a small toolbar has **Home**, **Run** (build, install and launch the project open in the
Studio), **Screenshot**, **Inspo**, **Signing** (a yellow dot when signing or phone control still needs
setting up), **SwiftWin Studio** (brings the full window forward) and your **Account**. In the Claude
app's Browser pane, Account and Signing open in the SwiftWin Studio window: the phone link never gets
account access.

**Inside the Claude desktop app (Code tab):** the phone appears by itself. The first time Claude uses a
swiftwin tool in a session, the live iPhone opens in the app's built-in browser, the **Browser pane** on the
right beside the chat: live mirror, click to tap, drag to swipe, Home, the mirror menu, Claude's taps. No
SwiftWin window opens: if SwiftWin isn't running it starts in the background, as a tray icon (right-click it
for **Open SwiftWin Studio** or **Quit SwiftWin**). Closing the Studio window keeps that background bridge
running while Claude uses the phone; **Quit SwiftWin** in the tray ends it, and the pane says so.

Set it up once, from the Claude desktop app's Terminal panel (it installs SwiftWin's small Claude plugin,
`claude-plugin/swiftwin-phone`, which opens the pane for Claude):

```
claude plugin marketplace add C:\Users\<you>\Desktop\THENDER-WORK\claude-plugin
claude plugin install swiftwin-phone@swiftwin
```

Without the plugin it still works: SwiftWin's MCP server tells Claude to call `show_phone` and open its
one-time link (2 minutes, works once) without being asked. Either way the pane is a browser tab with a
`127.0.0.1` address: the Claude app's side panels for plugins can draw text, buttons and pictures, but not a
live video you can click on, so the Browser pane is the one place in the app that can show the phone.

### Inspo: design inspiration from apps on your phone

Open your project, connect the iPhone, then click **Inspo** in the title bar and pick one of your apps
(for example a music app you like). SwiftWin opens it and looks around by itself: it taps each tab-bar
button and scrolls each tab once or twice, capturing up to 12 screens (it never taps any other button,
so nothing gets bought, sent or deleted). Tick **Deep capture** to also open every button and row on each tab and come back (up to 40 screens, several minutes); it still skips anything that buys, sends, posts, deletes, signs out, likes or adds. Keep the phone unlocked and don't touch it until it finishes;
**Stop** ends it early. Afterwards the phone goes back to your app (or Home).

The dialog then shows the screens, the colour palette (background, surface, text, accent), the type
scale, spacing and the patterns it saw. **Add theme to my project** writes `Sources/InspoTheme.swift`
(`InspoTheme.background`, `.accent`, `.title`, `.padding`, `.cornerRadius`…); **Copy prompt for your AI agent** opens
AI mode and copies a message asking your agent to restyle your app with it; paste it into Claude Code or
Codex. The agent can also do it by itself with the `inspo_capture` tool.

Everything is saved in your project's `.inspo/<app>/` folder (screenshots, UI trees, `INSPO.md`,
`inspo.json`). The screenshots may show your personal data (account name, playlists, messages): they stay
on this PC, and the folder is git-ignored so an Upload never includes them. Use it as inspiration only:
don't copy the other app's logo, icons, artwork, name or exact screens.

## Part E (optional)

### Smooth AirPlay mirroring
The phone window shows WebDriverAgent's video stream over USB (about 23 fps, the "Video" pill) once
WebDriverAgent runs, and USB screenshots (2–3 fps, "Screenshots") until then; the menu next to the pill
can force either. For 60 fps over Wi-Fi, install UxPlay with its GStreamer and Bonjour requirements (see
[dependencies.md](dependencies.md)), put `uxplay.exe` in `%LOCALAPPDATA%\SwiftWin\data\tools\uxplay\`,
and run:

```powershell
swiftwin mirror --airplay --seconds 30
```

On the iPhone, open **Control Center › Screen Mirroring › SwiftWin**. The phone and PC must be on the same
Wi-Fi network. Allow UxPlay through the Windows firewall when Windows asks.

### TestFlight via a cloud Mac
1. Create the app once in App Store Connect: **My Apps › + › New App**, with the same bundle id as
   `swiftwin.toml`.
2. Create a GitHub repository with a README, then create a token that can access it. Use either a
   classic token with `repo` + `workflow` scopes, or a fine-grained token with Contents, Actions,
   Secrets and Workflows set to read/write.
3. Run:

   ```powershell
   swiftwin release setup --repo you/hello-ios --token ghp_...
   swiftwin release
   ```

   This uploads the project to the `swiftwin/release` branch, and a `macos-26` runner builds it with
   Xcode 26 and uploads it to TestFlight. To see the generated files first, use `swiftwin release --dry-run`.

---

## Part F: Accounts and uploading to GitHub

### For everyone: signing in
When this build of SwiftWin has accounts set up, the app opens on a **Sign in** screen before
"Connect your iPhone". Sign in with your email and password, or click **Continue with Google** /
**Continue with GitHub**: your browser opens, you approve, and the app continues by itself.

- **New account:** *Create an account*, enter your email and a password (8+ characters), then open the
  link in the email **on this PC**: the app signs you in by itself. (Opened it on your phone? Your email
  is confirmed anyway; sign in with your password.)
- **Forgot password:** *Forgot password?*, open the link in the email on this PC, then choose a new
  password in the app.
- If the email has a 6-digit code (only when the project's templates show one), *Got a 6-digit code
  instead?* takes it.
- **Who am I / sign out:** the person button in the title bar. Signing out also disconnects GitHub.
- **Delete your account:** Account settings → Delete account, then type your email. It is deleted at the
  sign-in service right away, and this PC is signed out. Your projects stay.
- From a terminal: `swiftwin login` (`--google`, `--github`), `swiftwin whoami`, `swiftwin logout`.

SwiftWin keeps your session in the Windows Credential Manager (`swiftwin:account-refresh-token`), never in
a file. `%LOCALAPPDATA%\SwiftWin\data\account.json` only holds your email, name and plan.

### Uploading a project
Click **Upload** in the title bar (or run `swiftwin upload`).

1. The first time, click **Connect GitHub**. SwiftWin shows a code and opens github.com/login/device:
   enter the code and approve. SwiftWin asks for the `repo` permission, which lets it create repositories
   and push to them. The token stays in the Credential Manager (`swiftwin:github-upload-token`).
2. The first upload creates a **private** repository named after the project (you can rename it, or
   untick *Private*). Later uploads update the same repository with one new commit each time; GitHub
   keeps every earlier version in the history. If nothing changed, nothing is committed.
3. Uploads only happen when you click **Upload**.

Never uploaded: anything your `.gitignore` (or a `.swiftwinignore`) excludes, `build/` and other build
output, `.app`/`.ipa`/`.xcarchive` bundles, SDKs, signing files and secrets (`.p8`, `.p12`, `.pem`,
`.key`, `.mobileprovision`, `.env`…) and files over 50 MB. The dialog lists what it skipped. If an
ordinary file contains a private key, the upload stops and says which file.

To take SwiftWin's access away completely: GitHub › Settings › Applications › Authorized OAuth Apps.

### For the owner: setting up accounts (once)
SwiftWin uses **Supabase Auth** for accounts, so there is no SwiftWin server to run. Only you can create
these accounts; never paste a password or a *secret* anywhere but the service's own website.

**1. Supabase project**
1. Sign up at [supabase.com](https://supabase.com) and create a project (the free plan is enough). Pick a
   strong database password and keep it in your password manager; SwiftWin never needs it.
2. **Project Settings › API Keys**: copy the **Project URL** (`https://<ref>.supabase.co`) and the
   **publishable** key (`sb_publishable_…`; older projects call it the `anon` `public` key). These two are
   public: they ship inside the app. **Never** copy the `secret` / `service_role` key anywhere (SwiftWin
   refuses it).
3. **Authentication › URL Configuration**: set **Site URL** to `http://127.0.0.1` and add the redirect
   URL `http://127.0.0.1:*/auth/callback` (the app listens on a random local port during sign-in).
4. **Authentication › Sign In / Providers › Email**: keep *Confirm email* on, set *Minimum password length*
   to 8 or more.
5. **Email sending:** Supabase's built-in email only reaches members of your Supabase team, about 2 emails
   an hour, and its templates can't be edited. That's fine to test with your own address: the default
   emails have a link, which comes back to the app. Before anyone else signs up, set up custom SMTP
   (**Authentication › Emails › SMTP Settings**, e.g. Resend or Postmark, which needs a domain of your own).
   Then you may also add the code to the **Confirm signup** and **Reset Password** templates:
   `<p>Your SwiftWin code: <b>{{ .Token }}</b></p>`.

**2. GitHub OAuth app** (used for "Continue with GitHub" *and* for uploads)
1. GitHub › Settings › Developer settings › **OAuth Apps › New OAuth App**. Name: SwiftWin. Homepage URL:
   any page of yours. **Authorization callback URL:** `https://<ref>.supabase.co/auth/v1/callback`.
2. On the app's page, tick **Enable Device Flow** and save.
3. **Generate a new client secret** and paste it, with the **Client ID**, into Supabase: **Authentication ›
   Sign In / Providers › GitHub**, then enable it.

**3. Google OAuth client**
1. [Google Cloud Console](https://console.cloud.google.com): create a project, then **Google Auth Platform**:
   fill in *Branding* (app name SwiftWin, your support email) and *Audience* (External).
2. **Clients › Create client › Web application.** Authorized redirect URI:
   `https://<ref>.supabase.co/auth/v1/callback`.
3. Paste the client ID and client secret into Supabase: **Authentication › Sign In / Providers › Google**,
   then enable it. While the Google app is in *Testing*, only the test users you list can sign in; publish
   it before selling.

**4. Tell SwiftWin**
Give Claude the **Project URL**, the **publishable key** and the **GitHub Client ID** (all three are public).
They go into `swiftwin-build.env` at the repository root (git-ignored, so they stay out of the public
source; `crates/swiftwin-account/build.rs` bakes them into every build made on that PC):

```
SWIFTWIN_BUILD_SUPABASE_URL=https://<ref>.supabase.co
SWIFTWIN_BUILD_SUPABASE_KEY=sb_publishable_…
SWIFTWIN_BUILD_GITHUB_CLIENT_ID=Ov23…
```

The same names as environment variables work too (e.g. as secrets in a release workflow). To try values
without rebuilding, put them in `%LOCALAPPDATA%\SwiftWin\data\auth.json`:

```json
{ "supabase_url": "https://<ref>.supabase.co", "supabase_key": "sb_publishable_…", "github_client_id": "Ov23…" }
```

(or set `SWIFTWIN_SUPABASE_URL`, `SWIFTWIN_SUPABASE_KEY`, `SWIFTWIN_GITHUB_CLIENT_ID`). Then restart the app:
it should open on the Sign in screen. *Check:* sign up with email, sign out, sign in with Google, sign out,
sign in with GitHub, then **Upload** a project and open the new private repository on GitHub.

### Selling SwiftWin later
Each account has a **plan**, read from the user's `app_metadata.plan` in Supabase (`free` when missing).
Only the server can set `app_metadata`, so a paid plan will be granted by a payment webhook (for example a
Supabase Edge Function called by Stripe) that sets `plan` to `pro`. The app already shows the plan in the
Account dialog; gating a feature is a check of `Accounts::plan()`. A check inside the desktop app can always
be patched out, so anything really worth paying for should also be enforced on a server.

---

## If something fails: what to send back to Claude

Run these and attach the output:

```powershell
swiftwin --version
swiftwin doctor --json > doctor.json
swiftwin info --json > info.json
swiftwin logs --limit 300 > logs.txt
```

Also include the exact command you ran and its full output. `%LOCALAPPDATA%\SwiftWin\data\config.json`
is safe to share; it holds no secrets. **Never** share the `.p8` file, GitHub tokens, or a Supabase
*secret*/`service_role` key or OAuth client secret.

[untested.md](untested.md) lists every code path that hasn't touched a real phone yet, and
[open-questions.md](open-questions.md) lists the assumptions to verify. Each numbered question there
says which step will confirm it.
