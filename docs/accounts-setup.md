# Accounts: getting the Supabase project ready for real users

SwiftWin's accounts run on the owner's Supabase project (free plan). Three things must be done once,
by the owner, in web dashboards. SwiftWin itself needs no change for any of them: the project URL,
publishable key and GitHub client id built in from `swiftwin-build.env` stay the same.

| Step | Why | Time |
|---|---|---|
| 1. Add `delete_my_account` | "Delete account" on the Account page calls it | 2 min |
| 2. Email sign-in off; Google Group as contact | No email sender needed; contact address without showing your Gmail | 5 min |
| 3. Publish the Google sign-in | While "Testing", only listed test users can use Google | 10 min (+ review if you want a logo) |

Checked against the Supabase, Resend and Google docs on 2026-11-10. Dashboards move things around;
the names below are what they were called then.

---

## 1. Let people delete their account

Supabase Auth only deletes a user when asked with the project's **secret** key, and that key must never
be inside an app people download. Instead, a small database function deletes **the caller only**
(`auth.uid()` is the user of the access token that called it). It is the common Supabase pattern for
self-deletion; `auth.admin.deleteUser()` (server side) is what Supabase documents for admins.

1. Open <https://supabase.com/dashboard>, pick the SwiftWin project.
2. Left bar: **SQL Editor** → **New query**.
3. Paste this and click **Run**:

```sql
-- SwiftWin: "Delete account". Deletes only the signed-in caller.
create or replace function public.delete_my_account()
returns void
language plpgsql
security definer          -- runs as the owner (postgres), who may delete from auth.users
set search_path = ''      -- no search-path tricks: every name below is schema-qualified
as $$
begin
  if auth.uid() is null then
    raise exception 'not signed in' using errcode = '28000';
  end if;
  delete from auth.users where id = auth.uid();
end;
$$;

-- Only signed-in users may call it (not anonymous visitors, not PUBLIC).
revoke all on function public.delete_my_account() from public, anon;
grant execute on function public.delete_my_account() to authenticated;
```

4. It should say "Success. No rows returned".

**Why this is safe:** the function takes no arguments, so nobody can name another user; it acts only
on `auth.uid()`, which comes from the signed JWT; `anon` can't call it; `search_path` is empty. Deleting
from `auth.users` also removes the user's sessions, refresh tokens and identities (Supabase's foreign
keys cascade). SwiftWin keeps no other data at Supabase. If you ever add tables that point at
`auth.users`, give them `on delete cascade` or delete those rows in this function first.

**Things to know:**
- Supabase's **Security Advisor** may list it ("security definer function executable by authenticated").
  That is expected here: being callable by signed-in users is the point.
- On projects created after 2026-05-30, new functions are not exposed to the Data API automatically;
  the `grant` above is what exposes this one. The Data API itself must be on (it is by default:
  **Project Settings → Data API**, schema `public` exposed).
- An access token already handed out stays valid until it expires (1 hour by default). SwiftWin forgets
  it right away, so this only matters for a copy someone stole earlier.

**Test it:** in the installed app, create a throwaway account (an email alias of yours), open
**Account settings → Delete account**, type the email, delete. The app returns to the sign-in screen and
the user is gone from **Authentication → Users**. Before step 1 is done, the app says "deleting accounts is
not set up on the sign-in service yet".

---

## 2. Email: off for now (Google and GitHub sign-in only)

**Current choice (2026-11-11):** email + password sign-in is switched **off** in Supabase
(**Authentication → Sign In / Providers → Email → Enable Email provider** off). People sign in only
with Google or GitHub, so Supabase sends no emails and no SMTP is needed. SwiftWin reads Supabase's
public `/auth/v1/settings` at start-up and then shows only the two buttons; switch the provider back on
and the email form returns, with no new build.

**Public contact address without a new account:** a Google Group made with your existing Google
account.
1. <https://groups.google.com> → **Create group**.
2. Name `SwiftWin support`, email `swiftwin-support` (it becomes `swiftwin-support@googlegroups.com`;
   pick another if taken). **Next**.
3. Privacy: **Who can join: Only invited users**, **Who can view conversations: Group managers**,
   **Who can post: Anyone on the web**. **Create group**. Messages to it arrive in your Gmail; senders
   never see your address.
4. Done (2026-11-11): `swiftwin-support@googlegroups.com` is in PRIVACY.md, TERMS.md and site/index.md (after a change,
   then `bash scripts/update-site.sh`), and choose it as **User support email** in Google Auth Platform →
   Branding (a group you manage is allowed there).

To turn email sign-up on later, set up a sender first (below), then switch the Email provider back on.

## 2b. Later: send email through a real sender (custom SMTP)

Supabase's built-in sender is for trying things out: it only delivers to members of the project's team
(others get "Email address not authorized") and about 2 emails an hour. Sign-up codes, confirmation
links and password resets need a real sender.

**Two ways.** No domain? Use **2-free** (a dedicated Gmail account, free, about 500 emails a day).
With a domain, **2a–2c** (Resend) looks more professional and scales further. You can switch later
without changing SwiftWin.

### 2-free. Send through a dedicated Gmail account (no domain)

1. **Create a new Google account** just for SwiftWin, e.g. `swiftwin.app@gmail.com` (you create it; it
   keeps your personal address out of every email users get).
2. In that account: <https://myaccount.google.com/security> → turn on **2-Step Verification**.
3. <https://myaccount.google.com/apppasswords> → app name `Supabase` → **Create**. Google shows a
   16-letter **app password** once. Don't save it anywhere but the next step.
4. Supabase → **Authentication → Emails → SMTP Settings** → **Enable custom SMTP**:
   - **Sender email:** the new Gmail address (Gmail only sends as itself)
   - **Sender name:** `SwiftWin`
   - **Host:** `smtp.gmail.com`  **Port:** `465`
   - **Username:** the new Gmail address  **Password:** the app password → **Save**
5. Test as in **2c**. Gmail allows roughly 500 messages a day from a normal account; plenty to start.
   If Google ever blocks sending, it emails that account: check its inbox now and then.

### With your own domain: Resend

You need a **domain you own** (for example `swiftwin.dev`, about 10–15 USD a year at any registrar).
The same domain later serves the home page and Google's brand check (section 3). Buying one is your call.

### 2a. Resend account and domain

1. Sign up at <https://resend.com> (the free plan was 3,000 emails a month, 100 a day, one domain; check
   <https://resend.com/pricing>).
2. **Domains → Add domain**. Use a subdomain just for these emails, e.g. `auth.swiftwin.dev`: Resend
   recommends subdomains so this mail's reputation is kept apart from anything else on the domain.
   Pick the region nearest your users.
3. Resend shows DNS records. Add **exactly what it shows** at your domain's DNS host. They are usually:

| Type | Name (host) | Value | Purpose |
|---|---|---|---|
| MX | `send.auth` | `feedback-smtp.<region>.amazonses.com`, priority 10 | bounces come back to Resend |
| TXT | `send.auth` | `v=spf1 include:amazonses.com ~all` | SPF: Resend may send for you |
| TXT | `resend._domainkey.auth` | a long `p=...` key | DKIM: signs every message |
| TXT (add yourself) | `_dmarc` | `v=DMARC1; p=none; rua=mailto:you@your-domain` | DMARC: tells inboxes what to do with failures; tighten to `p=quarantine` later |

   Many DNS hosts add the domain to the name by themselves: type `send.auth`, not
   `send.auth.swiftwin.dev`. With Cloudflare, set these records to **DNS only** (grey cloud).
4. Click **Verify DNS records**. Often a few minutes; can take up to 72 hours.
5. **API Keys → Create API key**, permission **Sending access**, domain = the one you added. Copy it
   once; **don't paste it anywhere but Supabase** (not in the repo, not in chat).

### 2b. Supabase SMTP settings

1. Supabase dashboard → **Authentication → Emails → SMTP Settings** (older layout: Authentication →
   Notifications → Email → SMTP Settings). Turn on **Enable custom SMTP**.
2. Fill in:
   - **Sender email:** `no-reply@auth.swiftwin.dev` (any address on the verified domain)
   - **Sender name:** `SwiftWin`
   - **Host:** `smtp.resend.com`  **Port:** `465`
   - **Username:** `resend`  **Password:** the Resend API key
3. **Save**.
4. **Authentication → Rate Limits**: with custom SMTP Supabase starts at 30 emails an hour. Raise it if
   many people sign up at once (Resend's own daily limit still applies).
5. Optional: **Authentication → Emails → Templates**. The app works with the default templates (it
   follows the link, and also accepts the 6-digit `{{ .Token }}` code if a template shows it).

### 2c. Test it

1. In SwiftWin, sign out, **Create an account** with an address that is **not** on the Supabase team
   (a friend's, or a Gmail alias).
2. The email should arrive within a minute, from `SwiftWin <no-reply@auth...>`, not in spam. Open the
   link on this PC: SwiftWin signs in by itself.
3. Try **Forgot password?** too.
4. In Gmail, **⋮ → Show original** should say `SPF: PASS`, `DKIM: PASS`, `DMARC: PASS`.
5. Resend's **Emails** page lists each message and whether it was delivered.

If nothing arrives: Supabase **Logs → Auth** shows the SMTP error (wrong key, domain not verified).

---

## 3. Google sign-in for everyone

In Google Cloud, the OAuth app ("Google Auth Platform") starts in **Testing**: only up to 100 listed test
users can sign in, and their sign-in expires after 7 days. To let anyone in, publish it.

SwiftWin asks only for `openid`, `email` and `profile`. These are **non-sensitive** scopes: publishing
needs **no** Google review. Review ("brand verification") is needed only if you want your **app name and
logo** shown on Google's screen.

### 3a. Publish (free, no review)

1. <https://console.cloud.google.com> → pick the project of SwiftWin's OAuth client → **Google Auth
   Platform**.
2. **Branding:** app name `SwiftWin`, user support email (yours), developer contact email. Leave the
   logo **empty** (a logo triggers the review). Home page / privacy policy / terms links: fill them in
   once they exist on a public URL (see below); they are optional for publishing.
3. **Data Access:** only `.../auth/userinfo.email`, `.../auth/userinfo.profile` and `openid`. Remove
   anything else: extra scopes would make Google show an "unverified app" warning.
4. **Audience:** User type **External** → **Publish app** → confirm. Status becomes **In production**.
5. Until then (or for testing), **Audience → Test users → Add users** lets specific Google accounts in.

People then see "Choose an account to continue to `<project-ref>.supabase.co`". That works for everyone,
but shows the Supabase address instead of "SwiftWin".

### 3b. Show "SwiftWin" and a logo (optional, later)

Google checks that every domain on the consent screen belongs to you. The `supabase.co` address can't be
yours, so this needs:

1. **Your own domain** (the one from section 2) with a simple home page, e.g. GitHub Pages with a custom
   domain (free) once the repo is public. The page must say what SwiftWin is and link to the privacy policy.
2. **A Supabase custom domain** such as `auth.swiftwin.dev` (a paid Supabase add-on; needs a paid plan).
   Then add the new callback `https://auth.swiftwin.dev/auth/v1/callback` to the Google client, and
   change `SWIFTWIN_BUILD_SUPABASE_URL` in `swiftwin-build.env` (git-ignored; see first-run.md Part F) and rebuild.
3. **Google Search Console:** verify ownership of the domain (a DNS TXT record).
4. Google Auth Platform → **Branding**: logo (120×120 PNG), home page `https://swiftwin.dev`, privacy
   policy `https://swiftwin.dev/privacy` (the text of [PRIVACY.md](../PRIVACY.md)), terms
   `https://swiftwin.dev/terms`, **Authorized domains** `swiftwin.dev` → **Submit for verification**.
   Google says it usually takes 2–3 business days.

Until then 3a is enough: sign-in works for everyone.

### GitHub sign-in

GitHub OAuth apps have no review or test mode: "Continue with GitHub" already works for everyone.
