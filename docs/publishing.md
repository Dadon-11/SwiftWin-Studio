# Publishing SwiftWin without leaking anything about you

The repo is local only. When you're ready to open-source it, **publish a fresh repository made from a
cleaned snapshot**, not this repository: old commits still contain your name and email (as author) and
earlier versions of files. This repository and its history stay on your PC, unchanged.

## What the privacy sweep found (2026-11-10)

Searched all tracked files and the full git history (every branch, every commit, commit messages
included) for personal data and keys. Images were looked at by eye.

| What | Where it was | What was done | Your decision |
|---|---|---|---|
| Your name | LICENSE, README, PRIVACY.md, TERMS.md, git history | **Kept on purpose** (2026-11-11): you want credit as the author | Use your name as the snapshot author; keep your personal Gmail private with GitHub's `noreply` address |
| Your first name | test fixtures ("…'s iPhone", `com.<name>.wda`), in old commits | Files changed to "Ada's iPhone", `com.example.wda`; old commits stay local | — |
| Your GitHub username and the old repo name | `Cargo.toml` `repository`, `docs/history/pull-requests.md` (links) | `repository` removed; `docs/history/` is left out of the snapshot by the script | Your username is fine to show; add the new public repo URL to `Cargo.toml` after publishing, if you like |
| Windows user folder (`C:\Users\<name>`) | `docs/releasing.md` in one earlier commit | Replaced with `%USERPROFILE%` | — |
| Supabase project URL, publishable key, GitHub OAuth client id | `crates/swiftwin-account/src/settings.rs`, `docs/progress.md` | **Moved out of the source** into `swiftwin-build.env` (git-ignored), baked in at build time by `crates/swiftwin-account/build.rs`. Your builds keep working; a public build without the file works without accounts (only cloud release and upload are off) | Keep the file safe; for a CI release, set the same names as secrets/env vars |
| Updater keys | private key `%USERPROFILE%\.tauri\swiftwin-updater.key` (outside the repo); public key in `tauri.conf.json` | Public key stays (it must, to check updates) | Back up the private key offline |
| Private keys, tokens, `sb_secret`/`service_role`, `.p8`/`.p12`/`.pem`, `.env`, plumesign `accounts.json` | — | **None found**, in files or history (only dummy test values) | — |
| iPhone UDID, phone name, Apple team id, team-derived bundle ids, location (city), Wi-Fi name, the phone's app list | — | **None found** in files or history (UDIDs in tests are fakes like `00008110-000A1B2C3D4E5F60`) | — |
| Phone screenshots | `docs/responsive/`, `docs/screenshots/` | All show the **simulated** iPhone and the fake "Ada" account | — |
| Inspo captures | `<project>/.inspo/` | Never committed; `.inspo/` is now git-ignored in this repo too | Delete old captures in your projects when done |
| `Hello/`, `MyApp/` | repo root, **untracked** (never committed) | **Deleted** (owner's decision, 2026-11-10) | — |
| `CLAUDE.md` | repo root | No personal data; it linked the design doc as a private claude.ai artifact | **Kept**, as many open-source projects now keep an AI-assistant file; the script drops every `claude.ai/artifact` link from the public copy. After publishing, rewrite its "Development workflow" section (it still says "local only, no GitHub") |
| `docs/progress.md`, `docs/open-questions.md` | docs | Work log; mentions "the owner's iPhone" (model iPhone18,3, iOS 27) but nothing identifying | **Kept**: design notes and "what was tried" logs are common in open-source docs, and other docs link to them |

## Step by step

1. **Your private scan list.** `publish-patterns.txt` in the repo root (git-ignored, already created
   with your personal Gmail, PC user name and the old private repo name) lists words that must never be published,
   one regular expression per line. Add anything else: your phone's name, your city, other usernames.
2. **Commit** your latest work in this repo (the snapshot uses `HEAD`).
3. **Make the snapshot** from PowerShell in the repo folder:

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts\publish-snapshot.ps1 -Dest ..\swiftwin-public -AuthorName "<public name or handle>" -AuthorEmail "<id>+<user>@users.noreply.github.com"
   ```

   It copies only tracked files at `HEAD`, removes `docs/history/`, scans every text file for keys and
   for your patterns (plus the IDs in `swiftwin-build.env`), and **stops** if anything matches. Then it
   creates a new git repository in `..\swiftwin-public` with one commit by the author you gave. Nothing
   is pushed. Your GitHub noreply address is under GitHub → Settings → Emails.
4. **Look at it yourself:** open the images in `..\swiftwin-public\docs`, skim `README.md`, and
   `git -C ..\swiftwin-public log --format="%an <%ae>"`.
5. **Build it once from the snapshot** without `swiftwin-build.env` to see a contributor's build works
   (`cargo test --workspace`). Such a build has no account service and works fully
   without one (building, running on the iPhone, the phone in Claude, Inspo, AI mode); only the cloud
   features (TestFlight release, Upload to GitHub) need an account, which a contributor can get by pointing
   the build at their own Supabase project (`SWIFTWIN_BUILD_*`, see accounts-setup.md).
6. **Create the GitHub repository** (empty, no README) and push **from the snapshot folder** only:

   ```powershell
   cd ..\swiftwin-public
   git remote add origin https://github.com/<you>/swiftwin.git
   git push -u origin main
   ```

   Never add a remote to this original repository.
7. **On GitHub:** Settings → Emails → "Keep my email addresses private" and "Block command line pushes
   that expose my email"; Security → turn on private vulnerability reporting, Dependabot alerts, secret
   scanning and push protection.
8. **Later changes:** keep working here, then either make a new snapshot and copy it over the public
   folder (`git -C ..\swiftwin-public add -A; git commit`), or work directly in the public repo from
   then on with `git config user.email` set to the noreply address.

## Re-checking by hand

```bash
# keys and tokens anywhere in this repo's history
git log -p --all | grep -n -E "sb_secret_|service_role|BEGIN [A-Z ]*PRIVATE KEY|gh[pousr]_[A-Za-z0-9]{20}|github_pat_|AKIA[0-9A-Z]{16}|sk-ant-|minisign secret"
# your own words: run the snapshot script, it scans for publish-patterns.txt
```
