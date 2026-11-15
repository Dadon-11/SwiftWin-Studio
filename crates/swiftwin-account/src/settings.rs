//! Where the account services are. These values are public by design (they ship inside every
//! copy of the app): the Supabase project URL, its anon/publishable key, and the GitHub OAuth
//! app's client id. Secret keys (`service_role`, `sb_secret_…`, a client secret) are refused.
//!
//! Lookup order: environment (`SWIFTWIN_SUPABASE_URL`, `SWIFTWIN_SUPABASE_KEY`,
//! `SWIFTWIN_GITHUB_CLIENT_ID`), then `<data dir>/auth.json`, then the values built into the app.

use std::path::PathBuf;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use swiftwin_core::{Result, SwiftWinError};

/// Built in at compile time by `build.rs`, from `SWIFTWIN_BUILD_*` environment variables or the
/// git-ignored `swiftwin-build.env` (docs/accounts-setup.md). Empty = accounts not set up.
pub const BUILT_IN_SUPABASE_URL: &str = env!("SWIFTWIN_BUILD_SUPABASE_URL");
pub const BUILT_IN_SUPABASE_KEY: &str = env!("SWIFTWIN_BUILD_SUPABASE_KEY");
pub const BUILT_IN_GITHUB_CLIENT_ID: &str = env!("SWIFTWIN_BUILD_GITHUB_CLIENT_ID");

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AuthSettings {
    /// `https://<project>.supabase.co`
    #[serde(default)]
    pub supabase_url: String,
    /// The anon (legacy JWT) or publishable (`sb_publishable_…`) key.
    #[serde(default)]
    pub supabase_key: String,
    /// GitHub OAuth app client id (device flow enabled).
    #[serde(default)]
    pub github_client_id: String,
    /// `https://api.github.com` (fakes override it).
    #[serde(default)]
    pub github_api: String,
    /// `https://github.com` (device-flow endpoints; fakes override it).
    #[serde(default)]
    pub github_web: String,
}

impl AuthSettings {
    pub fn path() -> PathBuf {
        swiftwin_core::paths::data_dir().join("auth.json")
    }

    pub fn load() -> AuthSettings {
        let mut s = AuthSettings {
            supabase_url: BUILT_IN_SUPABASE_URL.into(),
            supabase_key: BUILT_IN_SUPABASE_KEY.into(),
            github_client_id: BUILT_IN_GITHUB_CLIENT_ID.into(),
            ..Default::default()
        };
        if let Some(f) = std::fs::read(Self::path()).ok().and_then(|d| serde_json::from_slice::<AuthSettings>(&d).ok()) {
            s.merge(f);
        }
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        s.merge(AuthSettings {
            supabase_url: env("SWIFTWIN_SUPABASE_URL").unwrap_or_default(),
            supabase_key: env("SWIFTWIN_SUPABASE_KEY").unwrap_or_default(),
            github_client_id: env("SWIFTWIN_GITHUB_CLIENT_ID").unwrap_or_default(),
            ..Default::default()
        });
        s.with_defaults()
    }

    fn merge(&mut self, o: AuthSettings) {
        for (mine, theirs) in [
            (&mut self.supabase_url, o.supabase_url),
            (&mut self.supabase_key, o.supabase_key),
            (&mut self.github_client_id, o.github_client_id),
            (&mut self.github_api, o.github_api),
            (&mut self.github_web, o.github_web),
        ] {
            if !theirs.trim().is_empty() {
                *mine = theirs.trim().to_string();
            }
        }
    }

    pub fn with_defaults(mut self) -> Self {
        if self.github_api.is_empty() {
            self.github_api = swiftwin_release::github::API.into();
        }
        if self.github_web.is_empty() {
            self.github_web = "https://github.com".into();
        }
        self.supabase_url = self.supabase_url.trim_end_matches('/').to_string();
        self
    }

    /// Sign-in is possible (the owner set up Supabase).
    pub fn accounts_configured(&self) -> bool {
        !self.supabase_url.is_empty() && !self.supabase_key.is_empty()
    }

    pub fn github_configured(&self) -> bool {
        !self.github_client_id.is_empty()
    }

    /// Refuse settings that would leak a secret or talk to the wrong place.
    pub fn validate(&self) -> Result<()> {
        if self.accounts_configured() {
            check_url(&self.supabase_url, "Supabase URL")?;
            check_public_key(&self.supabase_key)?;
        }
        check_url(&self.github_api, "GitHub API URL")?;
        check_url(&self.github_web, "GitHub URL")?;
        Ok(())
    }
}

/// https only, except loopback (the fakes).
fn check_url(u: &str, what: &str) -> Result<()> {
    let p = url::Url::parse(u).map_err(|e| SwiftWinError::invalid(format!("{what} {u:?}: {e}")))?;
    let loopback = matches!(p.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if p.scheme() == "https" || (p.scheme() == "http" && loopback) {
        Ok(())
    } else {
        Err(SwiftWinError::invalid(format!("{what} must use https: {u}")))
    }
}

/// The key must be the public one. A `service_role` / secret key bypasses every database rule
/// and must never ship in a desktop app.
pub fn check_public_key(key: &str) -> Result<()> {
    if key.starts_with("sb_secret_") {
        return Err(SwiftWinError::invalid("that is a Supabase *secret* key; SwiftWin needs the publishable (or anon) key"));
    }
    let mut parts = key.split('.');
    if let (Some(_), Some(payload), Some(_)) = (parts.next(), parts.next(), parts.next()) {
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload.trim_end_matches('='))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        if claims.as_ref().and_then(|c| c["role"].as_str()) == Some("service_role") {
            return Err(SwiftWinError::invalid("that is the Supabase service_role key; SwiftWin needs the anon (public) key"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(role: &str) -> String {
        let p = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(r#"{{"role":"{role}"}}"#));
        format!("eyJhbGciOiJIUzI1NiJ9.{p}.sig")
    }

    #[test]
    fn refuses_secret_keys_and_plain_http() {
        assert!(check_public_key(&jwt("anon")).is_ok());
        assert!(check_public_key("sb_publishable_abc").is_ok());
        assert!(check_public_key(&jwt("service_role")).is_err());
        assert!(check_public_key("sb_secret_abc").is_err());
        let s = AuthSettings { supabase_url: "http://evil.example".into(), supabase_key: "k".into(), ..Default::default() }.with_defaults();
        assert!(s.validate().is_err());
        let s = AuthSettings { supabase_url: "http://127.0.0.1:9/".into(), supabase_key: "k".into(), ..Default::default() }.with_defaults();
        assert!(s.validate().is_ok());
        assert_eq!(s.supabase_url, "http://127.0.0.1:9");
        assert!(s.accounts_configured());
        assert!(!AuthSettings::default().with_defaults().accounts_configured());
    }

    #[test]
    fn built_in_ids_come_from_the_build_file() {
        let f = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../swiftwin-build.env")).unwrap_or_default();
        let url = f.lines().find_map(|l| l.trim().strip_prefix("SWIFTWIN_BUILD_SUPABASE_URL=")).map(str::trim);
        if std::env::var("SWIFTWIN_BUILD_SUPABASE_URL").is_err() {
            assert_eq!(BUILT_IN_SUPABASE_URL, url.unwrap_or(""), "build.rs bakes in swiftwin-build.env");
        }
        if !BUILT_IN_SUPABASE_KEY.is_empty() {
            check_public_key(BUILT_IN_SUPABASE_KEY).unwrap();
        }
    }
}
