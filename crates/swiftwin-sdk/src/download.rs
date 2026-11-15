//! Resumable download of `Xcode_<version>.xip` using the user's own Apple
//! developer session.
//!
//! Apple's download host needs the `ADCDownloadAuth` cookie that
//! developer.apple.com sets after the user signs in with their Apple ID. The
//! desktop app obtains it by letting the user sign in inside an embedded
//! browser window; on the command line the user can paste it
//! (`swiftwin sdk download --cookie ...`) or download the .xip in any browser
//! and run `swiftwin sdk import`. SwiftWin never sees or stores the password.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use swiftwin_core::{Result, SwiftWinError};

/// Xcode 26.3 ships Swift 6.2.4 (xcodereleases.com), the Windows toolchain SwiftWin pairs it with:
/// the SDK's Swift interfaces must not be newer than the compiler reading them (26.4+ is Swift 6.3).
/// Override with `--xcode`.
pub const DEFAULT_XCODE: &str = "26.3";

/// Apple's download URL. Since Xcode 26 every release comes as `_Apple_silicon` and `_Universal`
/// .xip files with the same iOS SDK (the Apple silicon one is smaller), and 26.0 is named `26`.
pub fn xcode_url(version: &str) -> String {
    let v = version.strip_suffix(".0").filter(|v| !v.contains('.')).unwrap_or(version);
    let major: u32 = v.split('.').next().and_then(|m| m.parse().ok()).unwrap_or(0);
    let file = if major >= 26 { format!("Xcode_{v}_Apple_silicon.xip") } else { format!("Xcode_{v}.xip") };
    format!("https://download.developer.apple.com/Developer_Tools/Xcode_{v}/{file}")
}

/// Download `url` to `dest`, resuming from `dest.part` if present.
/// `cookie` is sent as `Cookie: ADCDownloadAuth=<value>` unless it already contains '='.
// UNTESTED-ON-DEVICE: Apple's real download host and cookie (needs the owner's Apple ID).
pub fn download(url: &str, dest: &Path, cookie: Option<&str>, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<PathBuf> {
    let part = dest.with_extension("xip.part");
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p)?;
    }
    let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).timeout_read(Duration::from_secs(120)).build();
    let mut req = agent.get(url).set("User-Agent", "SwiftWin (Xcode SDK import)");
    if let Some(c) = cookie {
        let c = if c.contains('=') { c.to_string() } else { format!("ADCDownloadAuth={c}") };
        req = req.set("Cookie", &c);
    }
    if have > 0 {
        req = req.set("Range", &format!("bytes={have}-"));
    }
    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(401 | 403, _)) => {
            return Err(SwiftWinError::Auth(
                "Apple refused the download: sign in at developer.apple.com with your Apple ID first (cookie missing or expired)".into(),
            ))
        }
        Err(ureq::Error::Status(416, _)) if have > 0 => {
            std::fs::rename(&part, dest)?;
            return Ok(dest.to_path_buf());
        }
        Err(e) => return Err(SwiftWinError::Http(format!("download failed: {e}"))),
    };
    let ctype = resp.header("Content-Type").unwrap_or("").to_string();
    if ctype.starts_with("text/html") {
        return Err(SwiftWinError::Auth(
            "Apple returned a web page instead of the .xip – your developer.apple.com session is not valid".into(),
        ));
    }
    let resumed = resp.status() == 206;
    let total = resp.header("Content-Length").and_then(|v| v.parse::<u64>().ok()).map(|l| if resumed { l + have } else { l });
    let mut file = OpenOptions::new().create(true).write(true).append(resumed).truncate(!resumed).open(&part)?;
    let mut done = if resumed { have } else { 0 };
    let mut r = resp.into_reader();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        progress(done, total);
    }
    file.flush()?;
    drop(file);
    if let Some(t) = total {
        if done != t {
            return Err(SwiftWinError::Http(format!("download incomplete ({done} of {t} bytes); run the command again to resume")));
        }
    }
    std::fs::rename(&part, dest)?;
    Ok(dest.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    /// Serves `body`, honouring `Range: bytes=N-`, requiring the cookie.
    fn server(body: Vec<u8>, cut_first_at: Option<usize>) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}/Xcode.xip", l.local_addr().unwrap());
        std::thread::spawn(move || {
            let mut first = true;
            for s in l.incoming().flatten() {
                let mut r = BufReader::new(s.try_clone().unwrap());
                let (mut range, mut cookie) = (0usize, false);
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    let l = line.to_ascii_lowercase();
                    if let Some(v) = l.strip_prefix("range: bytes=") {
                        range = v.trim().trim_end_matches('-').parse().unwrap();
                    }
                    if l.starts_with("cookie: adcdownloadauth=secret") {
                        cookie = true;
                    }
                }
                let mut s = s;
                if !cookie {
                    let _ = s.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                }
                let rest = &body[range..];
                let status = if range > 0 { "206 Partial Content" } else { "200 OK" };
                let hdr = format!("HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", rest.len());
                s.write_all(hdr.as_bytes()).unwrap();
                let n = if first { cut_first_at.unwrap_or(rest.len()) } else { rest.len() };
                let _ = s.write_all(&rest[..n]);
                first = false;
            }
        });
        addr
    }

    #[test]
    fn downloads_and_resumes() {
        let body: Vec<u8> = (0..3_000_000u32).map(|i| (i % 253) as u8).collect();
        let url = server(body.clone(), Some(1_000_000));
        let dir = std::env::temp_dir().join(format!("swiftwin-dl-{}", std::process::id()));
        let dest = dir.join("Xcode.xip");
        let e = download(&url, &dest, Some("secret"), &mut |_, _| {});
        assert!(e.is_err(), "first attempt is cut short");
        let mut last = 0;
        download(&url, &dest, Some("secret"), &mut |d, _| last = d).unwrap();
        assert_eq!(last, 3_000_000);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_cookie_is_auth_error() {
        let url = server(vec![1, 2, 3], None);
        let dest = std::env::temp_dir().join(format!("swiftwin-dl2-{}/x.xip", std::process::id()));
        assert!(matches!(download(&url, &dest, None, &mut |_, _| {}), Err(SwiftWinError::Auth(_))));
    }

    #[test]
    fn xcode_urls_follow_apples_naming() {
        // As listed by xcodereleases.com.
        let base = "https://download.developer.apple.com/Developer_Tools";
        assert_eq!(xcode_url("26.3"), format!("{base}/Xcode_26.3/Xcode_26.3_Apple_silicon.xip"));
        assert_eq!(xcode_url("26.0"), format!("{base}/Xcode_26/Xcode_26_Apple_silicon.xip"));
        assert_eq!(xcode_url("26.0.1"), format!("{base}/Xcode_26.0.1/Xcode_26.0.1_Apple_silicon.xip"));
        assert_eq!(xcode_url("16.4"), format!("{base}/Xcode_16.4/Xcode_16.4.xip"));
        assert_eq!(xcode_url(DEFAULT_XCODE), format!("{base}/Xcode_26.3/Xcode_26.3_Apple_silicon.xip"));
    }
}
