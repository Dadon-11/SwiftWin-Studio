//! Supabase Auth (GoTrue) REST client: only the endpoints SwiftWin uses.
//! Verified against github.com/supabase/auth `openapi.yaml` (see docs/open-questions.md).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use swiftwin_core::{Result, SwiftWinError};

/// The signed-in person. Not secret; cached in `<data dir>/account.json` so the app can
/// show who is signed in while offline.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
    /// `email`, `google` or `github`.
    pub provider: String,
    /// From `app_metadata.plan`, which only the server can set (`free` when absent). This is
    /// where a paid plan or licence will show up; see docs/first-run.md, "Selling SwiftWin".
    #[serde(default = "free")]
    pub plan: String,
    /// The Google / GitHub profile picture (https), shown through `Accounts::avatar`.
    #[serde(default)]
    pub avatar_url: Option<String>,
    /// When the account was created (RFC 3339), for "Member since".
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub email_confirmed: bool,
}

fn free() -> String {
    "free".into()
}

#[derive(Clone, PartialEq)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: i64,
    pub user: User,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("tokens", &"<redacted>").field("expires_at", &self.expires_at).field("user", &self.user).finish()
    }
}

impl Session {
    pub fn from_json(v: &Value) -> Result<Session> {
        let s = |k: &str| v[k].as_str().filter(|t| !t.is_empty()).map(str::to_string);
        let access_token = s("access_token").ok_or_else(|| SwiftWinError::protocol("Supabase returned no access token"))?;
        let refresh_token = s("refresh_token").ok_or_else(|| SwiftWinError::protocol("Supabase returned no refresh token"))?;
        let now = swiftwin_core::time::unix_seconds();
        let expires_at = v["expires_at"].as_i64().unwrap_or_else(|| now + v["expires_in"].as_i64().unwrap_or(3600));
        Ok(Session { access_token, refresh_token, expires_at, user: user_from_json(&v["user"]) })
    }

    pub fn expired(&self) -> bool {
        swiftwin_core::time::unix_seconds() + 60 >= self.expires_at
    }
}

pub fn user_from_json(u: &Value) -> User {
    let meta = &u["user_metadata"];
    // `display_name` is the one SwiftWin sets; the others come from Google / GitHub and are
    // rewritten by them at each sign-in.
    let name = ["display_name", "full_name", "name", "user_name"].iter().find_map(|k| meta[*k].as_str()).filter(|n| !n.is_empty()).map(str::to_string);
    User {
        id: u["id"].as_str().unwrap_or("").into(),
        email: u["email"].as_str().unwrap_or("").into(),
        name,
        provider: u["app_metadata"]["provider"].as_str().unwrap_or("email").into(),
        plan: u["app_metadata"]["plan"].as_str().unwrap_or("free").into(),
        avatar_url: ["avatar_url", "picture"].iter().find_map(|k| meta[*k].as_str()).filter(|a| !a.is_empty()).map(str::to_string),
        created_at: u["created_at"].as_str().map(str::to_string),
        email_confirmed: u["email_confirmed_at"].as_str().is_some_and(|t| !t.is_empty()),
    }
}

/// What `sign_up` led to.
#[derive(Debug)]
pub enum SignUp {
    /// Email confirmation is off in the project: signed in already.
    SignedIn(Session),
    /// A 6-digit code (and a link) was emailed.
    CodeSent,
}

// UNTESTED-ON-DEVICE: only run against the fake Supabase (fake.rs) so far.
pub struct AuthClient {
    base: String,
    key: String,
    agent: ureq::Agent,
}

impl AuthClient {
    /// `project_url` is `https://<ref>.supabase.co`.
    pub fn new(project_url: &str, key: &str) -> AuthClient {
        AuthClient {
            base: format!("{}/auth/v1", project_url.trim_end_matches('/')),
            key: key.into(),
            agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).redirects(0).build(),
        }
    }

    /// The URL to open in the browser for Google / GitHub sign-in (PKCE, S256).
    pub fn authorize_url(&self, provider: &str, redirect_to: &str, challenge: &str) -> String {
        let q: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("provider", provider)
            .append_pair("redirect_to", redirect_to)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "s256")
            .finish();
        format!("{}/authorize?{q}", self.base)
    }

    fn send(&self, method: &str, path: &str, bearer: Option<&str>, body: Option<Value>) -> Result<Value> {
        // Only `apikey` for anonymous calls: it works for both legacy anon JWTs and the newer
        // `sb_publishable_` keys, which are not JWTs and must not be sent as a bearer token.
        let mut req = self
            .agent
            .request(method, &format!("{}{}", self.base, path))
            .set("apikey", &self.key)
            .set("Accept", "application/json")
            .set("X-Client-Info", concat!("swiftwin/", env!("CARGO_PKG_VERSION")));
        if let Some(t) = bearer {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        let res = match body {
            Some(b) => req.send_json(b),
            None => req.call(),
        };
        match res {
            Ok(r) => {
                let text = r.into_string().map_err(|e| SwiftWinError::Http(e.to_string()))?;
                Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
            }
            Err(ureq::Error::Status(code, r)) => Err(api_error(code, &r.into_json().unwrap_or(Value::Null))),
            Err(e) => Err(SwiftWinError::Http(format!("can't reach the sign-in service: {e}"))),
        }
    }

    /// `link` = (redirect_to, PKCE challenge): the confirmation email's link then comes back to
    /// the app's loopback address with `?code=`, like browser sign-in.
    pub fn sign_up(&self, email: &str, password: &str, link: Option<(&str, &str)>) -> Result<SignUp> {
        let mut body = json!({"email": email, "password": password});
        let path = with_link("/signup", &mut body, link);
        let v = self.send("POST", &path, None, Some(body))?;
        if v["access_token"].is_string() {
            Ok(SignUp::SignedIn(Session::from_json(&v)?))
        } else {
            Ok(SignUp::CodeSent)
        }
    }

    pub fn resend_signup(&self, email: &str) -> Result<()> {
        self.send("POST", "/resend", None, Some(json!({"type": "signup", "email": email})))?;
        Ok(())
    }

    pub fn sign_in_password(&self, email: &str, password: &str) -> Result<Session> {
        Session::from_json(&self.send("POST", "/token?grant_type=password", None, Some(json!({"email": email, "password": password})))?)
    }

    pub fn refresh(&self, refresh_token: &str) -> Result<Session> {
        Session::from_json(&self.send("POST", "/token?grant_type=refresh_token", None, Some(json!({"refresh_token": refresh_token})))?)
    }

    pub fn exchange_code(&self, auth_code: &str, verifier: &str) -> Result<Session> {
        Session::from_json(&self.send("POST", "/token?grant_type=pkce", None, Some(json!({"auth_code": auth_code, "code_verifier": verifier})))?)
    }

    /// `kind` is `signup` or `recovery`; `code` is the 6-digit code from the email.
    pub fn verify_code(&self, kind: &str, email: &str, code: &str) -> Result<Session> {
        Session::from_json(&self.send("POST", "/verify", None, Some(json!({"type": kind, "email": email, "token": code})))?)
    }

    pub fn send_reset(&self, email: &str, link: Option<(&str, &str)>) -> Result<()> {
        let mut body = json!({"email": email});
        let path = with_link("/recover", &mut body, link);
        self.send("POST", &path, None, Some(body))?;
        Ok(())
    }

    pub fn set_password(&self, access_token: &str, password: &str) -> Result<User> {
        Ok(user_from_json(&self.send("PUT", "/user", Some(access_token), Some(json!({"password": password})))?))
    }

    /// Set `user_metadata.display_name` (merged into the existing metadata by Supabase).
    pub fn set_display_name(&self, access_token: &str, name: &str) -> Result<User> {
        Ok(user_from_json(&self.send("PUT", "/user", Some(access_token), Some(json!({"data": {"display_name": name}})))?))
    }

    /// Whether email + password sign-in is turned on at the service (`external.email` of the public
    /// `/settings`). The owner can switch it off in Supabase and the app follows.
    pub fn email_enabled(&self) -> Result<bool> {
        Ok(self.send("GET", "/settings", None, None)?["external"]["email"].as_bool().unwrap_or(true))
    }

    /// Delete the signed-in user for good, through the SQL function `public.delete_my_account()`
    /// the owner installs once (docs/accounts-setup.md). Supabase Auth itself only deletes users
    /// with the secret key, which must never be in the app.
    pub fn delete_account(&self, access_token: &str) -> Result<()> {
        let rest = self.base.replace("/auth/v1", "/rest/v1");
        let res = self
            .agent
            .post(&format!("{rest}/rpc/delete_my_account"))
            .set("apikey", &self.key)
            .set("Authorization", &format!("Bearer {access_token}"))
            .set("Accept", "application/json")
            .send_json(json!({}));
        match res {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(code, r)) => {
                let v: Value = r.into_json().unwrap_or(Value::Null);
                // PGRST202: no such function (the owner hasn't run the SQL yet).
                if code == 404 || v["code"] == "PGRST202" {
                    return Err(SwiftWinError::other("deleting accounts is not set up on the sign-in service yet (the owner must add delete_my_account, see docs/accounts-setup.md)"));
                }
                Err(api_error(code, &v))
            }
            Err(e) => Err(SwiftWinError::Http(format!("can't reach the sign-in service: {e}"))),
        }
    }

    /// Ends this session only (`scope=local`), not the user's other devices.
    pub fn sign_out(&self, access_token: &str) -> Result<()> {
        self.send("POST", "/logout?scope=local", Some(access_token), None)?;
        Ok(())
    }
}

/// Add `?redirect_to=` (as supabase-js does) and the PKCE challenge to an email-sending call.
fn with_link(path: &str, body: &mut Value, link: Option<(&str, &str)>) -> String {
    match link {
        Some((redirect, challenge)) => {
            body["code_challenge"] = json!(challenge);
            body["code_challenge_method"] = json!("s256");
            let q: String = url::form_urlencoded::Serializer::new(String::new()).append_pair("redirect_to", redirect).finish();
            format!("{path}?{q}")
        }
        None => path.to_string(),
    }
}

/// Turn a GoTrue error body into a message for the person at the keyboard.
fn api_error(code: u16, v: &Value) -> SwiftWinError {
    let ec = v["error_code"].as_str().or_else(|| v["error"].as_str()).unwrap_or("");
    let msg = ["msg", "error_description", "message"].iter().find_map(|k| v[*k].as_str()).unwrap_or("").to_string();
    let friendly = match ec {
        "invalid_credentials" => "Wrong email or password.".to_string(),
        "email_not_confirmed" => "Confirm your email first: enter the 6-digit code we emailed you.".into(),
        "otp_expired" => "That code is wrong or has expired. Ask for a new one.".into(),
        "same_password" => "That's already your password. Choose a new one.".into(),
        "reauthentication_needed" => "For your security, sign out and sign in again, then change the password.".into(),
        "weak_password" => format!("Choose a stronger password. {msg}"),
        "user_already_exists" | "email_exists" => "An account with this email already exists. Sign in instead.".into(),
        "over_email_send_rate_limit" | "over_request_rate_limit" => "Too many attempts. Wait a minute and try again.".into(),
        "email_address_not_authorized" => {
            "The sign-in service can't email this address yet (its built-in email only reaches the project team; the owner must set up SMTP).".into()
        }
        "signup_disabled" => "New sign-ups are turned off.".into(),
        "provider_disabled" | "validation_failed" if msg.to_lowercase().contains("provider") => format!("This sign-in method is not turned on: {msg}"),
        "refresh_token_not_found" | "refresh_token_already_used" | "session_not_found" | "session_expired" => {
            return SwiftWinError::Auth(format!("your session ended, sign in again ({ec})"));
        }
        "flow_state_not_found" | "flow_state_expired" | "bad_code_verifier" => "The sign-in took too long or was already used. Try again.".into(),
        _ if code == 400 && msg.to_lowercase().contains("refresh token") => return SwiftWinError::Auth(format!("your session ended, sign in again ({msg})")),
        _ if !msg.is_empty() => msg.clone(),
        _ => format!("sign-in service error (HTTP {code})"),
    };
    if code == 401 || code == 403 {
        SwiftWinError::Auth(friendly)
    } else {
        SwiftWinError::InvalidInput(friendly)
    }
}

/// True when the error means the stored session is gone for good (vs. a network problem).
pub fn session_is_dead(e: &SwiftWinError) -> bool {
    matches!(e, SwiftWinError::Auth(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_readable_and_tokens_never_debug_print() {
        let e = api_error(400, &json!({"code": 400, "error_code": "invalid_credentials", "msg": "Invalid login credentials"}));
        assert_eq!(e.to_string(), "invalid input: Wrong email or password.");
        let e = api_error(400, &json!({"error": "invalid_grant", "error_description": "Invalid Refresh Token: Already Used"}));
        assert!(session_is_dead(&e));
        let s = Session::from_json(&json!({"access_token": "AT-SECRET", "refresh_token": "RT-SECRET", "expires_in": 3600,
            "user": {"id": "u", "email": "a@b.c", "app_metadata": {"provider": "github"}, "user_metadata": {"user_name": "octo"}}}))
        .unwrap();
        let d = format!("{s:?}");
        assert!(!d.contains("SECRET") && d.contains("octo") && d.contains("github"));
        assert_eq!(s.user.plan, "free");
        assert!(!s.expired());
        let c = AuthClient::new("https://x.supabase.co/", "k");
        let u = c.authorize_url("google", "http://127.0.0.1:5000/auth/callback", "abc");
        assert!(u.starts_with("https://x.supabase.co/auth/v1/authorize?provider=google&redirect_to=http%3A%2F%2F127.0.0.1%3A5000%2Fauth%2Fcallback&code_challenge=abc&code_challenge_method=s256"));
    }
}
