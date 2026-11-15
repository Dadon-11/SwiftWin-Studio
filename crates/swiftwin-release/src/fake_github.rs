//! In-process fake of the GitHub REST endpoints used by cloud release.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

#[derive(Default, Clone)]
pub struct State {
    pub secrets: HashMap<String, Vec<u8>>,
    pub files: Vec<String>,
    pub ref_sha: Option<String>,
    pub ref_created: bool,
    pub ref_updated: bool,
    pub run_polls: HashMap<String, u32>,
    pub next_conclusion: String,
    pub conclusions: HashMap<String, String>,
    n: u32,
}

pub struct FakeGitHub {
    port: u16,
    state: Arc<Mutex<State>>,
}

impl FakeGitHub {
    pub fn start() -> FakeGitHub {
        let sk = crypto_box::SecretKey::generate(&mut rand::rngs::OsRng);
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State { next_conclusion: "success".into(), ..Default::default() }));
        let st = state.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let mut it = line.split_whitespace();
                let (method, path) = (it.next().unwrap().to_string(), it.next().unwrap().to_string());
                let (mut len, mut auth) = (0usize, false);
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h == "\r\n" || h.is_empty() {
                        break;
                    }
                    let lower = h.to_ascii_lowercase();
                    if let Some(v) = lower.strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                    if lower.starts_with("authorization: bearer ghp_test") {
                        auth = true;
                    }
                }
                let mut body = vec![0; len];
                r.read_exact(&mut body).unwrap();
                let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let (code, v) = if !auth {
                    (401, json!({"message": "Bad credentials"}))
                } else {
                    route(&mut st.lock().unwrap(), &sk, &method, &path, &body)
                };
                let text = v.to_string();
                let mut s = s;
                write!(s, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len()).unwrap();
            }
        });
        FakeGitHub { port, state }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }
    pub fn secrets(&self) -> HashMap<String, Vec<u8>> {
        self.state().secrets
    }
    pub fn set_next_conclusion(&self, c: &str) {
        self.state.lock().unwrap().next_conclusion = c.into();
    }
}

fn route(st: &mut State, sk: &crypto_box::SecretKey, method: &str, path: &str, body: &Value) -> (u16, Value) {
    let p = path.strip_prefix("/repos/me/shop").unwrap_or("/bad");
    st.n += 1;
    let sha = format!("{:040x}", st.n);
    match (method, p.split('?').next().unwrap()) {
        ("GET", "/actions/secrets/public-key") => {
            (200, json!({"key_id": "K1", "key": swiftwin_core::base64_bytes::encode(sk.public_key().as_bytes())}))
        }
        ("PUT", s) if s.starts_with("/actions/secrets/") => {
            assert_eq!(body["key_id"], "K1");
            let sealed = swiftwin_core::base64_bytes::decode(body["encrypted_value"].as_str().unwrap()).unwrap();
            let plain = sk.unseal(&sealed).expect("secret decrypts with the repo key");
            st.secrets.insert(s.trim_start_matches("/actions/secrets/").into(), plain);
            (201, json!({}))
        }
        ("POST", "/git/blobs") => {
            assert_eq!(body["encoding"], "base64");
            (201, json!({"sha": sha}))
        }
        ("POST", "/git/trees") => {
            st.files = body["tree"].as_array().unwrap().iter().map(|t| t["path"].as_str().unwrap().to_string()).collect();
            (201, json!({"sha": sha}))
        }
        ("POST", "/git/commits") => {
            let parents = body["parents"].as_array().unwrap();
            assert_eq!(parents.len(), usize::from(st.ref_sha.is_some()));
            let c = st.next_conclusion.clone();
            st.conclusions.insert(sha.clone(), c);
            (201, json!({"sha": sha}))
        }
        ("GET", "/git/ref/heads/swiftwin/release") => match &st.ref_sha {
            Some(s) => (200, json!({"object": {"sha": s}})),
            None => (404, json!({"message": "Not Found"})),
        },
        ("POST", "/git/refs") => {
            st.ref_created = true;
            st.ref_sha = body["sha"].as_str().map(str::to_string);
            (201, json!({}))
        }
        ("PATCH", "/git/refs/heads/swiftwin/release") => {
            assert_eq!(body["force"], true);
            st.ref_updated = true;
            st.ref_sha = body["sha"].as_str().map(str::to_string);
            (200, json!({}))
        }
        ("GET", "/actions/runs") => {
            let head = p.split("head_sha=").nth(1).unwrap().split('&').next().unwrap().to_string();
            let n = st.run_polls.entry(head.clone()).or_insert(0);
            *n += 1;
            let n = *n;
            if n == 1 {
                return (200, json!({"workflow_runs": []}));
            }
            let (status, concl) = if n < 4 { ("in_progress", Value::Null) } else { ("completed", json!(st.conclusions[&head])) };
            (200, json!({"workflow_runs": [{"id": 77, "status": status, "conclusion": concl, "html_url": "https://github.com/me/shop/actions/runs/77"}]}))
        }
        _ => (404, json!({"message": format!("no route {method} {path}")})),
    }
}
