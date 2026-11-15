//! "Connect GitHub": GitHub's OAuth device flow (no client secret in the app) for a
//! `repo`-scoped token, stored in the Credential Manager.
//! Docs: https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use swiftwin_core::{Result, SwiftWinError};
use swiftwin_signer::secrets::SecretStore;

use crate::account::Opener;
use crate::settings::AuthSettings;
use crate::GITHUB_UPLOAD_TOKEN;

pub const SCOPE: &str = "repo";

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build()
}

// UNTESTED-ON-DEVICE: only run against the fake GitHub (fake.rs) so far.
/// Step 1: ask GitHub for a user code.
pub fn request_code(s: &AuthSettings) -> Result<DeviceCode> {
    let r = agent()
        .post(&format!("{}/login/device/code", s.github_web))
        .set("Accept", "application/json")
        .set("User-Agent", "swiftwin")
        .send_form(&[("client_id", &s.github_client_id), ("scope", SCOPE)]);
    let v: Value = match r {
        Ok(r) => r.into_json().map_err(|e| SwiftWinError::Http(e.to_string()))?,
        Err(ureq::Error::Status(c, r)) => {
            let v: Value = r.into_json().unwrap_or(Value::Null);
            return Err(SwiftWinError::Http(format!("GitHub device code: HTTP {c} {}", v["error_description"].as_str().unwrap_or(""))));
        }
        Err(e) => return Err(SwiftWinError::Http(format!("can't reach GitHub: {e}"))),
    };
    if let Some(e) = v["error"].as_str() {
        return Err(device_error(e, &v));
    }
    let st = |k: &str| v[k].as_str().map(str::to_string).ok_or_else(|| SwiftWinError::protocol(format!("GitHub device code: no {k}")));
    Ok(DeviceCode {
        device_code: st("device_code")?,
        user_code: st("user_code")?,
        verification_uri: st("verification_uri")?,
        expires_in: v["expires_in"].as_u64().unwrap_or(900),
        interval: v["interval"].as_u64().unwrap_or(5),
    })
}

fn device_error(e: &str, v: &Value) -> SwiftWinError {
    match e {
        "device_flow_disabled" => SwiftWinError::invalid("Device flow is off for SwiftWin's GitHub OAuth app (turn on \"Enable Device Flow\" in its settings)"),
        "access_denied" => SwiftWinError::Auth("you cancelled the GitHub authorization".into()),
        "expired_token" => SwiftWinError::Timeout("the GitHub code expired; click Connect GitHub again".into()),
        "incorrect_client_credentials" => SwiftWinError::invalid("SwiftWin's GitHub client id is wrong"),
        _ => SwiftWinError::Http(format!("GitHub: {e} {}", v["error_description"].as_str().unwrap_or(""))),
    }
}

/// Step 2: poll until the user approves (or it fails / expires / is cancelled).
pub fn poll_token(s: &AuthSettings, code: &DeviceCode, cancel: &AtomicBool) -> Result<String> {
    let end = Instant::now() + Duration::from_secs(code.expires_in);
    let mut interval = code.interval.max(1);
    loop {
        let wake = Instant::now() + Duration::from_secs(interval);
        while Instant::now() < wake {
            if cancel.load(Ordering::SeqCst) {
                return Err(SwiftWinError::other("cancelled"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if Instant::now() > end {
            return Err(device_error("expired_token", &Value::Null));
        }
        let r = agent()
            .post(&format!("{}/login/oauth/access_token", s.github_web))
            .set("Accept", "application/json")
            .set("User-Agent", "swiftwin")
            .send_form(&[
                ("client_id", s.github_client_id.as_str()),
                ("device_code", code.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ]);
        let v: Value = match r {
            Ok(r) => r.into_json().unwrap_or(Value::Null),
            Err(ureq::Error::Status(_, r)) => r.into_json().unwrap_or(Value::Null),
            // A network blip: keep trying until the code expires.
            Err(_) => continue,
        };
        match v["error"].as_str() {
            None => {
                let token = v["access_token"].as_str().filter(|t| !t.is_empty()).ok_or_else(|| SwiftWinError::protocol("GitHub returned no token"))?;
                let scopes = v["scope"].as_str().unwrap_or("");
                if !scopes.split([',', ' ']).any(|sc| sc == SCOPE) {
                    return Err(SwiftWinError::Auth(format!("GitHub granted {scopes:?} but SwiftWin needs `repo` to create and push to your repositories")));
                }
                return Ok(token.to_string());
            }
            Some("authorization_pending") => {}
            Some("slow_down") => interval = v["interval"].as_u64().unwrap_or(interval + 5).max(interval + 1),
            Some(e) => return Err(device_error(e, &v)),
        }
    }
}

/// The GitHub login for a token (also proves the token works).
pub fn login_for(api: &str, token: &str) -> Result<String> {
    let gh = swiftwin_release::GitHub::account(api, token);
    let v = gh.send("GET", "/user", None)?;
    v["login"].as_str().map(str::to_string).ok_or_else(|| SwiftWinError::protocol("GitHub /user has no login"))
}

#[derive(Default)]
struct State {
    login: Option<String>,
    pending: Option<DeviceCode>,
    cancel: Option<Arc<AtomicBool>>,
    error: Option<String>,
}

/// The app's GitHub connection.
pub struct GitHubLink {
    settings: AuthSettings,
    store: Arc<dyn SecretStore>,
    opener: Opener,
    state: Arc<Mutex<State>>,
    login_file: PathBuf,
}

impl GitHubLink {
    pub fn new(settings: AuthSettings, store: Arc<dyn SecretStore>, opener: Opener, login_file: PathBuf) -> GitHubLink {
        let login = if store.get(GITHUB_UPLOAD_TOKEN).ok().flatten().is_some() {
            std::fs::read_to_string(&login_file).ok().map(|l| l.trim().to_string()).filter(|l| !l.is_empty())
        } else {
            None
        };
        GitHubLink { settings, store, opener, state: Arc::new(Mutex::new(State { login, ..Default::default() })), login_file }
    }

    pub fn configured(&self) -> bool {
        self.settings.github_configured()
    }

    pub fn api(&self) -> &str {
        &self.settings.github_api
    }

    pub fn token(&self) -> Result<String> {
        self.store.get(GITHUB_UPLOAD_TOKEN)?.ok_or_else(|| SwiftWinError::Auth("connect GitHub first".into()))
    }

    pub fn status(&self) -> Value {
        let st = self.state.lock().unwrap();
        json!({
            "configured": self.configured(),
            "connected": st.login.is_some(),
            "login": st.login,
            "pending": st.pending.as_ref().map(|p| json!({"user_code": p.user_code, "verification_uri": p.verification_uri})),
            "error": st.error,
        })
    }

    /// Start the device flow: returns the code to show; approval is awaited in the background.
    pub fn connect(&self) -> Result<Value> {
        if !self.configured() {
            return Err(SwiftWinError::invalid("GitHub connection is not set up in this build (no GitHub client id)"));
        }
        self.cancel();
        let code = request_code(&self.settings)?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut st = self.state.lock().unwrap();
            st.pending = Some(code.clone());
            st.cancel = Some(cancel.clone());
            st.error = None;
        }
        let _ = (self.opener)(&code.verification_uri);
        let (settings, store, state, file) = (self.settings.clone(), self.store.clone(), self.state.clone(), self.login_file.clone());
        let c2 = code.clone();
        std::thread::spawn(move || {
            let r = poll_token(&settings, &c2, &cancel).and_then(|token| {
                let login = login_for(&settings.github_api, &token)?;
                store.set(GITHUB_UPLOAD_TOKEN, &token)?;
                if let Some(d) = file.parent() {
                    std::fs::create_dir_all(d)?;
                }
                std::fs::write(&file, &login)?;
                Ok(login)
            });
            let mut st = state.lock().unwrap();
            if cancel.load(Ordering::SeqCst) {
                return;
            }
            st.pending = None;
            st.cancel = None;
            match r {
                Ok(l) => st.login = Some(l),
                Err(e) => st.error = Some(e.to_string()),
            }
        });
        Ok(json!({"user_code": code.user_code, "verification_uri": code.verification_uri}))
    }

    pub fn cancel(&self) {
        let mut st = self.state.lock().unwrap();
        if let Some(c) = st.cancel.take() {
            c.store(true, Ordering::SeqCst);
        }
        st.pending = None;
    }

    /// Forget the token here. (It stays valid at GitHub until revoked under
    /// Settings > Applications > Authorized OAuth Apps.)
    pub fn disconnect(&self) -> Result<()> {
        self.cancel();
        self.store.delete(GITHUB_UPLOAD_TOKEN)?;
        let _ = std::fs::remove_file(&self.login_file);
        let mut st = self.state.lock().unwrap();
        st.login = None;
        st.error = None;
        Ok(())
    }

    /// Wait for a background connect to finish (CLI and tests).
    pub fn wait(&self, timeout: Duration) -> Result<String> {
        let end = Instant::now() + timeout;
        loop {
            {
                let st = self.state.lock().unwrap();
                if st.pending.is_none() {
                    if let Some(e) = &st.error {
                        return Err(SwiftWinError::other(e.clone()));
                    }
                    if let Some(l) = &st.login {
                        return Ok(l.clone());
                    }
                    return Err(SwiftWinError::other("not connected"));
                }
            }
            if Instant::now() > end {
                return Err(SwiftWinError::Timeout("GitHub authorization".into()));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
