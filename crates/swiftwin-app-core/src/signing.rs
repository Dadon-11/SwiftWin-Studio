//! The Signing dialog's commands (`signing_*`): free Apple ID or App Store Connect key.
//!
//! They take a `.p8` key, so like `account_*` the MCP bridge refuses them
//! (`account::is_private`), and no reply ever contains the key.

use std::sync::Mutex;

use serde_json::{json, Value};
use swiftwin_core::config::Config;
use swiftwin_core::{Result, SwiftWinError};
use swiftwin_signer::secrets::{self, MemoryStore, SecretStore};
use swiftwin_signer::{plumesign, setup, AscKey};

pub struct SigningUi {
    /// `--mock`: settings and the key stay in memory, no plumesign and no Apple calls.
    mock: Option<(Mutex<Config>, MemoryStore)>,
    /// An Apple ID sign-in waiting for its 2FA code (mock: no plumesign).
    login: Mutex<Option<(String, Option<plumesign::LoginSession>)>>,
}

impl SigningUi {
    pub fn new(mock: bool) -> SigningUi {
        SigningUi { mock: mock.then(|| (Mutex::new(Config::default()), MemoryStore::default())), login: Mutex::new(None) }
    }

    fn send_code(&self, code: &str) -> Result<Value> {
        let email = {
            let mut g = self.login.lock().unwrap();
            let (email, session) = g.as_mut().ok_or_else(|| SwiftWinError::invalid("sign in with your Apple ID first"))?;
            match session {
                Some(sess) => sess.send_code(code)?,
                // mock: any 6 digits but 000000
                None if code.len() == 6 && code.chars().all(|c| c.is_ascii_digit()) && code != "000000" => {}
                None => return Err(SwiftWinError::Auth("Apple: incorrect verification code (mock)".into())),
            }
            email.clone()
        };
        if self.mock.is_some() {
            *self.login.lock().unwrap() = None;
            return self.finish(&email);
        }
        self.advance()
    }

    /// Wait for plumesign's next step: the 2FA prompt, success or an error.
    fn advance(&self) -> Result<Value> {
        let mut g = self.login.lock().unwrap();
        let Some((email, Some(sess))) = g.as_mut() else { return Err(SwiftWinError::invalid("sign in with your Apple ID first")) };
        let state = sess.wait(std::time::Duration::from_secs(90));
        let email = email.clone();
        match state {
            plumesign::LoginState::NeedCode => Ok(json!({"need_code": true})),
            plumesign::LoginState::Done => {
                *g = None;
                drop(g);
                self.finish(&email)
            }
            plumesign::LoginState::Failed(msg) => {
                *g = None;
                Err(SwiftWinError::Auth(format!("Apple ID sign-in failed:
{msg}")))
            }
            plumesign::LoginState::Running => {
                *g = None;
                Err(SwiftWinError::Timeout("Apple did not answer within 90 seconds: check the internet connection and try again".into()))
            }
        }
    }

    fn finish(&self, email: &str) -> Result<Value> {
        let prefix = self.with_config(|c, _| {
            let p = setup::use_apple_id(c, email)?;
            // mock: an address with "paid" plays an Apple ID whose team turned out to be paid.
            if self.mock.is_some() && email.contains("paid") {
                c.paid_team_hint = Some("ABCDE12345".into());
            }
            Ok(p)
        })?;
        Ok(json!({"id_prefix": prefix, "limits": setup::FREE_LIMITS}))
    }

    fn with_config<T>(&self, f: impl FnOnce(&mut Config, &dyn SecretStore) -> Result<T>) -> Result<T> {
        match &self.mock {
            Some((cfg, store)) => f(&mut cfg.lock().unwrap(), store),
            None => {
                let mut cfg = Config::load();
                let r = f(&mut cfg, secrets::platform_store().as_ref())?;
                cfg.save()?;
                Ok(r)
            }
        }
    }

    pub fn dispatch(&self, cmd: &str, args: &Value) -> Result<Value> {
        let s = |k: &str| args[k].as_str().unwrap_or("").trim().to_string();
        match cmd {
            "signing_status" => {
                let mut v = self.with_config(|c, st| setup::status(c, st))?;
                v["mock"] = json!(self.mock.is_some());
                Ok(v)
            }
            // Check the key with App Store Connect; with `save`, also store it and switch to it.
            "signing_asc" => {
                let key = AscKey { key_id: s("key_id"), issuer_id: s("issuer_id"), p8_pem: args["p8"].as_str().unwrap_or("").to_string() };
                let team = s("team_id").to_ascii_uppercase();
                let message = if self.mock.is_some() {
                    key.jwt(0)?;
                    setup::check_asc(&key, &team, "http://127.0.0.1:9").map(|_| ()).or_else(|e| match e {
                        SwiftWinError::Http(_) => Ok(()), // mock: nothing listens there; the offline checks passed
                        e => Err(e),
                    })?;
                    "The key is valid (mock: App Store Connect was not called)".to_string()
                } else {
                    setup::check_asc(&key, &team, swiftwin_signer::asc::API_BASE)?
                };
                if args["save"] == true {
                    self.with_config(|c, st| setup::use_asc(c, st, &key, &team))?;
                }
                Ok(json!({"ok": true, "message": message}))
            }
            // Sign in with a free Apple ID through plumesign: `{email, password}`, then `{code}`
            // when Apple asks for the 2FA code. An e-mail plumesign already has a session for
            // needs no password. The password only goes to plumesign's stdin.
            "signing_apple_id" => {
                let code = s("code");
                if !code.is_empty() {
                    return self.send_code(&code);
                }
                let email = s("email");
                if !setup::valid_email(&email) {
                    return Err(SwiftWinError::invalid("enter your Apple ID e-mail"));
                }
                let password = args["password"].as_str().unwrap_or("");
                if self.mock.is_some() {
                    return match password {
                        "" => Err(SwiftWinError::invalid("enter your Apple ID password")),
                        "wrong" => Err(SwiftWinError::Auth("Apple: incorrect Apple ID or password (mock)".into())),
                        _ => {
                            *self.login.lock().unwrap() = Some((email, None));
                            Ok(json!({"need_code": true}))
                        }
                    };
                }
                let ps = setup::ensure_apple_id_tools(&|_| {})?;
                if ps.signed_in_as(&email)? {
                    return self.finish(&email);
                }
                if password.is_empty() {
                    return Err(SwiftWinError::invalid("enter your Apple ID password"));
                }
                let session = plumesign::LoginSession::start(&ps.exe, &email, password)?;
                *self.login.lock().unwrap() = Some((email, Some(session)));
                self.advance()
            }
            other => Err(SwiftWinError::invalid(format!("unknown command {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p8() -> String {
        use p256::pkcs8::EncodePrivateKey;
        p256::ecdsa::SigningKey::random(&mut rand::thread_rng()).to_pkcs8_pem(p256::pkcs8::LineEnding::LF).unwrap().to_string()
    }

    #[test]
    fn mock_switches_modes_and_never_returns_the_key() {
        let ui = SigningUi::new(true);
        assert_eq!(ui.dispatch("signing_status", &json!({})).unwrap()["mode"], "none");
        let key = p8();
        let args = |team: &str, save: bool| json!({"key_id": "KEY123", "issuer_id": "iss", "p8": key, "team_id": team, "save": save});
        assert!(ui.dispatch("signing_asc", &args("nope", false)).is_err());
        assert!(ui.dispatch("signing_asc", &json!({"key_id": "K", "issuer_id": "i", "p8": "junk", "team_id": "ABCDE12345"})).is_err());
        let r = ui.dispatch("signing_asc", &args("abcde12345", false)).unwrap();
        assert!(r["message"].as_str().unwrap().contains("valid"));
        assert_eq!(ui.dispatch("signing_status", &json!({})).unwrap()["mode"], "none", "check only: nothing saved");
        ui.dispatch("signing_asc", &args("ABCDE12345", true)).unwrap();
        let st = ui.dispatch("signing_status", &json!({})).unwrap();
        assert_eq!((st["mode"].as_str(), st["team_id"].as_str(), st["p8_stored"].as_bool()), (Some("asc"), Some("ABCDE12345"), Some(true)));
        assert!(!st.to_string().contains("PRIVATE KEY"));

        assert!(ui.dispatch("signing_apple_id", &json!({"email": "x\" & calc", "password": "p"})).is_err());
        assert!(ui.dispatch("signing_apple_id", &json!({"code": "123456"})).is_err(), "no sign-in started");
        assert!(ui.dispatch("signing_apple_id", &json!({"email": "paid@example.com", "password": "wrong"})).is_err());
        let r = ui.dispatch("signing_apple_id", &json!({"email": "paid@example.com", "password": "pw"})).unwrap();
        assert_eq!(r["need_code"], true);
        assert!(ui.dispatch("signing_apple_id", &json!({"code": "12"})).is_err());
        ui.dispatch("signing_apple_id", &json!({"code": "123456"})).unwrap();
        let st = ui.dispatch("signing_status", &json!({})).unwrap();
        assert_eq!((st["mode"].as_str(), st["paid_team_hint"].as_str()), (Some("apple-id"), Some("ABCDE12345")));
    }
}
