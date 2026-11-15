//! `.mobileprovision` parsing and entitlements generation.
//!
//! A provisioning profile is a CMS (PKCS#7) SignedData blob whose content is
//! an XML plist. We locate the plist inside the DER bytes (it is stored as an
//! OCTET STRING, unencrypted) – the same approach `security cms -D` users and
//! many tools rely on.

use plist::{Dictionary, Value};
use swiftwin_core::{Result, SwiftWinError};

#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub uuid: String,
    pub team_id: String,
    pub app_id_name: String,
    /// "TEAMID.com.example.app" or "TEAMID.*"
    pub application_identifier: String,
    pub expiration: String,
    /// Days from `CreationDate` to `ExpirationDate`: 7 for a free Apple ID's team, a year when paid.
    pub valid_days: Option<u64>,
    pub devices: Vec<String>,
    pub entitlements: Dictionary,
    /// DER certificates allowed to sign with this profile (`DeveloperCertificates`).
    pub certificates: Vec<Vec<u8>>,
    pub raw: Vec<u8>,
}

impl Profile {
    pub fn parse(data: &[u8]) -> Result<Profile> {
        let start = find(data, b"<?xml").ok_or_else(|| SwiftWinError::invalid("not a provisioning profile (no plist)"))?;
        let end = find(&data[start..], b"</plist>").ok_or_else(|| SwiftWinError::invalid("truncated provisioning profile"))?
            + start
            + b"</plist>".len();
        let d: Dictionary =
            plist::from_bytes(&data[start..end]).map_err(|e| SwiftWinError::invalid(format!("profile plist: {e}")))?;
        let s = |k: &str| d.get(k).and_then(|v| v.as_string()).unwrap_or_default().to_string();
        let ent = d.get("Entitlements").and_then(|v| v.as_dictionary()).cloned().unwrap_or_default();
        let team_id = d
            .get("TeamIdentifier")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_string())
            .unwrap_or_default()
            .to_string();
        let expiration = match d.get("ExpirationDate") {
            Some(Value::Date(dt)) => dt.to_xml_format(),
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        let date = |k: &str| d.get(k).and_then(|v| v.as_date()).map(std::time::SystemTime::from);
        let valid_days = match (date("CreationDate"), date("ExpirationDate")) {
            (Some(c), Some(e)) => e.duration_since(c).ok().map(|d| d.as_secs() / 86_400),
            _ => None,
        };
        Ok(Profile {
            name: s("Name"),
            uuid: s("UUID"),
            team_id,
            app_id_name: s("AppIDName"),
            application_identifier: ent.get("application-identifier").and_then(|v| v.as_string()).unwrap_or_default().to_string(),
            expiration,
            valid_days,
            devices: d
                .get("ProvisionedDevices")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_string().map(str::to_string)).collect())
                .unwrap_or_default(),
            entitlements: ent,
            certificates: d
                .get("DeveloperCertificates")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_data().map(<[u8]>::to_vec)).collect())
                .unwrap_or_default(),
            raw: data.to_vec(),
        })
    }

    /// Signed by a paid team? Free (Personal Team) profiles last 7 days; paid ones a year.
    // UNTESTED-ON-DEVICE: only free-account profiles have been seen (fake profiles in tests).
    pub fn is_paid_team(&self) -> bool {
        self.valid_days.is_some_and(|d| d > 8)
    }

    pub fn includes_device(&self, udid: &str) -> bool {
        let n = |s: &str| s.replace('-', "").to_ascii_lowercase();
        self.devices.iter().any(|d| n(d) == n(udid))
    }

    /// Does this profile cover `bundle_id` (exactly or via wildcard)?
    pub fn covers(&self, bundle_id: &str) -> bool {
        let Some(app) = self.application_identifier.strip_prefix(&format!("{}.", self.team_id)) else { return false };
        app == bundle_id || app == "*" || app.strip_suffix('*').is_some_and(|p| bundle_id.starts_with(p))
    }

    /// Entitlements to sign `bundle_id` with: the profile's, made concrete.
    pub fn entitlements_for(&self, bundle_id: &str) -> Dictionary {
        let mut e = self.entitlements.clone();
        let app_id = format!("{}.{bundle_id}", self.team_id);
        e.insert("application-identifier".into(), Value::String(app_id));
        e.insert("com.apple.developer.team-identifier".into(), Value::String(self.team_id.clone()));
        if !e.contains_key("get-task-allow") {
            e.insert("get-task-allow".into(), Value::Boolean(true));
        }
        e.insert(
            "keychain-access-groups".into(),
            Value::Array(vec![Value::String(format!("{}.{bundle_id}", self.team_id))]),
        );
        e
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

pub fn to_xml(d: &Dictionary) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    Value::Dictionary(d.clone()).to_writer_xml(&mut out).map_err(|e| SwiftWinError::other(e.to_string()))?;
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fake profile: some DER-ish bytes around a real-shaped plist.
    pub fn fake_profile(team: &str, app: &str, devices: &[&str]) -> Vec<u8> {
        let mut ent = Dictionary::new();
        ent.insert("application-identifier".into(), format!("{team}.{app}").into());
        ent.insert("get-task-allow".into(), true.into());
        let mut d = Dictionary::new();
        d.insert("Name".into(), format!("SwiftWin {app}").into());
        d.insert("UUID".into(), "11111111-2222-3333-4444-555555555555".into());
        d.insert("AppIDName".into(), "SwiftWin".into());
        d.insert("TeamIdentifier".into(), Value::Array(vec![team.into()]));
        d.insert("Entitlements".into(), Value::Dictionary(ent));
        d.insert("ProvisionedDevices".into(), Value::Array(devices.iter().map(|x| Value::String(x.to_string())).collect()));
        let mut out = vec![0x30, 0x80, 0x06, 0x09, 0x2a, 0x86, 0x48];
        Value::Dictionary(d).to_writer_xml(&mut out).unwrap();
        out.extend_from_slice(&[0xa0, 0x82, 0x01, 0x00, 0xde, 0xad]);
        out
    }

    #[test]
    fn profile_lifetime_tells_free_from_paid() {
        use std::time::{Duration, SystemTime};
        let with_days = |days: u64| {
            let raw = fake_profile("ABCDE12345", "x", &[]);
            let mut d: Dictionary = plist::from_bytes(&raw[7..raw.len() - 6]).unwrap();
            let now = SystemTime::now();
            d.insert("CreationDate".into(), Value::Date(now.into()));
            d.insert("ExpirationDate".into(), Value::Date((now + Duration::from_secs(days * 86_400)).into()));
            let mut out = Vec::new();
            Value::Dictionary(d).to_writer_xml(&mut out).unwrap();
            Profile::parse(&out).unwrap()
        };
        assert!(!with_days(7).is_paid_team());
        assert!(with_days(365).is_paid_team());
        assert!(!Profile::parse(&fake_profile("A", "x", &[])).unwrap().is_paid_team(), "no dates: assume free");
    }

    #[test]
    fn parses_and_builds_entitlements() {
        let p = Profile::parse(&fake_profile("ABCDE12345", "com.example.hello", &["00008110-000A1B2C3D4E5F60"])).unwrap();
        assert_eq!(p.team_id, "ABCDE12345");
        assert!(p.covers("com.example.hello"));
        assert!(!p.covers("com.example.other"));
        assert!(p.includes_device("00008110000a1b2c3d4e5f60"));
        let e = p.entitlements_for("com.example.hello");
        assert_eq!(e["application-identifier"].as_string(), Some("ABCDE12345.com.example.hello"));
        assert_eq!(e["com.apple.developer.team-identifier"].as_string(), Some("ABCDE12345"));
        let w = Profile::parse(&fake_profile("T", "*", &[])).unwrap();
        assert!(w.covers("any.thing"));
        assert!(Profile::parse(b"garbage").is_err());
        assert!(p.certificates.is_empty());
    }

    #[test]
    fn reads_developer_certificates() {
        let mut d = Dictionary::new();
        d.insert("TeamIdentifier".into(), Value::Array(vec!["T".into()]));
        d.insert("DeveloperCertificates".into(), Value::Array(vec![Value::Data(vec![0x30, 0x82, 1]), Value::Data(vec![0x30, 2])]));
        let mut raw = Vec::new();
        Value::Dictionary(d).to_writer_xml(&mut raw).unwrap();
        assert_eq!(Profile::parse(&raw).unwrap().certificates, [vec![0x30, 0x82, 1], vec![0x30, 2]]);
    }
}
