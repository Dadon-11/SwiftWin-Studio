# Releasing SwiftWin

How to build a release, ship it as an automatic update, and get rid of the "Windows protected your PC"
warning. Checked against the Tauri 2 and Microsoft docs on 2026-11-10.

## The update signing key

SwiftWin Studio updates itself with Tauri's updater plugin. Every update is signed; the installed app
only accepts updates signed with **this project's key**.

| File | Where | In the repo? |
|---|---|---|
| Private key | `%USERPROFILE%\.tauri\swiftwin-updater.key` (on the owner's PC) | **Never** |
| Public key | `%USERPROFILE%\.tauri\swiftwin-updater.key.pub`, and in `app/src-tauri/tauri.conf.json` → `plugins.updater.pubkey` | yes (it is public) |

The private key was made with `npx tauri signer generate --ci -w %USERPROFILE%\.tauri\swiftwin-updater.key`
(no password). **Back it up** somewhere safe and offline (a password manager's file attachment, an
encrypted USB stick). If it is lost, installed copies can no longer be updated: users would have to
download a new installer by hand. If it leaks, anyone could sign an "update": generate a new pair,
put the new public key in `tauri.conf.json`, and release by hand once.

To protect it with a password, generate a new pair with `-p` (or interactively), update the public key
in the config, and set `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` when building.

## Where the app looks for updates

There is no public feed yet, so by default the app **doesn't check**. It checks once, 5 seconds after
start, when an update URL is set, first found of:

- `SWIFTWIN_UPDATE_URL` (environment variable),
- `"update_url"` in `%LOCALAPPDATA%\SwiftWin\data\config.json`,
- `plugins.updater.endpoints` in `app/src-tauri/tauri.conf.json` (empty today).

The URL must be `https://` and return a `latest.json` (below). If a newer version is listed, the app shows
"SwiftWin x.y.z is available. Click to install and restart"; a click downloads it, checks the signature,
and runs the installer in **passive** mode (a small progress window). The app closes during the install.

Once the repo is public, put the GitHub Releases URL into `plugins.updater.endpoints` in
`tauri.conf.json` so every install checks it by default:

```json
"endpoints": ["https://github.com/<owner>/<repo>/releases/latest/download/latest.json"]
```

## Build a release

1. Raise the version in **all** of: `app/src-tauri/tauri.conf.json` (`version`),
   `app/src-tauri/Cargo.toml`, `app/package.json`, and the workspace crates if you like. The updater
   compares `tauri.conf.json`'s version.
2. Run the tests: `cargo test --workspace` and the UI test (README).
3. Build, from the Claude desktop **Terminal panel** or a normal PowerShell:

```powershell
cargo build --release -p swiftwin-cli
cd app
$env:TAURI_SIGNING_PRIVATE_KEY = "$env:USERPROFILE\.tauri\swiftwin-updater.key"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
npm run tauri build -- --config src-tauri/tauri.release.conf.json
```

`tauri.release.conf.json` only turns on `createUpdaterArtifacts`. Without it (the everyday
`npm run tauri build`) no key is needed and no `.sig` is made.

4. In `app\src-tauri\target\release\bundle\nsis\` you get:
   - `SwiftWin_<version>_x64-setup.exe`: the installer (also what the updater downloads)
   - `SwiftWin_<version>_x64-setup.exe.sig`: its signature
5. Write `latest.json`:

```json
{
  "version": "0.2.0",
  "notes": "What changed, in a sentence or two.",
  "pub_date": "2026-12-01T12:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<the whole text of SwiftWin_0.2.0_x64-setup.exe.sig>",
      "url": "https://github.com/<owner>/<repo>/releases/download/v0.2.0/SwiftWin_0.2.0_x64-setup.exe"
    }
  }
}
```

   `signature` is the **content** of the `.sig` file, not a path. Every platform listed must be complete.
6. Publish SHA-256 sums for people who download by hand: `Get-FileHash .\SwiftWin_*_x64-setup.exe`.

## Where to host it

- **Now (local only):** nothing to host. To try the updater, serve the folder over https (for example a
  private GitHub Gist is not enough: the installer must be downloadable too), or test on another PC.
- **Once public:** a GitHub Release per version, with `SwiftWin_<v>_x64-setup.exe`, its `.sig`, and
  `latest.json` as assets. `releases/latest/download/latest.json` always points at the newest one.
  Tauri's GitHub Action (`tauri-apps/tauri-action`) can do steps 3–5 for you, with the private key stored
  as an encrypted repository secret (`TAURI_SIGNING_PRIVATE_KEY`).

## "Windows protected your PC" (SmartScreen)

The installer and `swiftwin.exe` are **not Authenticode-signed**. When a downloaded unsigned installer
is opened, Windows SmartScreen shows "Windows protected your PC / Microsoft Defender SmartScreen
prevented an unrecognized app from starting". Users must click **More info → Run anyway**. Some will
stop there; some companies block it entirely. (Installing from a file you built yourself doesn't show
it: there is no "downloaded from the internet" mark.)

Tell users in the README until releases are signed. To make it go away, sign the installer and the
binaries inside it:

| Option | Rough cost | Who can get it | Notes |
|---|---|---|---|
| **Azure Artifact Signing** (was "Trusted Signing") | Basic tier about 10 USD/month | Organizations in the US, Canada, EU, UK; **individuals only in the US and Canada** (as of 2026) | Cheapest; Microsoft-managed keys, no hardware token. Needs a paid Azure subscription and identity validation |
| **OV code-signing certificate** (Sectigo, DigiCert, SSL.com, Certum…) | about 150–400 USD/year | anyone, after identity checks (individual or company) | Since June 2023 the key must live on a hardware token or the CA's cloud HSM. Certum's open-source certificate is cheaper for open-source projects |
| **EV certificate** | about 300–600 USD/year | registered companies | No longer gives instant SmartScreen trust (since 2024); not worth the extra for this |

None of them gives instant trust: SmartScreen still warns until a signed file has built up a download
reputation (usually weeks). Signing keeps that reputation across versions, which unsigned builds never
get. **Recommendation:** when the repo goes public, start with SHA-256 sums and the README note; buy
signing (Artifact Signing if eligible, else an OV certificate) once real users download it. Nothing
has been bought.

### Plugging signing into the build

Tauri signs the installer and the app's `.exe` during `npm run tauri build` when `bundle.windows` has
signing settings. Put them in `tauri.release.conf.json` so everyday builds stay unsigned.

**Azure Artifact Signing:**

```powershell
cargo install artifact-signing-cli
$env:AZURE_CLIENT_ID = "..."; $env:AZURE_TENANT_ID = "..."; $env:AZURE_CLIENT_SECRET = "..."   # an app registration with the signer role; never commit these
```

```json
{ "bundle": { "createUpdaterArtifacts": true, "windows": {
  "signCommand": "artifact-signing-cli -e https://<region>.codesigning.azure.net -a <account> -c <certificate profile> -d SwiftWin %1"
} } }
```

**Certificate on a token or in the Windows certificate store:**

```json
{ "bundle": { "createUpdaterArtifacts": true, "windows": {
  "certificateThumbprint": "<thumbprint from certmgr.msc>",
  "digestAlgorithm": "sha256",
  "timestampUrl": "http://timestamp.digicert.com"
} } }
```

(or a `signCommand` that runs your CA's tool, e.g. `signtool sign /fd sha256 /tr <timestamp> /td sha256 /sha1 <thumbprint> %1`).

`bin\swiftwin.exe` is copied into the installer as a resource and is **not** signed by Tauri: sign it
first with the same tool (`signtool sign ... target\release\swiftwin.exe`, or
`artifact-signing-cli ... target\release\swiftwin.exe`) right after `cargo build --release -p swiftwin-cli`.

Check a result with `Get-AuthenticodeSignature .\SwiftWin_<v>_x64-setup.exe` (Status `Valid`).
