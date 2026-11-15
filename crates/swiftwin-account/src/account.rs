//! The signed-in state of SwiftWin: one `Accounts` per process (app or CLI).
//!
//! Storage: the refresh token in the secret store (Windows Credential Manager); the public
//! profile (`User`) in `<data dir>/account.json`; the access token only in memory.
//!
//! Supabase rotates refresh tokens and revokes the whole session if an old one is reused, so a
//! refresh always reads the latest token from the store first and saves the new one right away,
//! and `whoami` in the CLI never refreshes (the app and the CLI share the token).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use swiftwin_core::{Result, SwiftWinError};
use swiftwin_signer::secrets::SecretStore;

use crate::github::GitHubLink;
use crate::loopback::Loopback;
use crate::settings::AuthSettings;
use crate::supabase::{session_is_dead, AuthClient, Session, SignUp, User};
use crate::{pkce, REFRESH_TOKEN};

/// Opens a URL in the user's browser.
pub type Opener = Arc<dyn Fn(&str) -> Result<()> + Send + Sync>;

// UNTESTED-ON-DEVICE: tests use the fake browser.
/// The system browser. Only `https://` (and loopback `http://` for the fakes) is ever opened.
pub fn system_opener() -> Opener {
    Arc::new(|u: &str| {
        check_openable(u)?;
        open::that_detached(u).map_err(|e| SwiftWinError::other(format!("could not open the browser: {e}")))
    })
}

pub fn check_openable(u: &str) -> Result<()> {
    let p = url::Url::parse(u).map_err(|e| SwiftWinError::invalid(format!("bad URL: {e}")))?;
    let loopback = matches!(p.host_str(), Some("127.0.0.1" | "localhost"));
    if p.scheme() == "https" || (p.scheme() == "http" && loopback) {
        Ok(())
    } else {
        Err(SwiftWinError::invalid(format!("refusing to open {u}")))
    }
}

/// How long the browser may take for Google / GitHub sign-in.
const OAUTH_TIMEOUT: Duration = Duration::from_secs(600);
/// How long the app listens for the link in a confirmation or password-reset email.
const EMAIL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub const PROVIDERS: [&str; 2] = ["google", "github"];

#[derive(Default)]
struct State {
    session: Option<Session>,
    /// Signed in according to the stored token, but the service couldn't be reached.
    offline_user: Option<User>,
    /// What the app is waiting for: `google` / `github` (browser sign-in), `email-signup` or
    /// `email-reset` (the link in an email).
    pending: Option<String>,
    /// Signed in through a password-reset link: the app must ask for a new password.
    recovered: bool,
    cancel: Option<Arc<AtomicBool>>,
    error: Option<String>,
    /// (picture URL, `data:` URL) of the last profile picture fetched.
    avatar: Option<(String, String)>,
    /// Email + password sign-in on at the service; `None` until checked (shown as on).
    email_enabled: Option<bool>,
}

pub struct Accounts {
    settings: AuthSettings,
    store: Arc<dyn SecretStore>,
    opener: Opener,
    profile_file: PathBuf,
    state: Arc<Mutex<State>>,
    pub github: GitHubLink,
}

impl Accounts {
    /// `dir` holds `account.json` and `github-login.txt` (the data dir in the app).
    pub fn new(settings: AuthSettings, store: Arc<dyn SecretStore>, opener: Opener, dir: PathBuf) -> Accounts {
        let github = GitHubLink::new(settings.clone(), store.clone(), opener.clone(), dir.join("github-login.txt"));
        Accounts { settings, store, opener, profile_file: dir.join("account.json"), state: Arc::new(Mutex::new(State::default())), github }
    }

    /// The real thing: settings from `AuthSettings::load`, Credential Manager, system browser.
    pub fn platform() -> Accounts {
        Accounts::new(AuthSettings::load(), Arc::from(swiftwin_signer::secrets::platform_store()), system_opener(), swiftwin_core::paths::data_dir())
    }

    fn client(&self) -> Result<AuthClient> {
        if !self.settings.accounts_configured() {
            return Err(SwiftWinError::invalid("accounts are not set up in this build of SwiftWin (no Supabase project)"));
        }
        self.settings.validate()?;
        Ok(AuthClient::new(&self.settings.supabase_url, &self.settings.supabase_key))
    }

    pub fn configured(&self) -> bool {
        self.settings.accounts_configured()
    }

    /// The cached profile, without any network call (CLI `whoami`).
    pub fn cached_user(&self) -> Option<User> {
        self.store.get(REFRESH_TOKEN).ok().flatten()?;
        std::fs::read(&self.profile_file).ok().and_then(|d| serde_json::from_slice(&d).ok())
    }

    pub fn user(&self) -> Option<User> {
        let st = self.state.lock().unwrap();
        st.session.as_ref().map(|s| s.user.clone()).or_else(|| st.offline_user.clone())
    }

    /// The paid-plan hook: `free` unless the server set `app_metadata.plan`.
    pub fn plan(&self) -> String {
        self.user().map(|u| u.plan).unwrap_or_else(|| "free".into())
    }

    fn save(&self, s: Session) -> Result<User> {
        self.store.set(REFRESH_TOKEN, &s.refresh_token)?;
        if let Some(d) = self.profile_file.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(&self.profile_file, serde_json::to_vec_pretty(&s.user)?)?;
        let u = s.user.clone();
        let mut st = self.state.lock().unwrap();
        st.session = Some(s);
        st.offline_user = None;
        st.error = None;
        Ok(u)
    }

    /// Keep the updated profile (in memory and in `account.json`).
    fn set_user(&self, u: User) -> Result<User> {
        std::fs::write(&self.profile_file, serde_json::to_vec_pretty(&u)?)?;
        let mut st = self.state.lock().unwrap();
        if let Some(s) = st.session.as_mut() {
            s.user = u.clone();
        }
        Ok(u)
    }

    fn signed_in_user(&self) -> Result<User> {
        let st = self.state.lock().unwrap();
        st.session.as_ref().map(|s| s.user.clone()).ok_or_else(|| SwiftWinError::Auth("sign in first (or check your connection)".into()))
    }

    /// Change the name SwiftWin shows (`user_metadata.display_name`).
    pub fn update_name(&self, name: &str) -> Result<User> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            return Err(SwiftWinError::invalid("enter a name of 1 to 64 characters"));
        }
        self.signed_in_user()?;
        let at = self.access_token()?;
        let mut u = self.client()?.set_display_name(&at, name)?;
        if u.id.is_empty() {
            u = User { name: Some(name.into()), ..self.signed_in_user()? };
        }
        self.set_user(u)
    }

    /// Change the password of an email account. The current password is checked first by
    /// signing in with it, which also makes the session fresh for Supabase's "secure password
    /// change" setting.
    pub fn change_password(&self, current: &str, new_password: &str) -> Result<User> {
        let u = self.signed_in_user()?;
        if u.provider != "email" {
            return Err(SwiftWinError::invalid(format!("you sign in with {}: there is no SwiftWin password to change", provider_name(&u.provider))));
        }
        check_password(new_password)?;
        if current == new_password {
            return Err(SwiftWinError::invalid("the new password must be different"));
        }
        let c = self.client()?;
        let s = c.sign_in_password(&u.email, current).map_err(|e| match e {
            SwiftWinError::InvalidInput(m) if m.starts_with("Wrong email") => SwiftWinError::invalid("your current password is wrong"),
            e => e,
        })?;
        let at = s.access_token.clone();
        self.save(s)?;
        c.set_password(&at, new_password)?;
        self.signed_in_user()
    }

    /// The profile picture as a `data:` URL (the UI's CSP allows no remote images), fetched
    /// once. Only Google's and GitHub's picture hosts are fetched, so a crafted profile can't
    /// make the app call anything else; `None` → the UI shows initials.
    pub fn avatar(&self) -> Option<String> {
        let url = self.user()?.avatar_url?;
        if let Some((u, d)) = &self.state.lock().unwrap().avatar {
            if *u == url {
                return Some(d.clone());
            }
        }
        if !avatar_allowed(&url, &self.settings.supabase_url) {
            return None;
        }
        let data = fetch_image(&url)?;
        self.state.lock().unwrap().avatar = Some((url, data.clone()));
        Some(data)
    }

    fn forget(&self) -> Result<()> {
        self.store.delete(REFRESH_TOKEN)?;
        let _ = std::fs::remove_file(&self.profile_file);
        let mut st = self.state.lock().unwrap();
        st.session = None;
        st.offline_user = None;
        Ok(())
    }

    /// At start-up: turn the stored refresh token into a session. A dead token signs out; a
    /// network failure keeps the person signed in (offline) with the cached profile.
    pub fn restore(&self) -> Result<Option<User>> {
        if !self.configured() {
            return Ok(None);
        }
        if let Ok(on) = self.client().and_then(|c| c.email_enabled()) {
            self.state.lock().unwrap().email_enabled = Some(on);
        }
        match self.refresh() {
            Ok(u) => Ok(u),
            Err(e) if session_is_dead(&e) => {
                self.forget()?;
                Ok(None)
            }
            Err(_) => {
                let u = self.cached_user();
                self.state.lock().unwrap().offline_user = u.clone();
                Ok(u)
            }
        }
    }

    fn refresh(&self) -> Result<Option<User>> {
        let Some(rt) = self.store.get(REFRESH_TOKEN)? else { return Ok(None) };
        let s = self.client()?.refresh(&rt)?;
        self.save(s).map(Some)
    }

    /// A valid access token (refreshing if needed).
    pub fn access_token(&self) -> Result<String> {
        let cur = self.state.lock().unwrap().session.clone();
        match cur {
            Some(s) if !s.expired() => Ok(s.access_token),
            _ => {
                self.refresh()?.ok_or_else(|| SwiftWinError::Auth("not signed in".into()))?;
                Ok(self.state.lock().unwrap().session.as_ref().map(|s| s.access_token.clone()).unwrap_or_default())
            }
        }
    }

    pub fn status(&self) -> Value {
        let st = self.state.lock().unwrap();
        let user = st.session.as_ref().map(|s| s.user.clone()).or_else(|| st.offline_user.clone());
        json!({
            "configured": self.configured(),
            "signed_in": user.is_some(),
            "offline": st.session.is_none() && st.offline_user.is_some(),
            "user": user,
            "pending": st.pending,
            "email": st.email_enabled.unwrap_or(true),
            "recovered": st.recovered,
            "error": st.error,
        })
    }

    /// Create an account. The confirmation email's link signs in on this PC by itself (the app
    /// listens on a loopback port, PKCE); a 6-digit code works too when the template shows one.
    pub fn sign_up(&self, email: &str, password: &str) -> Result<Value> {
        let email = check_email(email)?;
        check_password(password)?;
        let client = self.client()?;
        self.cancel_browser_sign_in();
        let lb = Loopback::start()?;
        let verifier = pkce::verifier();
        let challenge = pkce::challenge(&verifier);
        match client.sign_up(&email, password, Some((&lb.redirect_uri(), &challenge)))? {
            SignUp::SignedIn(s) => Ok(json!({"signed_in": true, "user": self.save(s)?})),
            SignUp::CodeSent => {
                self.wait_for_code("email-signup", lb, verifier, client, EMAIL_TIMEOUT);
                Ok(json!({"signed_in": false, "code_sent": true, "email": email}))
            }
        }
    }

    pub fn confirm_sign_up(&self, email: &str, code: &str) -> Result<User> {
        let s = self.client()?.verify_code("signup", &check_email(email)?, &check_code(code)?)?;
        self.cancel_browser_sign_in();
        self.save(s)
    }

    pub fn resend_code(&self, email: &str) -> Result<()> {
        self.client()?.resend_signup(&check_email(email)?)
    }

    pub fn sign_in(&self, email: &str, password: &str) -> Result<User> {
        if password.is_empty() {
            return Err(SwiftWinError::invalid("enter your password"));
        }
        let s = self.client()?.sign_in_password(&check_email(email)?, password)?;
        self.cancel_browser_sign_in();
        self.save(s)
    }

    /// Email a password-reset link (and code). Opening the link on this PC signs in and sets
    /// `recovered`, so the app asks for the new password (`set_new_password`).
    pub fn send_reset(&self, email: &str) -> Result<()> {
        let email = check_email(email)?;
        let client = self.client()?;
        self.cancel_browser_sign_in();
        let lb = Loopback::start()?;
        let verifier = pkce::verifier();
        client.send_reset(&email, Some((&lb.redirect_uri(), &pkce::challenge(&verifier))))?;
        self.wait_for_code("email-reset", lb, verifier, client, EMAIL_TIMEOUT);
        Ok(())
    }

    /// Change the password of the signed-in account (after a reset link).
    pub fn set_new_password(&self, new_password: &str) -> Result<User> {
        check_password(new_password)?;
        let at = self.access_token()?;
        self.client()?.set_password(&at, new_password)?;
        let mut st = self.state.lock().unwrap();
        st.recovered = false;
        st.session.as_ref().map(|s| s.user.clone()).ok_or_else(|| SwiftWinError::Auth("not signed in".into()))
    }

    /// Finish "Forgot password": the code from the email signs in, then the password changes.
    pub fn reset_password(&self, email: &str, code: &str, new_password: &str) -> Result<User> {
        check_password(new_password)?;
        let c = self.client()?;
        let s = c.verify_code("recovery", &check_email(email)?, &check_code(code)?)?;
        c.set_password(&s.access_token, new_password)?;
        self.cancel_browser_sign_in();
        self.save(s)
    }

    /// Start Google / GitHub sign-in in the system browser. Returns the URL (shown in the app in
    /// case the browser didn't open); the result arrives in `status()`.
    pub fn start_browser_sign_in(&self, provider: &str) -> Result<String> {
        if !PROVIDERS.contains(&provider) {
            return Err(SwiftWinError::invalid(format!("unknown sign-in provider {provider}")));
        }
        let client = self.client()?;
        self.cancel_browser_sign_in();
        let lb = Loopback::start()?;
        let verifier = pkce::verifier();
        let url = client.authorize_url(provider, &lb.redirect_uri(), &pkce::challenge(&verifier));
        self.wait_for_code(provider, lb, verifier, client, OAUTH_TIMEOUT);
        if let Err(e) = (self.opener)(&url) {
            self.state.lock().unwrap().error = Some(format!("{e}; open the link below yourself"));
        }
        Ok(url)
    }

    /// In the background: wait for `?code=` on the loopback address, exchange it (PKCE) and
    /// sign in. `kind` is shown in `status().pending` meanwhile.
    fn wait_for_code(&self, kind: &str, lb: Loopback, verifier: String, client: AuthClient, timeout: Duration) {
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut st = self.state.lock().unwrap();
            st.pending = Some(kind.into());
            st.cancel = Some(cancel.clone());
            st.error = None;
            st.recovered = false;
        }
        let recovery = kind == "email-reset";
        let (state, store, file) = (self.state.clone(), self.store.clone(), self.profile_file.clone());
        std::thread::spawn(move || {
            let r = lb.wait(timeout, &cancel).and_then(|code| client.exchange_code(&code, &verifier));
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            let r = r.and_then(|s| {
                store.set(REFRESH_TOKEN, &s.refresh_token)?;
                if let Some(d) = file.parent() {
                    std::fs::create_dir_all(d)?;
                }
                std::fs::write(&file, serde_json::to_vec_pretty(&s.user)?)?;
                Ok(s)
            });
            let mut st = state.lock().unwrap();
            st.pending = None;
            st.cancel = None;
            match r {
                Ok(s) => {
                    st.session = Some(s);
                    st.offline_user = None;
                    st.recovered = recovery;
                }
                Err(e) => st.error = Some(e.to_string()),
            }
        });
    }

    pub fn cancel_browser_sign_in(&self) {
        let mut st = self.state.lock().unwrap();
        if let Some(c) = st.cancel.take() {
            c.store(true, Ordering::SeqCst);
        }
        st.pending = None;
    }

    /// Wait for a browser sign-in to finish (CLI and tests).
    pub fn wait_browser_sign_in(&self, timeout: Duration) -> Result<User> {
        let end = std::time::Instant::now() + timeout;
        loop {
            {
                let st = self.state.lock().unwrap();
                if st.pending.is_none() {
                    if let Some(s) = &st.session {
                        return Ok(s.user.clone());
                    }
                    return Err(SwiftWinError::other(st.error.clone().unwrap_or_else(|| "sign-in did not finish".into())));
                }
            }
            if std::time::Instant::now() > end {
                return Err(SwiftWinError::Timeout("browser sign-in".into()));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Sign out here: end the session at Supabase (best effort), forget the tokens, and
    /// disconnect GitHub so the next person on this PC can't upload to this account.
    pub fn sign_out(&self) -> Result<()> {
        self.cancel_browser_sign_in();
        let at = self.state.lock().unwrap().session.as_ref().map(|s| s.access_token.clone());
        if let (Some(at), Ok(c)) = (at, self.client()) {
            let _ = c.sign_out(&at);
        }
        self.github.disconnect()?;
        self.forget()?;
        self.state.lock().unwrap().error = None;
        Ok(())
    }
}

impl Accounts {
    /// Delete the account for good. `confirm` must be the account's email, typed by the person.
    /// Afterwards this PC is signed out: tokens, profile and the GitHub link are forgotten.
    pub fn delete_account(&self, confirm: &str) -> Result<()> {
        let u = self.signed_in_user()?;
        if confirm.trim().to_lowercase() != u.email.to_lowercase() {
            return Err(SwiftWinError::invalid(format!("type {} to confirm", u.email)));
        }
        let at = self.access_token()?;
        self.client()?.delete_account(&at)?;
        self.cancel_browser_sign_in();
        self.github.disconnect()?;
        self.forget()?;
        let mut st = self.state.lock().unwrap();
        st.error = None;
        st.avatar = None;
        Ok(())
    }
}

fn provider_name(p: &str) -> &str {
    match p {
        "google" => "Google",
        "github" => "GitHub",
        p => p,
    }
}

/// https on Google's / GitHub's picture hosts; the fake's loopback address in tests and `--mock`.
fn avatar_allowed(u: &str, supabase_url: &str) -> bool {
    let Ok(p) = url::Url::parse(u) else { return false };
    let host = p.host_str().unwrap_or("");
    let picture_host = ["googleusercontent.com", "githubusercontent.com"].iter().any(|h| host == *h || host.ends_with(&format!(".{h}")));
    (p.scheme() == "https" && picture_host) || (p.scheme() == "http" && host == "127.0.0.1" && u.starts_with(&format!("{}/", supabase_url.trim_end_matches('/'))))
}

// UNTESTED-ON-DEVICE: only fetched from the fake so far.
fn fetch_image(u: &str) -> Option<String> {
    use base64::Engine;
    use std::io::Read;
    let r = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).redirects(0).build().get(u).call().ok()?;
    let ct = r.content_type().to_string();
    if r.status() != 200 || !ct.starts_with("image/") || ct.contains(';') || ct.contains('"') {
        return None;
    }
    let mut buf = Vec::new();
    r.into_reader().take(1 << 20).read_to_end(&mut buf).ok()?;
    if buf.is_empty() || buf.len() >= 1 << 20 {
        return None;
    }
    Some(format!("data:{ct};base64,{}", base64::engine::general_purpose::STANDARD.encode(buf)))
}

fn check_email(e: &str) -> Result<String> {
    let e = e.trim().to_lowercase();
    let ok = e.len() <= 254
        && !e.contains(char::is_whitespace)
        && e.split_once('@').is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.'));
    if ok {
        Ok(e)
    } else {
        Err(SwiftWinError::invalid("enter a valid email address"))
    }
}

/// At least 8 characters (the Supabase project may ask for more; its message is shown then).
fn check_password(p: &str) -> Result<()> {
    if p.chars().count() < 8 {
        return Err(SwiftWinError::invalid("use a password of at least 8 characters"));
    }
    if p.len() > 72 {
        return Err(SwiftWinError::invalid("use a password of at most 72 characters"));
    }
    Ok(())
}

fn check_code(c: &str) -> Result<String> {
    let c: String = c.chars().filter(|ch| !ch.is_whitespace() && *ch != '-').collect();
    if (6..=10).contains(&c.len()) && c.chars().all(|ch| ch.is_ascii_digit()) {
        Ok(c)
    } else {
        Err(SwiftWinError::invalid("enter the 6-digit code from the email"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_checks() {
        assert_eq!(check_email(" Me@Example.com ").unwrap(), "me@example.com");
        assert!(check_email("nope").is_err() && check_email("a@b").is_err() && check_email("a b@c.d").is_err());
        assert!(check_password("short").is_err() && check_password("long enough").is_ok());
        assert_eq!(check_code("123 456").unwrap(), "123456");
        assert!(check_code("12a456").is_err());
        assert!(check_openable("https://github.com/login/device").is_ok());
        assert!(check_openable("http://127.0.0.1:5/x").is_ok());
        assert!(check_openable("file:///C:/Windows/system32/calc.exe").is_err());
        assert!(check_openable("http://evil.example").is_err());
        let sb = "http://127.0.0.1:5";
        assert!(avatar_allowed("https://lh3.googleusercontent.com/a/x=s96-c", sb));
        assert!(avatar_allowed("https://avatars.githubusercontent.com/u/1?v=4", sb));
        assert!(avatar_allowed("http://127.0.0.1:5/avatar.svg", sb));
        for bad in ["http://avatars.githubusercontent.com/u/1", "https://evilgithubusercontent.com/x", "https://githubusercontent.com.evil.io/x", "http://127.0.0.1:6/x", "file:///C:/x"] {
            assert!(!avatar_allowed(bad, sb), "{bad}");
        }
    }
}
