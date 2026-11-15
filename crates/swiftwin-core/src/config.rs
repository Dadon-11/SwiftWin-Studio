//! Persistent, non-secret settings in `<data dir>/config.json`.
//! Secrets (Apple keys, passwords) never go here: they live in the Windows Credential Manager.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Bundle id of the WebDriverAgent runner as built by SwiftWin before re-signing.
pub const DEFAULT_WDA_BUNDLE_ID: &str = "com.facebook.WebDriverAgentRunner.xctrunner";
pub const WDA_PORT: u16 = 8100;
/// WebDriverAgent's MJPEG screen stream (its `MJPEG_SERVER_PORT` default).
pub const WDA_MJPEG_PORT: u16 = 9100;
/// `Config::signing` value for signing with a free Apple ID through plumesign.
pub const SIGNING_APPLE_ID: &str = "apple-id";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Bundle id of the installed (re-signed) WebDriverAgent runner.
    pub wda_bundle_id: Option<String>,
    /// Apple Developer Team ID used for signing.
    pub team_id: Option<String>,
    /// App Store Connect API key id + issuer (the private key is in the credential store).
    pub asc_key_id: Option<String>,
    pub asc_issuer_id: Option<String>,
    /// Apple ID e-mail (for display / SDK download; password is never stored here).
    pub apple_id: Option<String>,
    /// Name of the active iOS SDK under `<data dir>/sdk`.
    pub sdk: Option<String>,
    /// Path to the Swift toolchain's `bin` directory, if not on PATH.
    pub swift_bin: Option<String>,
    /// GitHub repo ("owner/name") used for cloud release builds.
    pub release_repo: Option<String>,
    /// How apps are signed: unset = App Store Connect API key (paid account),
    /// `"apple-id"` = free Apple ID through plumesign.
    pub signing: Option<String>,
    /// Bundle-id prefix for apps signed with a free Apple ID. Bundle ids are global at
    /// Apple, so `com.example.*` is usually taken; this one is random and kept forever.
    pub id_prefix: Option<String>,
    /// Free Apple ID mode only: the team plumesign signed with turned out to be a paid one
    /// (a provisioning profile valid for more than a week), so the app suggests the paid mode.
    pub paid_team_hint: Option<String>,
    /// Where the desktop app looks for updates: an https URL of a Tauri `latest.json`
    /// (docs/releasing.md). Unset = no update checks. `SWIFTWIN_UPDATE_URL` overrides it.
    pub update_url: Option<String>,
}

impl Config {
    pub fn path() -> PathBuf {
        crate::paths::data_dir().join("config.json")
    }

    pub fn load() -> Config {
        std::fs::read(Self::path()).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> crate::Result<()> {
        self.save_to(&Self::path())
    }

    /// Atomic: the app and `swiftwin mcp` both read it, and a half-written file would load as the
    /// default config (and could be saved back that way, losing the signing setup).
    fn save_to(&self, p: &std::path::Path) -> crate::Result<()> {
        crate::paths::write_atomic(p, &serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn wda_bundle_id(&self) -> String {
        self.wda_bundle_id.clone().unwrap_or_else(|| DEFAULT_WDA_BUNDLE_ID.to_string())
    }

    pub fn uses_apple_id(&self) -> bool {
        self.signing.as_deref() == Some(SIGNING_APPLE_ID)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_config_files_still_load() {
        let c: Config = serde_json::from_str(r#"{"team_id":"ABCDE12345"}"#).unwrap();
        assert_eq!(c.team_id.as_deref(), Some("ABCDE12345"));
        assert!(!c.uses_apple_id());
        let c: Config = serde_json::from_str(r#"{"signing":"apple-id","id_prefix":"dev.swiftwin.u1a2b3c4d"}"#).unwrap();
        assert!(c.uses_apple_id());
    }

    #[test]
    fn save_replaces_the_file_whole() {
        let dir = std::env::temp_dir().join(format!("swiftwin-cfg-{}", std::process::id()));
        let p = dir.join("config.json");
        let c = Config { team_id: Some("ABCDE12345".into()), ..Default::default() };
        c.save_to(&p).unwrap();
        Config { team_id: Some("ZZZZZ99999".into()), ..Default::default() }.save_to(&p).unwrap();
        let back: Config = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(back.team_id.as_deref(), Some("ZZZZZ99999"));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temp file left behind");
        std::fs::remove_dir_all(dir).ok();
    }
}
