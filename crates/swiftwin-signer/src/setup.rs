//! Choosing how SwiftWin signs, shared by `swiftwin signing …` and the desktop app's
//! Signing dialog. The callers load and save the [`Config`]; secrets go to the
//! [`SecretStore`] and are never returned.

use serde_json::{json, Value};
use swiftwin_core::config::{Config, SIGNING_APPLE_ID};
use swiftwin_core::{Result, SwiftWinError};

use crate::asc::{AscClient, AscKey};
use crate::{plumesign, secrets, tools, SecretStore};

/// Apple's limits for a free Apple ID, as shown to the user.
pub const FREE_LIMITS: &str = "Apple's limits: apps stop launching after 7 days (run them again from SwiftWin), 3 apps per phone, 10 new App IDs per week.";

fn team_id_ok(t: &str) -> bool {
    t.len() == 10 && t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// Check the key offline (a valid `.p8`, sane ids), then ask App Store Connect to list one
/// bundle id with it. Returns a one-line PASS message.
// UNTESTED-ON-DEVICE: real App Store Connect (fake_asc only).
pub fn check_asc(key: &AscKey, team_id: &str, base: &str) -> Result<String> {
    if key.key_id.trim().is_empty() || key.issuer_id.trim().is_empty() {
        return Err(SwiftWinError::invalid("enter the key id and the issuer id"));
    }
    if !team_id_ok(team_id) {
        return Err(SwiftWinError::invalid("the Team ID is 10 letters and digits, like ABCDE12345 (developer.apple.com > Account > Membership details)"));
    }
    key.jwt(swiftwin_core::time::unix_seconds())?;
    let ids = AscClient::with_base(key.clone(), base).list_bundle_ids(1)?;
    // A bundle id's seedId is its team's id: catches a Team ID from another account.
    if let Some(seed) = ids.first().and_then(|b| b.attr("seedId")) {
        if seed != team_id {
            return Err(SwiftWinError::invalid(format!("this key belongs to team {seed}, not {team_id}: check the Team ID")));
        }
    }
    Ok("App Store Connect accepted the key".into())
}

/// Store the key (after [`check_asc`] or the offline check) and switch to paid signing.
pub fn use_asc(cfg: &mut Config, store: &dyn SecretStore, key: &AscKey, team_id: &str) -> Result<()> {
    if !team_id_ok(team_id) {
        return Err(SwiftWinError::invalid("the Team ID is 10 letters and digits, like ABCDE12345"));
    }
    key.jwt(0)?;
    store.set(secrets::ASC_P8, &key.p8_pem)?;
    cfg.asc_key_id = Some(key.key_id.trim().to_string());
    cfg.asc_issuer_id = Some(key.issuer_id.trim().to_string());
    cfg.team_id = Some(team_id.to_string());
    cfg.signing = None;
    cfg.paid_team_hint = None;
    Ok(())
}

/// plumesign (and rcodesign, which finishes WebDriverAgent's test bundle), downloaded if missing.
pub fn ensure_apple_id_tools(progress: &dyn Fn(&str)) -> Result<plumesign::Plumesign> {
    let exe = match plumesign::find_plumesign() {
        Some(p) => p,
        None => {
            progress(&format!("Downloading plumesign {} ...", plumesign::PLUMESIGN_VERSION));
            plumesign::install_plumesign()?
        }
    };
    if tools::find_rcodesign().is_none() {
        progress(&format!("Downloading rcodesign {} ...", tools::RCODESIGN_VERSION));
        tools::install_rcodesign()?;
    }
    Ok(plumesign::Plumesign { exe })
}

/// The message when `email` has no plumesign session yet.
pub fn not_signed_in(email: &str) -> SwiftWinError {
    SwiftWinError::Auth(format!(
        "{email} is not signed in yet: sign in from SwiftWin Studio (avatar menu > Signing), or add --login to sign in here          (plumesign asks for the password and the 2FA code; the password is shown as you type it)."
    ))
}

/// Only plain e-mail characters: the address is passed to plumesign on a command line.
pub fn valid_email(email: &str) -> bool {
    let ok = |c: char| c.is_ascii_alphanumeric() || "@._+-".contains(c);
    email.len() <= 254 && email.chars().all(ok) && email.split('@').count() == 2 && !email.starts_with('@') && !email.ends_with('@')
}

/// Switch to free Apple ID signing (the plumesign session is checked by the caller).
/// Returns the bundle-id prefix new projects get.
pub fn use_apple_id(cfg: &mut Config, email: &str) -> Result<String> {
    if !valid_email(email) {
        return Err(SwiftWinError::invalid("enter your Apple ID e-mail"));
    }
    cfg.signing = Some(SIGNING_APPLE_ID.into());
    cfg.apple_id = Some(email.to_string());
    Ok(cfg.id_prefix.get_or_insert_with(plumesign::new_id_prefix).clone())
}

/// What is configured, without secrets (`swiftwin signing status`, the app's Signing dialog).
pub fn status(cfg: &Config, store: &dyn SecretStore) -> Result<Value> {
    let path = |p: Option<std::path::PathBuf>| p.map(|p| p.display().to_string());
    if cfg.uses_apple_id() {
        return Ok(json!({
            "mode": "apple-id",
            "apple_id": cfg.apple_id,
            "id_prefix": cfg.id_prefix,
            "plumesign": path(plumesign::find_plumesign()),
            "limits": FREE_LIMITS,
            "paid_team_hint": cfg.paid_team_hint,
        }));
    }
    let p8 = store.get(secrets::ASC_P8)?.is_some();
    Ok(json!({
        "mode": if p8 || cfg.asc_key_id.is_some() { "asc" } else { "none" },
        "team_id": cfg.team_id,
        "asc_key_id": cfg.asc_key_id,
        "asc_issuer_id": cfg.asc_issuer_id,
        "p8_stored": p8,
        "dev_key": store.get(secrets::DEV_KEY)?.is_some(),
        "rcodesign": path(tools::find_rcodesign()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asc_key_is_checked_against_the_api_and_stored() {
        let (key, vk) = crate::asc::tests::test_key();
        let server = crate::fake_asc::FakeAsc::start(vk, "TEAM123456");
        assert!(check_asc(&key, "ABCDE12345", &server.url()).unwrap().contains("accepted"));
        assert!(check_asc(&key, "abc", &server.url()).is_err(), "bad team id");
        let bad = AscKey { p8_pem: "not a key".into(), ..key.clone() };
        assert!(check_asc(&bad, "ABCDE12345", &server.url()).is_err());

        let store = secrets::MemoryStore::default();
        let mut cfg = Config { signing: Some(SIGNING_APPLE_ID.into()), paid_team_hint: Some("ABCDE12345".into()), ..Default::default() };
        use_asc(&mut cfg, &store, &key, "ABCDE12345").unwrap();
        assert!(!cfg.uses_apple_id() && cfg.paid_team_hint.is_none());
        assert_eq!(store.get(secrets::ASC_P8).unwrap().as_deref(), Some(key.p8_pem.as_str()));
        let s = status(&cfg, &store).unwrap();
        assert_eq!(s["mode"], "asc");
        assert_eq!(s["p8_stored"], true);
        assert!(!s.to_string().contains("PRIVATE KEY"), "status never returns the key");
    }

    #[test]
    fn apple_id_mode() {
        let mut cfg = Config::default();
        assert!(use_apple_id(&mut cfg, "a b@x.com").is_err());
        assert!(use_apple_id(&mut cfg, "x@y.com & calc").is_err());
        let prefix = use_apple_id(&mut cfg, "you@example.com").unwrap();
        assert!(cfg.uses_apple_id());
        assert_eq!(use_apple_id(&mut cfg, "you@example.com").unwrap(), prefix, "the prefix is kept");
        let s = status(&cfg, &secrets::MemoryStore::default()).unwrap();
        assert_eq!(s["mode"], "apple-id");
        assert_eq!(s["apple_id"], "you@example.com");
        assert_eq!(status(&Config::default(), &secrets::MemoryStore::default()).unwrap()["mode"], "none");
    }
}
