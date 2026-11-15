//! The app's account and GitHub upload commands (`account_*`, `github_*`, `upload_*`).
//!
//! These are for the person at the keyboard only: the local MCP bridge (what Claude Code
//! talks to) refuses them, see `http.rs`. No command ever returns a token.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use swiftwin_account::upload::{self, Links, Options};
use swiftwin_account::{fake, Accounts};
use swiftwin_core::{Result, SwiftWinError};

/// Commands that must never be reachable from the MCP bridge.
pub fn is_private(cmd: &str) -> bool {
    cmd.starts_with("account_") || cmd.starts_with("signing_") || cmd.starts_with("setup_") || cmd.starts_with("github_") || cmd.starts_with("upload_") || cmd == "open_url"
}

pub struct AccountUi {
    pub accounts: Arc<Accounts>,
    links: Links,
    /// Restoring the session at start-up (a network call) runs in the background.
    restoring: Arc<AtomicBool>,
    job: Arc<Mutex<Value>>,
    /// `--mock`: the fake services live as long as the app.
    _fake: Option<fake::FakeCloud>,
}

impl AccountUi {
    pub fn new(mock: bool) -> AccountUi {
        let (accounts, fake_cloud, links) = if mock {
            let cloud = fake::FakeCloud::start();
            let dir = std::env::temp_dir().join(format!("swiftwin-mock-account-{}", std::process::id()));
            let a = Accounts::new(cloud.settings(), Arc::new(swiftwin_signer::secrets::MemoryStore::default()), fake::fake_browser(), dir.clone());
            (a, Some(cloud), Links::new(dir.join("uploads.json")))
        } else {
            (Accounts::platform(), None, Links::new(swiftwin_core::paths::data_dir().join("uploads.json")))
        };
        let accounts = Arc::new(accounts);
        let restoring = Arc::new(AtomicBool::new(true));
        let (a, r) = (accounts.clone(), restoring.clone());
        std::thread::spawn(move || {
            let _ = a.restore();
            r.store(false, Ordering::SeqCst);
        });
        AccountUi { accounts, links, restoring, job: Arc::new(Mutex::new(json!({"running": false}))), _fake: fake_cloud }
    }

    fn signed_in(&self) -> Result<()> {
        if self.accounts.configured() && self.accounts.user().is_none() {
            return Err(SwiftWinError::Auth("sign in first".into()));
        }
        Ok(())
    }

    pub fn dispatch(&self, cmd: &str, args: &Value, project: Option<PathBuf>) -> Result<Value> {
        let s = |k: &str| args[k].as_str().unwrap_or("").to_string();
        let a = &self.accounts;
        match cmd {
            "account_status" => {
                let mut v = a.status();
                v["loading"] = json!(self.restoring.load(Ordering::SeqCst));
                v["github"] = a.github.status();
                if let Some(f) = &self._fake {
                    // `--mock` only: the "inbox", so the UI test can click the email's link.
                    v["mock_email_link"] = json!(f.state.lock().unwrap().last_link);
                }
                Ok(v)
            }
            "account_sign_up" => a.sign_up(&s("email"), &s("password")),
            "account_confirm" => Ok(json!({"user": a.confirm_sign_up(&s("email"), &s("code"))?})),
            "account_resend" => a.resend_code(&s("email")).map(|_| json!({})),
            "account_sign_in" => Ok(json!({"user": a.sign_in(&s("email"), &s("password"))?})),
            "account_forgot" => a.send_reset(&s("email")).map(|_| json!({})),
            "account_set_password" => Ok(json!({"user": a.set_new_password(&s("password"))?})),
            "account_reset" => Ok(json!({"user": a.reset_password(&s("email"), &s("code"), &s("password"))?})),
            "account_update_name" => Ok(json!({"user": a.update_name(&s("name"))?})),
            "account_change_password" => Ok(json!({"user": a.change_password(&s("current"), &s("password"))?})),
            "account_avatar" => Ok(json!({"avatar": a.avatar()})),
            "account_browser" => Ok(json!({"url": a.start_browser_sign_in(&s("provider"))?})),
            "account_cancel" => {
                a.cancel_browser_sign_in();
                Ok(json!({}))
            }
            "account_sign_out" => {
                if self.job.lock().unwrap()["running"] == true {
                    return Err(SwiftWinError::invalid("wait for the upload to finish"));
                }
                a.sign_out().map(|_| json!({}))
            }
            "account_delete" => {
                if self.job.lock().unwrap()["running"] == true {
                    return Err(SwiftWinError::invalid("wait for the upload to finish"));
                }
                a.delete_account(&s("confirm")).map(|_| json!({}))
            }
            "github_status" => Ok(a.github.status()),
            "github_connect" => {
                self.signed_in()?;
                a.github.connect()
            }
            "github_cancel" => {
                a.github.cancel();
                Ok(json!({}))
            }
            "github_disconnect" => a.github.disconnect().map(|_| json!({})),
            "upload_preview" => {
                self.signed_in()?;
                let dir = project.ok_or_else(|| SwiftWinError::invalid("open a project first"))?;
                let mut v = upload::preview(&dir, &self.links)?;
                v["github"] = a.github.status();
                Ok(v)
            }
            "upload_start" => {
                self.signed_in()?;
                let dir = project.ok_or_else(|| SwiftWinError::invalid("open a project first"))?;
                let token = a.github.token()?;
                let opts = Options {
                    name: Some(s("name")).filter(|n| !n.trim().is_empty()),
                    public: args["public"].as_bool().unwrap_or(false),
                    message: Some(s("message")),
                };
                self.start_upload(dir, token, opts)
            }
            "upload_status" => Ok(self.job.lock().unwrap().clone()),
            "open_url" => {
                let u = s("url");
                if !u.starts_with("https://") {
                    return Err(SwiftWinError::invalid("only https links can be opened"));
                }
                swiftwin_account::account::system_opener()(&u).map(|_| json!({}))
            }
            other => Err(SwiftWinError::invalid(format!("unknown command {other}"))),
        }
    }

    fn start_upload(&self, dir: PathBuf, token: String, opts: Options) -> Result<Value> {
        {
            let mut j = self.job.lock().unwrap();
            if j["running"] == true {
                return Err(SwiftWinError::invalid("an upload is already running"));
            }
            *j = json!({"running": true, "step": "Starting", "done": 0, "total": 0});
        }
        let job = self.job.clone();
        let api = self.accounts.github.api().to_string();
        let links = self.links.clone();
        std::thread::spawn(move || {
            let progress = |step: &str, done: usize, total: usize| {
                let mut j = job.lock().unwrap();
                *j = json!({"running": true, "step": step, "done": done, "total": total});
            };
            let r = upload::upload(&api, &token, Path::new(&dir), &opts, &links, &progress);
            let mut j = job.lock().unwrap();
            *j = match r {
                Ok(o) => json!({"running": false, "result": o}),
                Err(e) => json!({"running": false, "error": e.to_string()}),
            };
        });
        Ok(json!({"started": true}))
    }
}
