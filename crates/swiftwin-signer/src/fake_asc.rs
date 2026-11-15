//! In-process fake of the App Store Connect API resources SwiftWin uses.
//! Verifies the ES256 JWT on every request.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use p256::ecdsa::VerifyingKey;
use serde_json::{json, Value};

#[derive(Default, Clone)]
pub struct State {
    pub certs: usize,
    pub bundle_ids: usize,
    pub devices: usize,
    pub profiles_created: usize,
    pub profiles_deleted: usize,
    pub csr_seen: String,
    cert_list: Vec<Value>,
    bid_list: Vec<Value>,
    dev_list: Vec<Value>,
    prof_list: Vec<Value>,
}

pub struct FakeAsc {
    port: u16,
    state: Arc<Mutex<State>>,
}

impl FakeAsc {
    pub fn start(vk: VerifyingKey, team: &'static str) -> FakeAsc {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State::default()));
        let st = state.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let mut it = line.split_whitespace();
                let (method, path) = (it.next().unwrap().to_string(), it.next().unwrap().to_string());
                let (mut len, mut auth) = (0usize, String::new());
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h == "\r\n" || h.is_empty() {
                        break;
                    }
                    let (k, v) = h.split_once(':').unwrap();
                    match k.to_ascii_lowercase().as_str() {
                        "content-length" => len = v.trim().parse().unwrap(),
                        "authorization" => auth = v.trim().trim_start_matches("Bearer ").to_string(),
                        _ => {}
                    }
                }
                let mut body = vec![0; len];
                r.read_exact(&mut body).unwrap();
                let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let claims = crate::asc::tests::verify_jwt(&auth, &vk);
                assert_eq!(claims["aud"], "appstoreconnect-v1");
                let (code, resp) = route(&mut st.lock().unwrap(), &method, &path, &body, team);
                let text = resp.map(|v| v.to_string()).unwrap_or_default();
                let mut s = s;
                write!(s, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len()).unwrap();
            }
        });
        FakeAsc { port, state }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }
}

fn query(path: &str, key: &str) -> Option<String> {
    let q = path.split_once('?')?.1;
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then(|| v.replace("%20", " "))
    })
}

fn route(st: &mut State, method: &str, path: &str, body: &Value, team: &str) -> (u16, Option<Value>) {
    let base = path.split('?').next().unwrap();
    let attrs = &body["data"]["attributes"];
    match (method, base) {
        ("GET", "/v1/certificates") => (200, Some(json!({"data": st.cert_list}))),
        ("POST", "/v1/certificates") => {
            st.certs += 1;
            st.csr_seen = attrs["csrContent"].as_str().unwrap_or("").to_string();
            let c = json!({"type": "certificates", "id": format!("CERT{}", st.certs), "attributes": {
                "certificateType": "DEVELOPMENT", "serialNumber": "ABC", "expirationDate": "2027-11-03T00:00:00.000+00:00",
                "certificateContent": swiftwin_core::base64_bytes::encode(b"fake-der-certificate")}});
            st.cert_list.push(c.clone());
            (201, Some(json!({"data": c})))
        }
        ("GET", "/v1/bundleIds") => {
            let f = query(path, "filter[identifier]").unwrap_or_default();
            let v: Vec<Value> = st.bid_list.iter().filter(|b| b["attributes"]["identifier"].as_str().unwrap().starts_with(&f)).cloned().collect();
            (200, Some(json!({"data": v})))
        }
        ("POST", "/v1/bundleIds") => {
            st.bundle_ids += 1;
            let b = json!({"type": "bundleIds", "id": format!("BID{}", st.bundle_ids), "attributes": attrs});
            st.bid_list.push(b.clone());
            (201, Some(json!({"data": b})))
        }
        ("GET", "/v1/devices") => {
            let f = query(path, "filter[udid]").unwrap_or_default();
            let v: Vec<Value> = st.dev_list.iter().filter(|d| d["attributes"]["udid"] == f.as_str()).cloned().collect();
            (200, Some(json!({"data": v})))
        }
        ("POST", "/v1/devices") => {
            st.devices += 1;
            let d = json!({"type": "devices", "id": format!("DEV{}", st.devices), "attributes": attrs});
            st.dev_list.push(d.clone());
            (201, Some(json!({"data": d})))
        }
        ("GET", "/v1/profiles") => {
            let f = query(path, "filter[name]").unwrap_or_default();
            let v: Vec<Value> = st.prof_list.iter().filter(|p| p["attributes"]["name"] == f.as_str()).cloned().collect();
            (200, Some(json!({"data": v})))
        }
        ("POST", "/v1/profiles") => {
            st.profiles_created += 1;
            let rel = &body["data"]["relationships"];
            let bid = rel["bundleId"]["data"]["id"].as_str().unwrap();
            let ident = st.bid_list.iter().find(|b| b["id"] == bid).unwrap()["attributes"]["identifier"].as_str().unwrap().to_string();
            let udids: Vec<String> = rel["devices"]["data"].as_array().unwrap().iter()
                .map(|d| st.dev_list.iter().find(|x| x["id"] == d["id"]).unwrap()["attributes"]["udid"].as_str().unwrap().to_string())
                .collect();
            let refs: Vec<&str> = udids.iter().map(|s| s.as_str()).collect();
            let raw = crate::profile::tests::fake_profile(team, &ident, &refs);
            let p = json!({"type": "profiles", "id": format!("PROF{}", st.profiles_created),
                "attributes": {"name": attrs["name"], "profileState": "ACTIVE", "profileContent": swiftwin_core::base64_bytes::encode(&raw)},
                "relationships": {"devices": {"data": rel["devices"]["data"]}, "certificates": {"data": rel["certificates"]["data"]}}});
            st.prof_list.push(p.clone());
            (201, Some(json!({"data": p})))
        }
        ("DELETE", p) if p.starts_with("/v1/profiles/") => {
            let id = p.trim_start_matches("/v1/profiles/");
            st.prof_list.retain(|x| x["id"] != id);
            st.profiles_deleted += 1;
            (204, None)
        }
        _ => (404, Some(json!({"errors": [{"detail": format!("no route {method} {path}")}]}))),
    }
}
