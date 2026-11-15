---
title: Privacy Policy
permalink: /privacy/
---

# SwiftWin Privacy Policy


Last updated: 15 November 2026

SwiftWin is a Windows app that builds and runs SwiftUI apps on your own iPhone. It is made and run by
Yoav Dadon ("we"). Contact: swiftwin-support@googlegroups.com.

**In short:** an account is optional: you need one only for the cloud features (TestFlight builds and
uploading projects to GitHub). If you make one, we keep your email address, and
your name and profile picture link if you sign in with Google or GitHub. Everything else (your code, your
phone's screen, your Apple ID session, your tokens) stays on your PC. SwiftWin has no analytics,
tracking or ads.

## What we store about you

Your account is kept by our sign-in service, [Supabase](https://supabase.com) (Supabase Inc., region
Tokyo, Japan, a country the EU recognizes as protecting personal data adequately), on our behalf:

| Data | Where it comes from | Why |
|---|---|---|
| Email address | Google or GitHub, when you sign in with them | your account |
| Name you choose in SwiftWin; name and profile-picture link from Google / GitHub | you / Google / GitHub | showing who is signed in |
| Sign-in method, account creation date, last sign-in, plan (`free`) | the sign-in service | running the account |
| Technical logs of sign-in requests (time, IP address, browser/app) | the sign-in service | security and abuse prevention; kept by Supabase for a short time (about a day on its free plan) |

You sign in with Google or GitHub; SwiftWin never sees or stores a password for your account, and the
sign-in service sends you no emails.

We keep your account data for as long as you have an account; when you delete it, it is removed from
the sign-in service right away. Supabase stores it for us as our data processor, under its data
processing terms.

We don't sell or share this data with anyone else, and we don't use it for advertising.

## What stays on your PC

SwiftWin keeps these on your computer only, and never sends them to us:

- **Sign-in tokens:** the sign-in service's refresh token and, if you connect GitHub for uploads, your
  GitHub token, in the **Windows Credential Manager**. Your profile (name, email, picture link) in
  `%LOCALAPPDATA%\SwiftWin\data\account.json`.
- **Your projects and code**, build output, logs and crash reports from your iPhone.
- **Your iPhone's screen:** the live mirror and screenshots are shown in the app and never uploaded by
  SwiftWin.
- **Inspo captures:** screenshots and notes SwiftWin takes of apps on your phone, saved in your project's
  `.inspo` folder. They can show private things that were on screen; delete the folder when done.
- **Apple signing:** if you sign in with a free Apple ID, you type your Apple ID password and 2FA code
  into SwiftWin's own sign-in box. SwiftWin hands them straight to the separate signing tool (plumesign)
  on your PC and never saves or sends them anywhere else. The Apple ID session is stored by that tool in
  its own folder on your PC.

## Services SwiftWin talks to for you

Only when you use the feature, and directly from your PC:

- **Apple** (signing your apps with your Apple ID, downloading the iOS SDK, App Store Connect if you
  use it). The free-Apple-ID signing tool also contacts a third-party "anisette" server
  (`mac-ani.jkcoxson.com`) that Apple's sign-in requires on non-Apple computers.
- **GitHub**, if you connect it: creating repositories and uploading your projects, and the cloud
  TestFlight build. New repositories are private by default.
- **Google and GitHub**, if you sign in with them (their own privacy policies apply).
- **AI agents (Claude Code or Codex)**, in AI mode and through the MCP server: they run as **your own**
  Claude Code or Codex, under your own Anthropic or OpenAI account and terms. What the agent reads (your
  project, screenshots and logs of your phone) goes to Anthropic or OpenAI through it, not through us.
- **Downloads** of the prerequisites and helper tools ("Set up this PC" uses winget for Apple's USB
  service, Python, Swift and LLVM; helper tools come from GitHub and PyPI) and, if configured, of
  SwiftWin updates.

## Your choices and rights

- **See or change** your name: Account settings in the app.
- **Delete your account:** Account settings → Delete account. It deletes your account at the sign-in
  service immediately and signs this PC out. Sign-in logs expire on their own.
- **Remove local data:** sign out (clears the tokens), uninstall SwiftWin, delete
  `%LOCALAPPDATA%\SwiftWin` and your projects' `.inspo` folders.
- Ask us at swiftwin-support@googlegroups.com for a copy of your data, a correction or deletion; we answer within 30 days.
  We use your data to run your account (our agreement with you) and to keep the service secure (our
  legitimate interest). You may also complain to your data protection authority (in Israel, the Privacy
  Protection Authority; in the EU/UK, your country's authority).

## Children

SwiftWin is not meant for children under 16.

## Changes

We'll update this page and its date when what SwiftWin does with data changes.
