//! App Store Connect API client (official, documented REST API).
//!
//! Auth: ES256 JWT signed with the team's API key (`AuthKey_<id>.p8`):
//! header `{alg: ES256, kid, typ: JWT}`, claims `{iss: issuer, iat, exp ≤ 20 min, aud: appstoreconnect-v1}`.
//! Resources used: certificates, bundleIds, devices, profiles.
//! Docs: https://developer.apple.com/documentation/appstoreconnectapi

use std::time::Duration;

use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use p256::pkcs8::DecodePrivateKey;
use serde_json::{json, Value};
use swiftwin_core::{base64_bytes, Result, SwiftWinError};

pub const API_BASE: &str = "https://api.appstoreconnect.apple.com";

#[derive(Clone)]
pub struct AscKey {
    pub key_id: String,
    pub issuer_id: String,
    /// Contents of the `.p8` file (PKCS#8 PEM, P-256).
    pub p8_pem: String,
}

fn b64url(data: &[u8]) -> String {
    base64_bytes::encode(data).trim_end_matches('=').replace('+', "-").replace('/', "_")
}

impl AscKey {
    pub fn jwt(&self, now: i64) -> Result<String> {
        // Tolerate a .p8 that went through Notepad (BOM, CRLF line endings).
        let pem = format!("{}\n", self.p8_pem.trim_start_matches('\u{feff}').replace("\r\n", "\n").trim());
        let sk = SigningKey::from_pkcs8_pem(&pem)
            .map_err(|e| SwiftWinError::Auth(format!("App Store Connect key is not a valid .p8 file: {e}")))?;
        let header = json!({"alg": "ES256", "kid": self.key_id, "typ": "JWT"});
        // Apple rejects lifetimes (exp - iat) over 20 minutes; iat is backdated a
        // minute so a PC clock that runs slightly fast is not "issued in the future".
        let iat = now - 60;
        let claims = json!({"iss": self.issuer_id, "iat": iat, "exp": iat + 1140, "aud": "appstoreconnect-v1"});
        let msg = format!("{}.{}", b64url(header.to_string().as_bytes()), b64url(claims.to_string().as_bytes()));
        let sig: Signature = sk.sign(msg.as_bytes());
        Ok(format!("{msg}.{}", b64url(&sig.to_bytes())))
    }
}

/// A resource returned by the API (`data` item).
#[derive(Debug, Clone, PartialEq)]
pub struct Resource {
    pub id: String,
    pub attributes: Value,
    pub relationships: Value,
}

impl Resource {
    fn from(v: &Value) -> Resource {
        Resource {
            id: v["id"].as_str().unwrap_or_default().to_string(),
            attributes: v["attributes"].clone(),
            relationships: v["relationships"].clone(),
        }
    }
    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attributes[k].as_str()
    }
}

pub struct AscClient {
    key: AscKey,
    base: String,
    agent: ureq::Agent,
}

impl AscClient {
    pub fn new(key: AscKey) -> Self {
        Self::with_base(key, API_BASE)
    }

    pub fn with_base(key: AscKey, base: &str) -> Self {
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(60)).build();
        AscClient { key, base: base.trim_end_matches('/').to_string(), agent }
    }

    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let token = self.key.jwt(swiftwin_core::time::unix_seconds())?;
        let req = self
            .agent
            .request(method, &format!("{}{}", self.base, path))
            .set("Authorization", &format!("Bearer {token}"))
            .set("Accept", "application/json");
        let res = match body {
            Some(b) => req.send_json(b),
            None => req.call(),
        };
        match res {
            Ok(r) if r.status() == 204 => Ok(Value::Null),
            Ok(r) => r.into_json().map_err(|e| SwiftWinError::Http(format!("App Store Connect: bad JSON ({e})"))),
            Err(ureq::Error::Status(code, r)) => {
                let v: Value = r.into_json().unwrap_or(Value::Null);
                let detail = v["errors"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|e| e["detail"].as_str()).collect::<Vec<_>>().join("; "))
                    .unwrap_or_default();
                if code == 401 {
                    return Err(SwiftWinError::Auth(format!(
                        "App Store Connect rejected the API key (check key id, issuer id and .p8): {detail}"
                    )));
                }
                Err(SwiftWinError::Http(format!("App Store Connect {method} {path}: HTTP {code}: {detail}")))
            }
            Err(e) => Err(SwiftWinError::Http(format!("App Store Connect unreachable: {e}"))),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<Resource>> {
        let v = self.call("GET", path, None)?;
        Ok(v["data"].as_array().map(|a| a.iter().map(Resource::from).collect()).unwrap_or_default())
    }

    fn create(&self, path: &str, body: Value) -> Result<Resource> {
        let v = self.call("POST", path, Some(body))?;
        Ok(Resource::from(&v["data"]))
    }

    // ---- certificates ----

    pub fn list_dev_certificates(&self) -> Result<Vec<Resource>> {
        self.list("/v1/certificates?filter[certificateType]=DEVELOPMENT&limit=200")
    }

    // UNTESTED-ON-DEVICE: real ASC API (exact csrContent format).
    pub fn create_dev_certificate(&self, csr_pem: &str) -> Result<Resource> {
        self.create(
            "/v1/certificates",
            json!({"data": {"type": "certificates", "attributes": {"certificateType": "DEVELOPMENT", "csrContent": csr_pem}}}),
        )
    }

    // ---- bundle ids ----

    pub fn list_bundle_ids(&self, limit: u32) -> Result<Vec<Resource>> {
        self.list(&format!("/v1/bundleIds?limit={limit}"))
    }

    pub fn find_bundle_id(&self, identifier: &str) -> Result<Option<Resource>> {
        // filter[identifier] is a prefix match, so check exactly.
        let all = self.list(&format!("/v1/bundleIds?filter[identifier]={identifier}&limit=200"))?;
        Ok(all.into_iter().find(|r| r.attr("identifier") == Some(identifier)))
    }

    pub fn ensure_bundle_id(&self, identifier: &str, name: &str) -> Result<Resource> {
        if let Some(r) = self.find_bundle_id(identifier)? {
            return Ok(r);
        }
        self.create(
            "/v1/bundleIds",
            json!({"data": {"type": "bundleIds", "attributes": {"identifier": identifier, "name": name, "platform": "IOS"}}}),
        )
    }

    // ---- devices ----

    pub fn ensure_device(&self, udid: &str, name: &str) -> Result<Resource> {
        let found = self.list(&format!("/v1/devices?filter[udid]={udid}&limit=200"))?;
        if let Some(d) = found.into_iter().find(|d| d.attr("udid").is_some_and(|u| u.eq_ignore_ascii_case(udid))) {
            return Ok(d);
        }
        self.create(
            "/v1/devices",
            json!({"data": {"type": "devices", "attributes": {"name": name, "udid": udid, "platform": "IOS"}}}),
        )
    }

    // ---- profiles ----

    /// Profiles named exactly `name`. The server-side filter may match more
    /// loosely (like `filter[identifier]`), and the caller deletes what this
    /// returns, so "SwiftWin com.example.hello2" must never match "SwiftWin com.example.hello".
    pub fn find_profiles(&self, name: &str) -> Result<Vec<Resource>> {
        let all = self.list(&format!("/v1/profiles?filter[name]={}&include=devices,certificates&limit=200", name.replace(' ', "%20")))?;
        Ok(all.into_iter().filter(|p| p.attr("name") == Some(name)).collect())
    }

    pub fn delete_profile(&self, id: &str) -> Result<()> {
        self.call("DELETE", &format!("/v1/profiles/{id}"), None).map(|_| ())
    }

    pub fn create_dev_profile(&self, name: &str, bundle_id_res: &str, cert_id: &str, device_ids: &[String]) -> Result<Resource> {
        let devices: Vec<Value> = device_ids.iter().map(|d| json!({"type": "devices", "id": d})).collect();
        self.create(
            "/v1/profiles",
            json!({"data": {
                "type": "profiles",
                "attributes": {"name": name, "profileType": "IOS_APP_DEVELOPMENT"},
                "relationships": {
                    "bundleId": {"data": {"type": "bundleIds", "id": bundle_id_res}},
                    "certificates": {"data": [{"type": "certificates", "id": cert_id}]},
                    "devices": {"data": devices}
                }
            }}),
        )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use p256::ecdsa::{signature::Verifier, VerifyingKey};
    use p256::pkcs8::EncodePrivateKey;

    pub fn test_key() -> (AscKey, VerifyingKey) {
        let sk = SigningKey::random(&mut rand::thread_rng());
        let pem = sk.to_pkcs8_pem(p256::pkcs8::LineEnding::LF).unwrap().to_string();
        (AscKey { key_id: "KEY123".into(), issuer_id: "issuer-uuid".into(), p8_pem: pem }, *sk.verifying_key())
    }

    pub fn verify_jwt(token: &str, vk: &VerifyingKey) -> Value {
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
        let dec = |s: &str| {
            let mut t = s.replace('-', "+").replace('_', "/");
            while !t.len().is_multiple_of(4) {
                t.push('=');
            }
            base64_bytes::decode(&t).unwrap()
        };
        let sig = Signature::from_slice(&dec(parts[2])).unwrap();
        vk.verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig).expect("JWT signature");
        let header: Value = serde_json::from_slice(&dec(parts[0])).unwrap();
        assert_eq!(header["alg"], "ES256");
        assert_eq!(header["kid"], "KEY123");
        serde_json::from_slice(&dec(parts[1])).unwrap()
    }

    #[test]
    fn jwt_is_valid_es256() {
        let (k, vk) = test_key();
        let claims = verify_jwt(&k.jwt(1_000).unwrap(), &vk);
        assert_eq!(claims["iss"], "issuer-uuid");
        assert_eq!(claims["aud"], "appstoreconnect-v1");
        assert_eq!(claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(), 1140);
        assert!(claims["iat"].as_i64().unwrap() <= 1_000);
        // A .p8 saved by Notepad (BOM + CRLF) still works.
        let notepad = AscKey { p8_pem: format!("\u{feff}{}", k.p8_pem.replace('\n', "\r\n")), ..k.clone() };
        verify_jwt(&notepad.jwt(1_000).unwrap(), &vk);
        let bad = AscKey { p8_pem: "nope".into(), ..k };
        assert!(matches!(bad.jwt(0), Err(SwiftWinError::Auth(_))));
    }
}
