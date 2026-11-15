# Legal review brief (for a lawyer or privacy professional)

`PRIVACY.md` and `TERMS.md` (and their website copies in `site/`) were written by the author with an AI
assistant to describe what the code really does. They were published without a lawyer's review (the
owner's decision, 2026-11-15: SwiftWin is free and open source, and the account is optional). If the
project grows (paid plans, a company, many users), have them reviewed: this page is the brief for that.

## The facts

- **What:** SwiftWin, a free, open-source (MIT) Windows app that builds iPhone apps and runs them on the
  user's own iPhone. Made by one person, an individual in Israel, with no company.
- **Account (optional):** sign-in with Google or GitHub only, needed only for the cloud features (TestFlight builds,
  uploading projects to GitHub). Stored at Supabase (free plan, Tokyo region): email, name, profile
  picture link, sign-in dates, plan. No passwords of ours, no emails sent, no analytics or tracking.
- **Everything else stays on the user's PC:** code, phone screenshots, logs, tokens (Windows Credential
  Manager), Apple ID session (third-party open-source tool plumesign, which also contacts a third-party
  "anisette" server, `mac-ani.jkcoxson.com`, as Apple's sign-in requires off Apple hardware).
- **Apple ID password:** typed into SwiftWin, passed in memory to plumesign, never stored or sent to us.
- **Apple's SDK:** each user downloads Xcode from Apple with their own Apple ID; SwiftWin extracts the
  iOS SDK locally and never redistributes it.
- **AI agents:** users connect their own Claude Code or OpenAI Codex; that data goes to Anthropic or
  OpenAI under the user's own account, not through us.
- **Users:** worldwide, expected to include the EU/UK and Israel. Minimum age 16.
- **Money:** free today; paid cloud features might come later.

## Questions for the reviewer

1. **GDPR / UK GDPR:** are the controller identity and contact (a name and a Google Groups email, no
   postal address) enough? Is an EU representative (Art. 27) needed for a free, individual-run project?
2. **Supabase:** is accepting Supabase's Data Processing Addendum enough on the free plan? Is the
   Japan transfer correctly described (EU adequacy decision)?
3. **Israel (Privacy Protection Law, Amendment 13):** anything required for a database this small
   (registration, security regulations, a privacy officer)?
4. **Apple:** risk around downloading Xcode with the user's Apple ID and extracting the SDK on Windows
   (Xcode and Apple SDK license agreement), free-Apple-ID signing through plumesign and the anisette
   server, and the trademark notice in TERMS.
5. **Liability:** are the "as is" and limitation of liability clauses valid for consumers in the EU and
   Israel? The app installs software on users' phones and lets AI tools control them.
6. **Age:** is 16 the right minimum for worldwide users (GDPR Art. 8 varies by country, 13 to 16)?
7. **Open source:** does the MIT license plus these terms cover the distributed installer, and do the
   third-party licenses (`THIRD-PARTY-NOTICES.md`, `docs/dependencies.md`) need anything more?
8. **Paid features later:** what has to change in TERMS (consumer withdrawal rights, refunds, VAT)?

## After the review

Apply the changes to `PRIVACY.md`, `TERMS.md`, `site/privacy.md` and `site/terms.md` (keep them in sync),
update "Last updated", and rebuild the app (it shows
`PRIVACY.md` in the sign-in screen's legal links).
