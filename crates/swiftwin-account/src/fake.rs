//! In-process fakes of every service this crate talks to, on one loopback port:
//! Supabase Auth at `/auth/v1`, GitHub's device flow at `/gh/login/...`, and the GitHub REST
//! API at `/gh/api`. Used by the tests, `swiftwin --mock`, and the UI tests.
//!
//! Fixed values: anon key `fake-anon-key`; email codes `123456` (sign-up) and `654321`
//! (password reset); GitHub device code `FAKE-CODE` (approved on the second poll); GitHub
//! token `gho_fake_token` for user `octo`. A seeded account: `demo@swiftwin.dev` / `swiftwin123`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::settings::AuthSettings;
use crate::upload::blob_sha;

pub const ANON_KEY: &str = "fake-anon-key";
pub const SIGNUP_CODE: &str = "123456";
pub const RESET_CODE: &str = "654321";
pub const GH_TOKEN: &str = "gho_fake_token";
pub const GH_LOGIN: &str = "octo";
pub const DEMO_EMAIL: &str = "demo@swiftwin.dev";
pub const DEMO_PASSWORD: &str = "swiftwin123";

#[derive(Clone, Debug)]
pub struct FakeUser {
    pub id: String,
    pub email: String,
    pub password: String,
    pub confirmed: bool,
    pub provider: String,
    pub plan: Option<String>,
    /// `user_metadata.display_name`.
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Repo {
    pub private: bool,
    pub head: Option<String>,
    pub commits: usize,
}

#[derive(Default)]
pub struct State {
    pub users: HashMap<String, FakeUser>,
    /// refresh token → email (rotated on use).
    pub refresh: HashMap<String, String>,
    /// access token → email.
    pub access: HashMap<String, String>,
    /// This fake's own address (for the profile pictures it serves).
    pub base: String,
    /// Tokens that were rotated away; reusing one revokes the user's sessions (like Supabase).
    pub used_refresh: HashMap<String, String>,
    /// PKCE auth code → (email, challenge).
    pub auth_codes: HashMap<String, (String, String)>,
    pub logouts: usize,
    /// Pretend the owner hasn't installed `delete_my_account` yet (PostgREST answers PGRST202).
    pub no_delete_fn: bool,
    /// The owner switched email + password sign-in off (`/settings` says so).
    pub email_off: bool,
    /// The last email "sent" to each address: its link (Supabase's `{{ .ConfirmationURL }}`).
    pub mail: HashMap<String, String>,
    /// Email-link token → (email, kind, redirect_to, PKCE challenge).
    pub link_tokens: HashMap<String, (String, String, String, String)>,
    pub last_link: Option<String>,
    pub device_polls: u32,
    pub device_deny: bool,
    pub repos: HashMap<String, Repo>,
    pub blobs: HashMap<String, Vec<u8>>,
    pub blob_posts: usize,
    /// tree sha → [(path, blob sha)]
    pub trees: HashMap<String, Vec<(String, String)>>,
    /// commit sha → (tree, parents)
    pub commits: HashMap<String, (String, Vec<String>)>,
    pub requests: Vec<String>,
    n: u64,
}

impl State {
    fn next(&mut self, tag: &str) -> String {
        self.n += 1;
        let mut h = Sha256::new();
        h.update(format!("{tag}{}", self.n));
        h.finalize().iter().take(20).map(|b| format!("{b:02x}")).collect()
    }

    fn session(&mut self, email: &str) -> Value {
        let rt = self.next("rt");
        self.refresh.insert(rt.clone(), email.into());
        let at = format!("at-{}", self.next("at"));
        self.access.insert(at.clone(), email.into());
        json!({
            "access_token": at, "token_type": "bearer", "expires_in": 3600,
            "refresh_token": rt,
            "user": self.user_json(email)
        })
    }

    fn user_json(&self, email: &str) -> Value {
        let u = &self.users[email];
        let oauth = u.provider != "email";
        json!({"id": u.id, "email": u.email, "created_at": "2026-11-01T09:30:00Z",
               "email_confirmed_at": if u.confirmed { json!("2026-11-01T09:31:00Z") } else { Value::Null },
               "app_metadata": {"provider": u.provider, "plan": u.plan},
               "user_metadata": {"display_name": u.name, "full_name": if oauth { json!("Octo Cat") } else { Value::Null },
                                 "avatar_url": if oauth { json!(format!("{}/avatar.svg", self.base)) } else { Value::Null }}})
    }

    fn files_at(&self, commit: &str) -> HashMap<String, Vec<u8>> {
        let tree = &self.commits[commit].0;
        self.trees[tree].iter().map(|(p, s)| (p.clone(), self.blobs[s].clone())).collect()
    }
}

pub struct FakeCloud {
    port: u16,
    pub state: Arc<Mutex<State>>,
}

impl FakeCloud {
    pub fn start() -> FakeCloud {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind fake cloud");
        let port = l.local_addr().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        // `SWIFTWIN_FAKE_EMAIL_OFF=1`: try the Google/GitHub-only sign-in screen with `--mock`.
        let mut st = State { base: base.clone(), email_off: std::env::var_os("SWIFTWIN_FAKE_EMAIL_OFF").is_some(), ..Default::default() };
        st.users.insert(
            DEMO_EMAIL.into(),
            FakeUser { id: "u-demo".into(), email: DEMO_EMAIL.into(), password: DEMO_PASSWORD.into(), confirmed: true, provider: "email".into(), plan: None, name: None },
        );
        let state = Arc::new(Mutex::new(st));
        let st = state.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let (st, base) = (st.clone(), base.clone());
                std::thread::spawn(move || {
                    let _ = serve_one(s, &st, &base);
                });
            }
        });
        FakeCloud { port, state }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn settings(&self) -> AuthSettings {
        AuthSettings {
            supabase_url: self.url(),
            supabase_key: ANON_KEY.into(),
            github_client_id: "fake-client-id".into(),
            github_api: format!("{}/gh/api", self.url()),
            github_web: format!("{}/gh", self.url()),
        }
    }

    /// The link in the last email sent to `email` (what the person would click).
    pub fn email_link(&self, email: &str) -> Option<String> {
        self.state.lock().unwrap().mail.get(email).cloned()
    }

    /// The files on a repo's default branch.
    pub fn repo_files(&self, full: &str) -> HashMap<String, Vec<u8>> {
        let st = self.state.lock().unwrap();
        let head = st.repos[full].head.clone().unwrap();
        st.files_at(&head)
    }
}

/// A browser stand-in for `--mock`: "visits" the URL; the fake's /authorize redirects straight
/// back to the app's loopback address (ureq follows the redirect).
pub fn fake_browser() -> crate::Opener {
    Arc::new(|u: &str| {
        crate::account::check_openable(u)?;
        let u = u.to_string();
        std::thread::spawn(move || {
            let _ = ureq::get(&u).call();
        });
        Ok(())
    })
}

fn serve_one(s: std::net::TcpStream, st: &Mutex<State>, base: &str) -> std::io::Result<()> {
    let mut r = BufReader::new(s.try_clone()?);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut it = line.split_whitespace();
    let (method, target) = (it.next().unwrap_or("").to_string(), it.next().unwrap_or("/").to_string());
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h == "\r\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = vec![0; len];
    r.read_exact(&mut body)?;
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let q: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
    let json_body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let form: HashMap<String, String> = url::form_urlencoded::parse(&body).into_owned().collect();
    if path == "/avatar.svg" {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 96"><defs><linearGradient id="g" x2="1" y2="1"><stop stop-color="#5b8cff"/><stop offset="1" stop-color="#a35bff"/></linearGradient></defs><rect width="96" height="96" fill="url(#g)"/><circle cx="48" cy="38" r="17" fill="#fff" opacity=".92"/><path d="M16 92c4-20 18-30 32-30s28 10 32 30" fill="#fff" opacity=".92"/></svg>"##;
        let mut s = s;
        return write!(s, "HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{svg}", svg.len());
    }
    let mut st = st.lock().unwrap();
    st.requests.push(format!("{method} {path}"));
    let (code, v, location) = if let Some(p) = path.strip_prefix("/auth/v1") {
        // Like Supabase, /authorize and email links (GET /verify) are plain browser links: no apikey.
        if p != "/authorize" && !(p == "/verify" && method == "GET") && headers.get("apikey").map(String::as_str) != Some(ANON_KEY) {
            (401, json!({"message": "Invalid API key"}), None)
        } else {
            auth(&mut st, &method, p, &q, &json_body, headers.get("authorization"), base)
        }
    } else if path == "/rest/v1/rpc/delete_my_account" && method == "POST" {
        delete_my_account(&mut st, headers.get("apikey"), headers.get("authorization"))
    } else if let Some(p) = path.strip_prefix("/gh/login") {
        let (c, v) = device(&mut st, p, &form, base);
        (c, v, None)
    } else if let Some(p) = path.strip_prefix("/gh/api") {
        let (c, v) = if headers.get("authorization").map(String::as_str) != Some(&format!("Bearer {GH_TOKEN}")) {
            (401, json!({"message": "Bad credentials"}))
        } else {
            api(&mut st, &method, p, &q, &json_body)
        };
        (c, v, None)
    } else {
        (404, json!({"message": "not found"}), None)
    };
    drop(st);
    let text = v.to_string();
    let mut s = s;
    let loc = location.map(|l| format!("Location: {l}\r\n")).unwrap_or_default();
    write!(s, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\n{loc}Content-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len())
}

fn err(code: u16, ec: &str, msg: &str) -> (u16, Value, Option<String>) {
    (code, json!({"code": code, "error_code": ec, "msg": msg}), None)
}

/// Remember an email with a link (PKCE when the app sent a challenge and redirect).
fn send_mail(st: &mut State, base: &str, email: &str, kind: &str, q: &HashMap<String, String>, b: &Value) {
    let token = st.next("mail");
    let redirect = q.get("redirect_to").cloned().unwrap_or_default();
    let challenge = b["code_challenge"].as_str().unwrap_or("").to_string();
    st.link_tokens.insert(token.clone(), (email.into(), kind.into(), redirect.clone(), challenge));
    let link = format!("{base}/auth/v1/verify?token=pkce_{token}&type={kind}&redirect_to={}", url::form_urlencoded::byte_serialize(redirect.as_bytes()).collect::<String>());
    st.last_link = Some(link.clone());
    st.mail.insert(email.into(), link);
}

fn auth(st: &mut State, method: &str, p: &str, q: &HashMap<String, String>, b: &Value, bearer: Option<&String>, base: &str) -> (u16, Value, Option<String>) {
    let s = |k: &str| b[k].as_str().unwrap_or("").to_string();
    match (method, p) {
        ("POST", "/signup") => {
            let email = s("email");
            if s("password").len() < 6 {
                return err(422, "weak_password", "Password should be at least 6 characters.");
            }
            if !st.users.contains_key(&email) {
                let id = format!("u-{}", st.next("u"));
                st.users.insert(email.clone(), FakeUser { id, email: email.clone(), password: s("password"), confirmed: false, provider: "email".into(), plan: None, name: None });
            }
            if !st.users[&email].confirmed {
                send_mail(st, base, &email, "signup", q, b);
            }
            // Like Supabase: the same answer whether or not the address was taken.
            (200, json!({"id": "x", "email": email, "identities": []}), None)
        }
        ("GET", "/settings") => (200, json!({"external": {"email": !st.email_off, "google": true, "github": true}, "disable_signup": false}), None),
        ("POST", "/resend") => (200, json!({}), None),
        ("POST", "/recover") => {
            let email = s("email");
            if st.users.contains_key(&email) {
                send_mail(st, base, &email, "recovery", q, b);
            }
            (200, json!({}), None)
        }
        ("GET", "/verify") => {
            // The email link: confirm, then back to the app with a PKCE auth code.
            let token = q.get("token").map(|t| t.trim_start_matches("pkce_").to_string()).unwrap_or_default();
            let Some((email, _kind, redirect, challenge)) = st.link_tokens.remove(&token) else {
                return (303, json!({}), Some(format!("{}?error=access_denied&error_code=otp_expired&error_description=Email+link+is+invalid+or+has+expired", q.get("redirect_to").cloned().unwrap_or_default())));
            };
            st.users.get_mut(&email).unwrap().confirmed = true;
            let code = st.next("code");
            st.auth_codes.insert(code.clone(), (email, challenge));
            (303, json!({}), Some(format!("{redirect}?code={code}")))
        }
        ("POST", "/verify") => {
            let email = s("email");
            let want = if s("type") == "recovery" { RESET_CODE } else { SIGNUP_CODE };
            if s("token") != want || !st.users.contains_key(&email) {
                return err(403, "otp_expired", "Token has expired or is invalid");
            }
            st.users.get_mut(&email).unwrap().confirmed = true;
            (200, st.session(&email), None)
        }
        ("POST", "/token") => match q.get("grant_type").map(String::as_str) {
            Some("password") => {
                let email = s("email");
                match st.users.get(&email) {
                    Some(u) if u.password == s("password") && u.confirmed => (200, st.session(&email), None),
                    Some(u) if u.password == s("password") => err(400, "email_not_confirmed", "Email not confirmed"),
                    _ => err(400, "invalid_credentials", "Invalid login credentials"),
                }
            }
            Some("refresh_token") => {
                let rt = s("refresh_token");
                if let Some(email) = st.refresh.remove(&rt) {
                    st.used_refresh.insert(rt, email.clone());
                    (200, st.session(&email), None)
                } else if let Some(email) = st.used_refresh.get(&rt).cloned() {
                    st.refresh.retain(|_, e| *e != email); // reuse detected: revoke everything
                    err(400, "refresh_token_already_used", "Invalid Refresh Token: Already Used")
                } else {
                    err(400, "refresh_token_not_found", "Invalid Refresh Token: Refresh Token Not Found")
                }
            }
            Some("pkce") => {
                let Some((email, challenge)) = st.auth_codes.remove(&s("auth_code")) else {
                    return err(404, "flow_state_not_found", "invalid flow state, no valid flow state found");
                };
                if crate::pkce::challenge(&s("code_verifier")) != challenge {
                    return err(400, "bad_code_verifier", "code challenge does not match previously saved code verifier");
                }
                (200, st.session(&email), None)
            }
            _ => err(400, "unsupported_grant_type", "bad grant"),
        },
        ("GET", "/authorize") => {
            let provider = q.get("provider").cloned().unwrap_or_default();
            let redirect = q.get("redirect_to").cloned().unwrap_or_default();
            if !["google", "github"].contains(&provider.as_str()) {
                return err(400, "validation_failed", "Unsupported provider: provider is not enabled");
            }
            if q.get("code_challenge_method").map(String::as_str) != Some("s256") || q.get("code_challenge").is_none_or(|c| c.len() < 43) {
                return err(400, "validation_failed", "PKCE required");
            }
            let email = format!("{provider}-user@example.com");
            st.users.entry(email.clone()).or_insert(FakeUser {
                id: format!("u-{provider}"),
                email: email.clone(),
                password: String::new(),
                confirmed: true,
                provider: provider.clone(),
                plan: None,
                name: None,
            });
            let code = st.next("code");
            st.auth_codes.insert(code.clone(), (email, q["code_challenge"].clone()));
            (302, json!({}), Some(format!("{redirect}?code={code}")))
        }
        ("PUT", "/user") => {
            let email = bearer.and_then(|b| b.strip_prefix("Bearer ")).and_then(|t| st.access.get(t)).cloned();
            let Some(email) = email else { return err(401, "bad_jwt", "invalid JWT") };
            let u = st.users.get_mut(&email).unwrap();
            if let Some(p) = b["password"].as_str() {
                if p == u.password {
                    return err(422, "same_password", "New password should be different from the old password.");
                }
                u.password = p.into();
            }
            if let Some(n) = b["data"]["display_name"].as_str() {
                u.name = Some(n.into());
            }
            (200, st.user_json(&email), None)
        }
        ("POST", "/logout") => {
            st.logouts += 1;
            (204, Value::Null, None)
        }
        _ => err(404, "not_found", "no such route"),
    }
}

/// Like the SQL function in docs/accounts-setup.md: deletes only the caller (`auth.uid()`), and
/// Supabase's foreign keys drop the user's sessions and refresh tokens with it.
fn delete_my_account(st: &mut State, apikey: Option<&String>, bearer: Option<&String>) -> (u16, Value, Option<String>) {
    if apikey.map(String::as_str) != Some(ANON_KEY) {
        return (401, json!({"message": "Invalid API key"}), None);
    }
    if st.no_delete_fn {
        return (404, json!({"code": "PGRST202", "message": "Could not find the function public.delete_my_account without parameters in the schema cache"}), None);
    }
    let email = bearer.and_then(|b| b.strip_prefix("Bearer ")).and_then(|t| st.access.get(t)).cloned();
    let Some(email) = email else { return (401, json!({"code": "PGRST301", "message": "JWT expired"}), None) };
    st.users.remove(&email);
    st.refresh.retain(|_, e| *e != email);
    st.used_refresh.retain(|_, e| *e != email);
    (204, Value::Null, None)
}

fn device(st: &mut State, p: &str, form: &HashMap<String, String>, base: &str) -> (u16, Value) {
    match p {
        "/device/code" => {
            if form.get("client_id").map(String::as_str) != Some("fake-client-id") {
                return (200, json!({"error": "incorrect_client_credentials"}));
            }
            st.device_polls = 0;
            (200, json!({"device_code": "dev-code", "user_code": "FAKE-CODE", "verification_uri": format!("{base}/gh/login/device"), "expires_in": 900, "interval": 1}))
        }
        "/device" => (200, json!({})),
        "/oauth/access_token" => {
            if form.get("grant_type").map(String::as_str) != Some("urn:ietf:params:oauth:grant-type:device_code") || form.get("device_code").map(String::as_str) != Some("dev-code") {
                return (200, json!({"error": "incorrect_device_code"}));
            }
            st.device_polls += 1;
            if st.device_deny {
                (200, json!({"error": "access_denied"}))
            } else if st.device_polls < 2 {
                (200, json!({"error": "authorization_pending"}))
            } else {
                (200, json!({"access_token": GH_TOKEN, "token_type": "bearer", "scope": "repo"}))
            }
        }
        _ => (404, json!({"message": "not found"})),
    }
}

fn api(st: &mut State, method: &str, p: &str, q: &HashMap<String, String>, b: &Value) -> (u16, Value) {
    let repo_json = |full: &str, r: &Repo| json!({"full_name": full, "html_url": format!("https://github.com/{full}"), "default_branch": "main", "private": r.private});
    if (method, p) == ("GET", "/user") {
        return (200, json!({"login": GH_LOGIN}));
    }
    if (method, p) == ("POST", "/user/repos") {
        let name = b["name"].as_str().unwrap_or("").to_string();
        let full = format!("{GH_LOGIN}/{name}");
        if st.repos.contains_key(&full) {
            return (422, json!({"message": "Repository creation failed.", "errors": [{"resource": "Repository", "code": "custom", "field": "name", "message": "name already exists on this account"}]}));
        }
        let mut r = Repo { private: b["private"].as_bool().unwrap_or(false), ..Default::default() };
        if b["auto_init"] == true {
            let readme = format!("# {name}\n").into_bytes();
            let bs = blob_sha(&readme);
            st.blobs.insert(bs.clone(), readme);
            let t = st.next("tree");
            st.trees.insert(t.clone(), vec![("README.md".into(), bs)]);
            let c = st.next("commit");
            st.commits.insert(c.clone(), (t, vec![]));
            r.head = Some(c);
            r.commits = 1;
        }
        let v = repo_json(&full, &r);
        st.repos.insert(full, r);
        return (201, v);
    }
    let Some(rest) = p.strip_prefix("/repos/") else { return (404, json!({"message": "Not Found"})) };
    let mut parts = rest.splitn(3, '/');
    let full = format!("{}/{}", parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let sub = parts.next().map(|s| format!("/{s}")).unwrap_or_default();
    let Some(repo) = st.repos.get(&full).cloned() else { return (404, json!({"message": "Not Found"})) };
    match (method, sub.as_str()) {
        ("GET", "") => (200, repo_json(&full, &repo)),
        ("GET", "/git/ref/heads/main") => match repo.head {
            Some(h) => (200, json!({"object": {"sha": h}})),
            None => (409, json!({"message": "Git Repository is empty."})),
        },
        ("GET", s) if s.starts_with("/git/commits/") => {
            let sha = s.trim_start_matches("/git/commits/");
            match st.commits.get(sha) {
                Some((t, _)) => (200, json!({"sha": sha, "tree": {"sha": t}})),
                None => (404, json!({"message": "Not Found"})),
            }
        }
        ("GET", s) if s.starts_with("/git/trees/") => {
            assert_eq!(q.get("recursive").map(String::as_str), Some("1"));
            let sha = s.trim_start_matches("/git/trees/");
            let entries: Vec<Value> = st.trees[sha].iter().map(|(p, s)| json!({"path": p, "type": "blob", "sha": s, "mode": "100644"})).collect();
            (200, json!({"sha": sha, "tree": entries, "truncated": false}))
        }
        ("POST", "/git/blobs") => {
            assert_eq!(b["encoding"], "base64");
            let data = swiftwin_core::base64_bytes::decode(b["content"].as_str().unwrap_or("")).unwrap_or_default();
            let sha = blob_sha(&data);
            st.blobs.insert(sha.clone(), data);
            st.blob_posts += 1;
            (201, json!({"sha": sha}))
        }
        ("POST", "/git/trees") => {
            assert!(b.get("base_tree").is_none());
            let mut entries = Vec::new();
            for e in b["tree"].as_array().into_iter().flatten() {
                let s = e["sha"].as_str().unwrap_or("").to_string();
                if !st.blobs.contains_key(&s) {
                    return (422, json!({"message": "tree.sha is not a valid blob"}));
                }
                entries.push((e["path"].as_str().unwrap_or("").to_string(), s));
            }
            let t = st.next("tree");
            st.trees.insert(t.clone(), entries);
            (201, json!({"sha": t}))
        }
        ("POST", "/git/commits") => {
            let t = b["tree"].as_str().unwrap_or("").to_string();
            let parents: Vec<String> = b["parents"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(str::to_string)).collect();
            let c = st.next("commit");
            st.commits.insert(c.clone(), (t, parents));
            (201, json!({"sha": c}))
        }
        ("PATCH", "/git/refs/heads/main") => {
            assert_eq!(b["force"], false);
            let sha = b["sha"].as_str().unwrap_or("").to_string();
            let parents = st.commits.get(&sha).map(|c| c.1.clone()).unwrap_or_default();
            if repo.head.as_ref().is_some_and(|h| !parents.contains(h)) {
                return (422, json!({"message": "Update is not a fast forward"}));
            }
            let r = st.repos.get_mut(&full).unwrap();
            r.head = Some(sha.clone());
            r.commits += 1;
            (200, json!({"object": {"sha": sha}}))
        }
        _ => (404, json!({"message": format!("no route {method} {p}")})),
    }
}
