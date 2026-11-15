//! Signing for SwiftWin: development certificate, App ID, device registration,
//! provisioning profile, and code signing – all from Windows.
//!
//! Account access uses the official **App Store Connect API** with a team API
//! key (Users and Access > Integrations > App Store Connect API). This needs a
//! paid Apple Developer Program membership; see docs/open-questions.md for the
//! free-account situation. The `.p8` key and the generated signing private key
//! live in the Windows Credential Manager, never on disk in the repo.

pub mod asc;
pub mod codesign;
pub mod csr;
pub mod der;
pub mod plumesign;
pub mod profile;
pub mod secrets;
pub mod setup;
pub mod tools;
pub mod wda;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use swiftwin_core::config::Config;
use swiftwin_core::{Result, SwiftWinError};

pub use asc::{AscClient, AscKey};
pub use codesign::{CodeSigner, MockSigner, Rcodesign, SignRequest};
pub use profile::Profile;
pub use secrets::SecretStore;

/// The development signing identity (certificate + private key).
#[derive(Clone)]
pub struct Identity {
    pub cert_id: String,
    pub cert_pem: String,
    pub key_pem: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CertRecord {
    id: String,
    serial: String,
    expiration: String,
    pem: String,
}

/// Talks to App Store Connect and signs bundles.
pub struct SigningService {
    pub asc: AscClient,
    store: Arc<dyn SecretStore>,
    dir: PathBuf,
}

impl SigningService {
    pub fn new(asc: AscClient, store: Arc<dyn SecretStore>, dir: PathBuf) -> Self {
        SigningService { asc, store, dir }
    }

    /// Build from `config.json` + the API key stored in the credential store.
    pub fn from_config(cfg: &Config, store: Arc<dyn SecretStore>) -> Result<Self> {
        let (Some(key_id), Some(issuer_id)) = (cfg.asc_key_id.clone(), cfg.asc_issuer_id.clone()) else {
            return Err(SwiftWinError::Auth(
                "signing is not set up: run `swiftwin signing setup --key-id <ID> --issuer-id <UUID> --p8 AuthKey_<ID>.p8 --team-id <TEAMID>`".into(),
            ));
        };
        let p8_pem = store.get(secrets::ASC_P8)?.ok_or_else(|| {
            SwiftWinError::Auth("the App Store Connect .p8 key is missing from the credential store; run `swiftwin signing setup` again".into())
        })?;
        Ok(Self::new(AscClient::new(AscKey { key_id, issuer_id, p8_pem }), store, swiftwin_core::paths::signing_dir()))
    }

    fn cert_file(&self) -> PathBuf {
        self.dir.join("certificate.json")
    }

    /// Reuse the stored development certificate if Apple still knows it, else create one.
    // UNTESTED-ON-DEVICE: against the real App Store Connect API.
    pub fn ensure_identity(&self) -> Result<Identity> {
        let record: Option<CertRecord> =
            std::fs::read(self.cert_file()).ok().and_then(|d| serde_json::from_slice(&d).ok());
        let key = self.store.get(secrets::DEV_KEY)?;
        if let (Some(rec), Some(key_pem)) = (record, key) {
            let live = self.asc.list_dev_certificates()?;
            if live.iter().any(|c| c.id == rec.id) {
                return Ok(Identity { cert_id: rec.id, cert_pem: rec.pem, key_pem });
            }
        }
        let kp = csr::KeyPair::generate()?;
        let csr = kp.csr_pem("SwiftWin Development", "US")?;
        let cert = self.asc.create_dev_certificate(&csr).map_err(|e| match e {
            // Apple's wording for the limit: "You already have a current Development certificate or a pending certificate request."
            SwiftWinError::Http(m)
                if m.contains("409") || m.to_lowercase().contains("maximum") || m.contains("already have a current") =>
            {
                SwiftWinError::Auth(format!(
                    "Apple refused a new development certificate ({m}). Revoke an old one at developer.apple.com > Certificates and retry"
                ))
            }
            e => e,
        })?;
        let der_b64 = cert.attr("certificateContent").ok_or_else(|| SwiftWinError::protocol("certificate has no content"))?;
        let der = swiftwin_core::base64_bytes::decode(der_b64).ok_or_else(|| SwiftWinError::protocol("certificate content is not base64"))?;
        let rec = CertRecord {
            id: cert.id.clone(),
            serial: cert.attr("serialNumber").unwrap_or_default().to_string(),
            expiration: cert.attr("expirationDate").unwrap_or_default().to_string(),
            pem: der::pem("CERTIFICATE", &der),
        };
        let key_pem = kp.to_pkcs8_pem()?;
        self.store.set(secrets::DEV_KEY, &key_pem)?;
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.cert_file(), serde_json::to_vec_pretty(&rec)?)?;
        Ok(Identity { cert_id: rec.id, cert_pem: rec.pem, key_pem })
    }

    /// App ID + device + development profile covering `bundle_id` on `udid`.
    // UNTESTED-ON-DEVICE: against the real App Store Connect API.
    pub fn ensure_profile(&self, id: &Identity, bundle_id: &str, app_name: &str, udid: &str, device_name: &str) -> Result<Profile> {
        let bid = self.asc.ensure_bundle_id(bundle_id, &format!("SwiftWin {app_name}"))?;
        let dev = self.asc.ensure_device(udid, device_name)?;
        let name = format!("SwiftWin {bundle_id}");
        let has = |rel: &serde_json::Value, key: &str, want: &str| {
            rel[key]["data"].as_array().is_some_and(|a| a.iter().any(|x| x["id"].as_str() == Some(want)))
        };
        let existing = self.asc.find_profiles(&name)?;
        let reusable = existing.iter().find(|p| {
            p.attr("profileState") == Some("ACTIVE")
                && has(&p.relationships, "devices", &dev.id)
                && has(&p.relationships, "certificates", &id.cert_id)
                && p.attr("profileContent").is_some()
        });
        let res = match reusable {
            Some(p) => p.clone(),
            None => {
                for p in &existing {
                    self.asc.delete_profile(&p.id)?;
                }
                self.asc.create_dev_profile(&name, &bid.id, &id.cert_id, std::slice::from_ref(&dev.id))?
            }
        };
        let content = res.attr("profileContent").ok_or_else(|| SwiftWinError::protocol("profile has no content"))?;
        let raw = swiftwin_core::base64_bytes::decode(content).ok_or_else(|| SwiftWinError::protocol("profile content is not base64"))?;
        let dir = self.dir.join("profiles");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(format!("{bundle_id}.mobileprovision")), &raw)?;
        Profile::parse(&raw)
    }

    /// Full pipeline for one bundle: identity → profile → sign.
    pub fn sign_app(&self, app: &Path, udid: &str, device_name: &str, signer: &dyn CodeSigner) -> Result<Profile> {
        let (bundle_id, name) = read_bundle(app)?;
        let identity = self.ensure_identity()?;
        let profile = self.ensure_profile(&identity, &bundle_id, &name, udid, device_name)?;
        sign_with(app, &bundle_id, &identity, &profile, signer)?;
        Ok(profile)
    }
}

pub fn read_bundle(app: &Path) -> Result<(String, String)> {
    let d: plist::Dictionary = plist::from_file(app.join("Info.plist"))
        .map_err(|e| SwiftWinError::invalid(format!("{}: {e}", app.join("Info.plist").display())))?;
    let id = d.get("CFBundleIdentifier").and_then(|v| v.as_string()).ok_or_else(|| SwiftWinError::invalid("no CFBundleIdentifier"))?;
    let name = d.get("CFBundleName").and_then(|v| v.as_string()).unwrap_or(id);
    Ok((id.to_string(), name.to_string()))
}

pub fn sign_with(app: &Path, bundle_id: &str, identity: &Identity, profile: &Profile, signer: &dyn CodeSigner) -> Result<()> {
    if !profile.covers(bundle_id) {
        return Err(SwiftWinError::invalid(format!("profile {} does not cover {bundle_id}", profile.name)));
    }
    let ent = profile.entitlements_for(bundle_id);
    signer.sign(&SignRequest {
        app,
        cert_pem: &identity.cert_pem,
        key_pem: &identity.key_pem,
        entitlements: &ent,
        profile: &profile.raw,
    })
}

/// Sign with a throwaway identity and fake profile (for `--mock`): exercises
/// the same bundle layout without any Apple account.
pub fn mock_sign(app: &Path, udid: &str) -> Result<Profile> {
    let (bundle_id, _) = read_bundle(app)?;
    let mut ent = plist::Dictionary::new();
    ent.insert("application-identifier".into(), format!("MOCKTEAM01.{bundle_id}").into());
    let mut d = plist::Dictionary::new();
    d.insert("Name".into(), format!("SwiftWin {bundle_id} (mock)").into());
    d.insert("UUID".into(), "00000000-0000-0000-0000-000000000000".into());
    d.insert("TeamIdentifier".into(), plist::Value::Array(vec!["MOCKTEAM01".into()]));
    d.insert("Entitlements".into(), plist::Value::Dictionary(ent));
    d.insert("ProvisionedDevices".into(), plist::Value::Array(vec![udid.into()]));
    let raw = profile::to_xml(&d)?;
    let profile = Profile::parse(&raw)?;
    let identity = Identity {
        cert_id: "MOCK".into(),
        cert_pem: "-----BEGIN CERTIFICATE-----\nTU9DSw==\n-----END CERTIFICATE-----\n".into(),
        key_pem: "-----BEGIN PRIVATE KEY-----\nTU9DSw==\n-----END PRIVATE KEY-----\n".into(),
    };
    sign_with(app, &bundle_id, &identity, &profile, &MockSigner)?;
    Ok(profile)
}

#[cfg(test)]
mod fake_asc;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;

    fn app_dir(tag: &str, id: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("swiftwin-sign-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let app = d.join("Hello.app");
        std::fs::create_dir_all(&app).unwrap();
        let mut info = plist::Dictionary::new();
        info.insert("CFBundleIdentifier".into(), id.into());
        info.insert("CFBundleName".into(), "Hello".into());
        plist::Value::Dictionary(info).to_file_xml(app.join("Info.plist")).unwrap();
        app
    }

    #[test]
    fn full_signing_flow_against_fake_asc() {
        let (key, vk) = asc::tests::test_key();
        let server = fake_asc::FakeAsc::start(vk, "TEAM123456");
        let store = Arc::new(MemoryStore::default());
        let app = app_dir("flow", "com.example.hello");
        let dir = app.parent().unwrap().join("signing");
        let svc = SigningService::new(AscClient::with_base(key, &server.url()), store.clone(), dir.clone());
        let udid = "00008110-000A1B2C3D4E5F60";
        let profile = svc.sign_app(&app, udid, "Test iPhone", &MockSigner).unwrap();
        assert_eq!(profile.team_id, "TEAM123456");
        assert!(profile.includes_device(udid));
        assert!(app.join("embedded.mobileprovision").exists());
        let ent: plist::Dictionary = plist::from_file(app.join("_CodeSignature/entitlements.plist")).unwrap();
        assert_eq!(ent["application-identifier"].as_string(), Some("TEAM123456.com.example.hello"));
        assert!(store.get(secrets::DEV_KEY).unwrap().unwrap().contains("BEGIN PRIVATE KEY"));
        assert!(dir.join("profiles/com.example.hello.mobileprovision").exists());

        // Second run reuses certificate, bundle id, device and profile.
        svc.sign_app(&app, udid, "Test iPhone", &MockSigner).unwrap();
        let s = server.state();
        assert_eq!(s.certs, 1);
        assert_eq!(s.bundle_ids, 1);
        assert_eq!(s.devices, 1);
        assert_eq!(s.profiles_created, 1);
        assert!(s.csr_seen.starts_with("-----BEGIN CERTIFICATE REQUEST-----"));

        // A new device forces a new profile (old one deleted).
        svc.sign_app(&app, "00008120-0000111122223333", "Other", &MockSigner).unwrap();
        let s = server.state();
        assert_eq!(s.profiles_created, 2);
        assert_eq!(s.profiles_deleted, 1);
        std::fs::remove_dir_all(app.parent().unwrap()).ok();
    }

    #[test]
    fn mock_sign_and_errors() {
        let app = app_dir("mock", "com.example.mock");
        let p = mock_sign(&app, "UDID").unwrap();
        assert_eq!(p.team_id, "MOCKTEAM01");
        assert!(app.join("_CodeSignature/CodeResources").exists());
        let cfg = Config::default();
        let e = SigningService::from_config(&cfg, Arc::new(MemoryStore::default())).err().unwrap().to_string();
        assert!(e.contains("swiftwin signing setup"));
        std::fs::remove_dir_all(app.parent().unwrap()).ok();
    }
}
