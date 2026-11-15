//! Loopback redirect receiver for browser sign-in (RFC 8252 §7.3): listen on
//! `127.0.0.1:<random port>`, wait for `GET /auth/callback?code=…` (or `?error=…`), show a
//! short page, and hand the code back. Only loopback is bound, so nothing else can reach it.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use swiftwin_core::{Result, SwiftWinError};

pub const CALLBACK_PATH: &str = "/auth/callback";

pub struct Loopback {
    listener: TcpListener,
    port: u16,
}

impl Loopback {
    pub fn start() -> Result<Loopback> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        Ok(Loopback { listener, port })
    }

    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}{}", self.port, CALLBACK_PATH)
    }

    /// Wait for the browser to come back. Returns the authorization code.
    pub fn wait(self, timeout: Duration, cancel: &AtomicBool) -> Result<String> {
        let end = Instant::now() + timeout;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(SwiftWinError::other("sign-in cancelled"));
            }
            if Instant::now() > end {
                return Err(SwiftWinError::Timeout("no answer from the browser; try signing in again".into()));
            }
            match self.listener.accept() {
                Ok((s, _)) => {
                    if let Some(r) = handle(s) {
                        return r;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
                Err(e) => return Err(e.into()),
            }
        }
    }
}

/// `None` = not our callback (favicon, stray request): keep waiting.
fn handle(s: TcpStream) -> Option<Result<String>> {
    let _ = s.set_nonblocking(false);
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let mut r = BufReader::new(s.try_clone().ok()?);
    let mut line = String::new();
    r.read_line(&mut line).ok()?;
    // Drain headers so the browser sees a clean response.
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).ok()? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
    }
    let mut it = line.split_whitespace();
    let (method, target) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut s = s;
    if method != "GET" || path != CALLBACK_PATH {
        let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return None;
    }
    let mut code = None;
    let mut error = None;
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "error_description" => error = Some(v.into_owned()),
            "error" if error.is_none() => error = Some(v.into_owned()),
            _ => {}
        }
    }
    let (title, body, result) = match (code, error) {
        (Some(c), _) if !c.is_empty() => ("Signed in", "You're signed in to SwiftWin. You can close this tab and go back to the app.", Ok(c)),
        (_, Some(e)) => ("Sign-in failed", "Sign-in didn't finish. Go back to SwiftWin to see why and try again.", Err(SwiftWinError::Auth(e))),
        _ => (
            "Sign-in failed",
            "SwiftWin got no sign-in code. Go back to the app and try again.",
            Err(SwiftWinError::protocol("the browser came back without a code (is PKCE on for this Supabase project?)")),
        ),
    };
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>SwiftWin: {title}</title>\
         <body style=\"font:16px system-ui;background:#0b0d12;color:#e8eaf0;display:grid;place-items:center;height:90vh\">\
         <div style=\"text-align:center\"><h2>{title}</h2><p>{body}</p></div>"
    );
    let _ = write!(
        s,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_code_and_ignores_strays() {
        let lb = Loopback::start().unwrap();
        let uri = lb.redirect_uri();
        assert!(uri.starts_with("http://127.0.0.1:") && uri.ends_with("/auth/callback"));
        let u = uri.clone();
        std::thread::spawn(move || {
            let base = u.trim_end_matches(CALLBACK_PATH).to_string();
            let _ = ureq::get(&format!("{base}/favicon.ico")).call();
            let body = ureq::get(&format!("{u}?code=abc-123")).call().unwrap().into_string().unwrap();
            assert!(body.contains("signed in"));
        });
        assert_eq!(lb.wait(Duration::from_secs(10), &AtomicBool::new(false)).unwrap(), "abc-123");

        let lb = Loopback::start().unwrap();
        let u = lb.redirect_uri();
        std::thread::spawn(move || {
            let _ = ureq::get(&format!("{u}?error=access_denied&error_description=User%20cancelled")).call();
        });
        assert_eq!(lb.wait(Duration::from_secs(10), &AtomicBool::new(false)).unwrap_err().to_string(), "authentication required: User cancelled");

        let lb = Loopback::start().unwrap();
        assert!(lb.wait(Duration::from_secs(5), &AtomicBool::new(true)).is_err());
    }
}
