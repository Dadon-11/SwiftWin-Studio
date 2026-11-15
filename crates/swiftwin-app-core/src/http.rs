//! Serve `AppCore` + the built web UI over local HTTP (127.0.0.1 only).
//! Used for browser-based UI testing and development: `swiftwin --mock ui`.
//!
//! `POST /api/<cmd>` with a JSON body → `{"ok": true, "result": ...}` or `{"ok": false, "error": "..."}`.
//! Anything else is a static file from `static_dir` (`/` → `index.html`).
//!
//! Security (see docs/security.md):
//! * Only loopback `Host` headers are accepted (defeats DNS rebinding), and a request whose
//!   `Origin` is not this server or the Tauri app is refused.
//! * Every API call needs `X-SwiftWin-Token`, compared in constant time. A custom header forces a
//!   CORS preflight, which this server never grants, so other websites can't call it.
//! * `GET /token` exists only for the browser UI (`swiftwin ui`, which has a static dir). The
//!   desktop app's bridge never hands its token out: `swiftwin mcp` reads it from the user-only
//!   `app-bridge.json`, so other accounts on the PC can't drive the phone.
//! * The bridge serves the app's own embedded phone page (`Ui::Embedded`) for the Claude desktop
//!   app's Browser pane. That page gets the token once, through a one-time pairing code that only a
//!   token holder can mint (`POST /api/phone_link` → `POST /pair`: single use, 2 minutes).
//! * Request lines, headers and bodies are size-limited, slow clients time out, and only a few
//!   connections are served at once.
//!
//! Account, GitHub and Upload commands (`account::is_private`) are only served where the UI is
//! served (`static_dir` set: `swiftwin ui`). The desktop app's bridge has no static dir, so
//! `swiftwin mcp` (Claude Code) can never sign in or out, see account details or upload code.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::AppCore;

/// Largest request body (a source file saved from the editor is far smaller).
pub const MAX_BODY: usize = 16 * 1024 * 1024;
const MAX_LINE: u64 = 8 * 1024;
const MAX_HEADERS: usize = 64;
const MAX_CONNECTIONS: usize = 64;
/// Time a client gets to send its request (long polls wait on the server side, not here).
const READ_TIMEOUT: Duration = Duration::from_secs(20);

/// Content-Security-Policy of the web UI. The Tauri app uses the same policy (tauri.conf.json),
/// so the browser UI test (`swiftwin ui`) catches anything it would block in the app.
pub const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; \
font-src 'self' data:; worker-src 'self' blob:; connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'none'; frame-ancestors 'none'";

/// How long a phone-page pairing code stays valid.
const PAIR_TTL: Duration = Duration::from_secs(120);

/// The web pages a server shows besides its API.
#[derive(Clone)]
pub enum Ui {
    /// API only.
    None,
    /// `swiftwin ui` (browser tests): the built web UI from a folder; hands out its token at `GET /token`.
    Dir(PathBuf),
    /// The desktop app's bridge: the app's own embedded web UI, looked up by path (`/index.html`).
    /// Never hands out its token: the phone page pairs once with a code (see the module docs).
    Embedded(Arc<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>),
}

/// Pairing codes minted by `phone_link`, with when they were minted.
type Codes = Arc<Mutex<Vec<(String, Instant)>>>;

/// `latency` delays every API call except `frame` (simulates a slow phone link in UI tests).
pub fn serve(core: Arc<AppCore>, port: u16, ui: Ui, latency: std::time::Duration) -> std::io::Result<u16> {
    let l = TcpListener::bind(("127.0.0.1", port))?;
    let port = l.local_addr()?.port();
    let open = Arc::new(AtomicUsize::new(0));
    let codes: Codes = Arc::default();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                open.fetch_sub(1, Ordering::SeqCst);
                let mut s = s;
                let _ = respond(&mut s, 503, "text/plain", b"busy");
                continue;
            }
            let (core, ui, open, codes) = (core.clone(), ui.clone(), open.clone(), codes.clone());
            std::thread::spawn(move || {
                if let Ok(c) = s.try_clone() {
                    let _ = handle(c, &core, &ui, &codes, latency, port);
                }
                close_gracefully(s);
                open.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    Ok(port)
}

/// Closing a socket that still has unread request bytes makes Windows send a reset, which can
/// destroy the response (say, a 413) before the client reads it. So: stop writing, then read and
/// drop what the client still sends, for a short while.
fn close_gracefully(s: TcpStream) {
    let _ = s.shutdown(std::net::Shutdown::Write);
    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = (&s).take(1024 * 1024).read_to_end(&mut Vec::new());
}

/// One line of at most `MAX_LINE` bytes; `None` at EOF or when the line is too long.
fn read_line_limited(r: &mut impl BufRead) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    let n = r.take(MAX_LINE).read_until(b'\n', &mut buf)?;
    if n == 0 || !buf.ends_with(b"\n") {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

fn handle(s: TcpStream, core: &AppCore, ui: &Ui, codes: &Codes, latency: std::time::Duration, port: u16) -> std::io::Result<()> {
    s.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut r = BufReader::new(s.try_clone()?);
    let mut s = s;
    let Some(line) = read_line_limited(&mut r)? else { return respond(&mut s, 400, "text/plain", b"bad request") };
    let mut it = line.split_whitespace();
    let (method, target) = (it.next().unwrap_or("").to_string(), it.next().unwrap_or("/").to_string());
    let (mut len, mut host, mut token, mut origin, mut fetch_site) = (0usize, String::new(), String::new(), None, None);
    let mut count = 0;
    loop {
        let Some(h) = read_line_limited(&mut r)? else { return respond(&mut s, 431, "text/plain", b"headers too large") };
        if h == "\r\n" || h == "\n" {
            break;
        }
        count += 1;
        if count > MAX_HEADERS {
            return respond(&mut s, 431, "text/plain", b"too many headers");
        }
        if let Some((k, v)) = h.split_once(':') {
            let v = v.trim();
            match k.trim().to_ascii_lowercase().as_str() {
                "content-length" => match v.parse() {
                    Ok(n) => len = n,
                    Err(_) => return respond(&mut s, 400, "text/plain", b"bad content-length"),
                },
                "transfer-encoding" => return respond(&mut s, 501, "text/plain", b"chunked bodies are not supported"),
                "host" => host = v.to_string(),
                "x-swiftwin-token" => token = v.to_string(),
                "origin" => origin = Some(v.to_string()),
                "sec-fetch-site" => fetch_site = Some(v.to_ascii_lowercase()),
                _ => {}
            }
        }
    }
    if len > MAX_BODY {
        return respond(&mut s, 413, "text/plain", b"too large");
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    let path = target.split('?').next().unwrap_or("/");
    if !host_is_loopback(&host) {
        return respond(&mut s, 403, "text/plain", b"forbidden host");
    }
    if origin.as_deref().is_some_and(|o| !origin_allowed(o, port)) || fetch_site.as_deref() == Some("cross-site") {
        return respond(&mut s, 403, "text/plain", b"forbidden origin");
    }
    let browser_ui = matches!(ui, Ui::Dir(_));
    if path == "/token" && method == "GET" {
        // Only the browser UI needs this; the desktop app's bridge never serves its token.
        if !browser_ui {
            return respond(&mut s, 404, "text/plain", b"not found");
        }
        return respond(&mut s, 200, "application/json", json!({"token": core.token()}).to_string().as_bytes());
    }
    if path == "/pair" && method == "POST" {
        let code = serde_json::from_slice::<Value>(&body).ok().and_then(|v| v["code"].as_str().map(String::from)).unwrap_or_default();
        let mut c = codes.lock().unwrap_or_else(|e| e.into_inner());
        c.retain(|(_, t)| t.elapsed() < PAIR_TTL);
        let Some(i) = c.iter().position(|(k, _)| constant_time_eq(k.as_bytes(), code.as_bytes())) else {
            return respond(&mut s, 403, "application/json", br#"{"ok":false,"error":"this phone link has expired: ask Claude to show the phone again"}"#);
        };
        c.remove(i); // single use
        return respond(&mut s, 200, "application/json", json!({"ok": true, "token": core.token()}).to_string().as_bytes());
    }
    if let (Some(cmd), "POST") = (path.strip_prefix("/api/"), method.as_str()) {
        if !constant_time_eq(token.as_bytes(), core.token().as_bytes()) {
            return respond(&mut s, 403, "application/json", br#"{"ok":false,"error":"bad or missing X-SwiftWin-Token"}"#);
        }
        if cmd == "phone_link" {
            if !matches!(ui, Ui::Embedded(_)) {
                return respond(&mut s, 404, "application/json", br#"{"ok":false,"error":"this server shows no phone page"}"#);
            }
            use rand::Rng;
            let code: String = (0..32).map(|_| format!("{:x}", rand::thread_rng().gen_range(0..16u8))).collect();
            let mut c = codes.lock().unwrap_or_else(|e| e.into_inner());
            c.retain(|(_, t)| t.elapsed() < PAIR_TTL);
            c.push((code.clone(), Instant::now()));
            let url = format!("http://127.0.0.1:{port}/#phone&pair={code}");
            return respond(&mut s, 200, "application/json", json!({"ok": true, "result": {"url": url}}).to_string().as_bytes());
        }
        if !browser_ui && crate::account::is_private(cmd) {
            return respond(&mut s, 403, "application/json", br#"{"ok":false,"error":"this command is only available in the SwiftWin window"}"#);
        }
        let args: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
        if cmd != "frame" && !latency.is_zero() {
            std::thread::sleep(latency);
        }
        let reply = match core.dispatch(cmd, &args) {
            Ok(v) => json!({"ok": true, "result": v}),
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        };
        return respond(&mut s, 200, "application/json", reply.to_string().as_bytes());
    }
    if method != "GET" && method != "HEAD" {
        return respond(&mut s, 405, "text/plain", b"method not allowed");
    }
    if matches!(ui, Ui::None) {
        return respond(&mut s, 404, "text/plain", b"no static dir");
    }
    let rel = path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    let rel_path = Path::new(rel);
    if !swiftwin_core::paths::is_safe_relative(rel_path) {
        return respond(&mut s, 400, "text/plain", b"bad path");
    }
    let data = match ui {
        Ui::Dir(dir) => std::fs::read(dir.join(rel_path)).ok(),
        Ui::Embedded(get) => get(&format!("/{rel}")),
        Ui::None => None,
    };
    match data {
        Some(data) => respond(&mut s, 200, content_type(rel), &data),
        None => respond(&mut s, 404, "text/plain", b"not found"),
    }
}

fn host_is_loopback(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host)
    };
    matches!(name, "127.0.0.1" | "localhost" | "::1")
}

/// Pages that may call the API: this server itself (any loopback name, same port) and the Tauri
/// app's own origins. `null` (sandboxed frames, local files) and every other site are refused.
fn origin_allowed(origin: &str, port: u16) -> bool {
    if matches!(origin, "tauri://localhost" | "http://tauri.localhost" | "https://tauri.localhost") {
        return true;
    }
    let Some(rest) = origin.strip_prefix("http://") else { return false };
    let Some((h, p)) = rest.rsplit_once(':') else { return false };
    // 5173: `npm run dev` (Vite, strictPort) proxies /api and /token here, keeping its Origin.
    p.parse::<u16>().is_ok_and(|p| p == port || p == 5173) && host_is_loopback(&format!("{h}:{p}"))
}

/// Compare secrets without leaking, through timing, how many leading bytes matched.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn content_type(p: &str) -> &'static str {
    match p.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "json" => "application/json",
        "ttf" => "font/ttf",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn respond(s: &mut TcpStream, code: u16, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        s,
        "HTTP/1.1 {code} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\
         X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: {CSP}\r\n\r\n",
        body.len()
    )?;
    s.write_all(body)?;
    s.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn raw(port: u16, req: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = Vec::new();
        let r = s.read_to_end(&mut out);
        assert!(r.is_ok() || !out.is_empty(), "no response: {r:?}");
        String::from_utf8_lossy(&out).into_owned()
    }

    #[test]
    fn token_and_host_are_enforced() {
        let core = Arc::new(AppCore::new(true));
        let tok = core.token().to_string();
        let port = serve(core, 0, Ui::None, std::time::Duration::ZERO).unwrap();
        let body = "{}";
        let call = |host: &str, token: Option<&str>| {
            let t = token.map(|t| format!("X-SwiftWin-Token: {t}\r\n")).unwrap_or_default();
            raw(port, &format!("POST /api/status HTTP/1.1\r\nHost: {host}\r\n{t}Content-Length: {}\r\n\r\n{body}", body.len()))
        };
        assert!(call(&format!("127.0.0.1:{port}"), Some(&tok)).contains("\"ok\":true"));
        assert!(call(&format!("localhost:{port}"), None).starts_with("HTTP/1.1 403"));
        assert!(call(&format!("localhost:{port}"), Some("wrong")).starts_with("HTTP/1.1 403"));
        assert!(call(&format!("localhost:{port}"), Some(&tok[..31])).starts_with("HTTP/1.1 403"), "a prefix is not the token");
        assert!(call("evil.example:80", Some(&tok)).contains("forbidden host"), "DNS rebinding is refused");
        assert!(host_is_loopback("[::1]:8787"));
        // The bridge (no UI served) refuses account and upload commands even with the token.
        for cmd in ["account_status", "signing_status", "signing_asc", "account_sign_out", "account_delete", "github_connect", "upload_start", "open_url"] {
            let r = raw(port, &format!("POST /api/{cmd} HTTP/1.1
Host: 127.0.0.1:{port}
X-SwiftWin-Token: {tok}
Content-Length: 2

{{}}"));
            assert!(r.starts_with("HTTP/1.1 403") && r.contains("only available in the SwiftWin window"), "{cmd}: {r}");
        }
    }

    /// The desktop app's bridge has no static dir: it must not hand its token to whoever asks
    /// (another Windows account on the same PC can reach 127.0.0.1 too).
    #[test]
    fn bridge_never_serves_its_token() {
        let core = Arc::new(AppCore::new(true));
        let tok = core.token().to_string();
        let port = serve(core, 0, Ui::None, std::time::Duration::ZERO).unwrap();
        let t = raw(port, &format!("GET /token HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"));
        assert!(t.starts_with("HTTP/1.1 404") && !t.contains(&tok), "{t}");
    }

    #[test]
    fn browser_ui_gets_its_token_but_other_sites_do_not() {
        let dir = std::env::temp_dir().join(format!("swiftwin-http-static-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<p>hi</p>").unwrap();
        let core = Arc::new(AppCore::new(true));
        let tok = core.token().to_string();
        let port = serve(core, 0, Ui::Dir(dir.clone()), std::time::Duration::ZERO).unwrap();
        let get = |path: &str, extra: &str| raw(port, &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{extra}\r\n"));
        assert!(get("/token", "").contains(&tok));
        assert!(get("/token", &format!("Origin: http://localhost:{port}\r\n")).contains(&tok));
        for evil in ["Origin: https://evil.example\r\n", "Origin: null\r\n", "Origin: http://127.0.0.1:1\r\n", "Sec-Fetch-Site: cross-site\r\n"] {
            let r = get("/token", evil);
            assert!(r.contains("forbidden origin") && !r.contains(&tok), "{evil}: {r}");
        }
        let page = get("/", "");
        assert!(page.contains("<p>hi</p>") && page.contains("Content-Security-Policy: default-src 'self'"), "{page}");
        assert!(page.contains("frame-ancestors 'none'"), "no framing (clickjacking)");
        for bad in ["/../Cargo.toml", "/..%2f", "/C:/Windows/win.ini", "/index.html::$DATA", "/CON", "/nul.txt", "/..\\..\\x"] {
            let r = get(bad, "");
            assert!(r.starts_with("HTTP/1.1 400") || r.starts_with("HTTP/1.1 404"), "{bad}: {r}");
            assert!(!r.contains("<p>hi</p>"), "{bad}");
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// The desktop app's bridge shows its phone page to the Claude app's Browser pane: the page gets
    /// the token only with a one-time code minted by a token holder (`swiftwin mcp`).
    #[test]
    fn bridge_phone_page_pairs_once() {
        let core = Arc::new(AppCore::new(true));
        let tok = core.token().to_string();
        let ui = Ui::Embedded(Arc::new(|p: &str| (p == "/index.html").then(|| b"<p>phone</p>".to_vec())));
        let port = serve(core, 0, ui, std::time::Duration::ZERO).unwrap();
        let post = |path: &str, extra: &str, body: &str| {
            raw(port, &format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{extra}Content-Length: {}\r\n\r\n{body}", body.len()))
        };
        assert!(raw(port, &format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")).contains("<p>phone</p>"));
        assert!(raw(port, &format!("GET /token HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")).starts_with("HTTP/1.1 404"));
        assert!(post("/api/phone_link", "", "{}").starts_with("HTTP/1.1 403"), "minting needs the token");
        let link = post("/api/phone_link", &format!("X-SwiftWin-Token: {tok}\r\n"), "{}");
        let url = link.split("\"url\":\"").nth(1).and_then(|r| r.split('"').next()).expect(&link).to_string();
        assert!(url.starts_with(&format!("http://127.0.0.1:{port}/#phone&pair=")) && !url.contains(&tok), "{url}");
        let code = url.rsplit('=').next().unwrap();
        let pair = |extra: &str, code: &str| post("/pair", extra, &format!("{{\"code\":\"{code}\"}}"));
        for (evil, c) in [("", ""), ("", "0"), ("", &code[..31]), ("Origin: https://evil.example\r\n", code), ("Origin: null\r\n", code)] {
            let r = pair(evil, c);
            assert!(r.starts_with("HTTP/1.1 403") && !r.contains(&tok), "{evil}{c}: {r}");
        }
        assert!(pair(&format!("Origin: http://127.0.0.1:{port}\r\n"), code).contains(&tok), "the page pairs");
        assert!(!pair("", code).contains(&tok), "a code works once");
        // The page can't reach account or upload commands either.
        assert!(post("/api/account_status", &format!("X-SwiftWin-Token: {tok}\r\n"), "{}").starts_with("HTTP/1.1 403"));
        // A server without the embedded page mints no links.
        let core2 = Arc::new(AppCore::new(true));
        let tok2 = core2.token().to_string();
        let port2 = serve(core2, 0, Ui::None, std::time::Duration::ZERO).unwrap();
        let r = raw(port2, &format!("POST /api/phone_link HTTP/1.1\r\nHost: 127.0.0.1:{port2}\r\nX-SwiftWin-Token: {tok2}\r\nContent-Length: 2\r\n\r\n{{}}"));
        assert!(r.starts_with("HTTP/1.1 404"), "{r}");
    }

    #[test]
    fn oversized_and_malformed_requests_are_refused() {
        let core = Arc::new(AppCore::new(true));
        let port = serve(core, 0, Ui::None, std::time::Duration::ZERO).unwrap();
        let big = raw(port, &format!("POST /api/status HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n", MAX_BODY + 1));
        assert!(big.starts_with("HTTP/1.1 413"), "{big}");
        let long = raw(port, &format!("GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Pad: {}\r\n\r\n", "a".repeat(MAX_LINE as usize + 10)));
        assert!(long.starts_with("HTTP/1.1 431"), "{long}");
        let many: String = (0..MAX_HEADERS + 1).map(|i| format!("X-{i}: 1\r\n")).collect();
        assert!(raw(port, &format!("GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n{many}\r\n")).starts_with("HTTP/1.1 431"));
        let te = raw(port, "POST /api/status HTTP/1.1\r\nHost: 127.0.0.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n");
        assert!(te.starts_with("HTTP/1.1 501"), "{te}");
        assert!(raw(port, "POST /api/status HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: -1\r\n\r\n").starts_with("HTTP/1.1 400"));
    }

    /// The browser UI test runs under this server's CSP; it only proves something about the
    /// desktop app if the app's CSP (tauri.conf.json) is the same, apart from Tauri's IPC.
    #[test]
    fn app_and_browser_ui_share_one_csp() {
        let conf: Value = serde_json::from_str(include_str!("../../../app/src-tauri/tauri.conf.json")).unwrap();
        let app = conf["app"]["security"]["csp"].as_str().expect("the desktop app must set a CSP");
        let directives = |csp: &str| -> Vec<String> {
            let mut d: Vec<String> = csp
                .split(';')
                .map(|d| d.trim().replace(" ipc: http://ipc.localhost", ""))
                .filter(|d| !d.is_empty() && !d.starts_with("frame-ancestors"))
                .collect();
            d.sort();
            d
        };
        assert_eq!(directives(app), directives(CSP));
        assert!(app.contains("connect-src 'self' ipc: http://ipc.localhost"));
        assert!(!app.contains("unsafe-eval") && !CSP.contains("unsafe-eval"));
        assert!(app.contains("script-src 'self';"), "no inline scripts");
        assert_eq!(conf["app"]["security"]["dangerousDisableAssetCspModification"], json!(["style-src"]), "keeps 'unsafe-inline' working for styles");
    }

    #[test]
    fn origins() {
        assert!(origin_allowed("http://127.0.0.1:8787", 8787));
        assert!(origin_allowed("http://localhost:8787", 8787));
        assert!(origin_allowed("tauri://localhost", 1));
        assert!(origin_allowed("http://tauri.localhost", 1));
        assert!(origin_allowed("http://localhost:5173", 8787), "npm run dev");
        assert!(!origin_allowed("http://127.0.0.1:3000", 8787), "another local app");
        assert!(!origin_allowed("https://127.0.0.1:8787", 8787));
        assert!(!origin_allowed("http://evil.example:8787", 8787));
        assert!(!origin_allowed("null", 8787));
        assert!(constant_time_eq(b"abc", b"abc") && !constant_time_eq(b"abc", b"abd") && !constant_time_eq(b"ab", b"abc"));
    }
}
