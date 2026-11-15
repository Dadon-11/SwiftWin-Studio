//! Locating and fetching the external tools the signer uses.
//!
//! * `rcodesign` 0.29.0 (apple-codesign, MPL-2.0) from indygreg/apple-platform-rs releases.
//! * WebDriverAgent `WebDriverAgentRunner-Runner.zip` (BSD-3-Clause) from appium/WebDriverAgent
//!   releases – an unsigned real-device build that SwiftWin re-signs with the user's identity.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use swiftwin_core::{Result, SwiftWinError};

pub const RCODESIGN_VERSION: &str = "0.29.0";
pub const WDA_VERSION: &str = "16.15.3";

pub fn rcodesign_url() -> String {
    let triple = if cfg!(windows) { "x86_64-pc-windows-msvc" } else if cfg!(target_os = "macos") { "macos-universal" } else { "x86_64-unknown-linux-musl" };
    let ext = if cfg!(windows) { "zip" } else { "tar.gz" };
    format!(
        "https://github.com/indygreg/apple-platform-rs/releases/download/apple-codesign%2F{v}/apple-codesign-{v}-{triple}.{ext}",
        v = RCODESIGN_VERSION
    )
}

/// SHA-256 of `apple-codesign-0.29.0-x86_64-pc-windows-msvc.zip`, from the `.sha256` file of the
/// release, checked again on 2026-11-05. Pinned here: a checksum fetched from the same place as
/// the file only catches corruption, not a replaced release asset.
const RCODESIGN_WINDOWS_SHA256: &str = "54bb500e2da7a8de02fcae0f331d1cac6e6d7173b4281042ff9c528ba3159aaa";

/// SHA-256 of `WebDriverAgentRunner-Runner.zip` v16.15.3 (GitHub's asset digest, and the
/// downloaded file, 2026-11-05). WebDriverAgent runs on the phone with the user's signing identity.
const WDA_SHA256: &str = "2176793538d902f6b1e61291e9997f68d306541f222ea715d256a7c6ce9d4fc5";

pub fn wda_url() -> String {
    format!("https://github.com/appium/WebDriverAgent/releases/download/v{WDA_VERSION}/WebDriverAgentRunner-Runner.zip")
}

/// Download the WebDriverAgent runner zip, checked against the pinned SHA-256.
pub fn download_wda() -> Result<Vec<u8>> {
    let data = http_get(&wda_url())?;
    check_pinned("WebDriverAgent", &data, WDA_SHA256)?;
    Ok(data)
}

/// Refuse a download whose SHA-256 is not the pinned one.
pub(crate) fn check_pinned(what: &str, data: &[u8], want: &str) -> Result<()> {
    let got = sha256_hex(data);
    if got != want {
        return Err(SwiftWinError::invalid(format!(
            "{what} download does not match the pinned checksum (SHA-256 {got}, expected {want}); try again, and report it if it keeps happening"
        )));
    }
    Ok(())
}

fn exe_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

/// `SWIFTWIN_RCODESIGN`, `<data>/tools/rcodesign(.exe)`, then PATH.
pub fn find_rcodesign() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SWIFTWIN_RCODESIGN") {
        return Some(PathBuf::from(p));
    }
    let local = swiftwin_core::paths::tools_dir().join(exe_name("rcodesign"));
    if local.exists() {
        return Some(local);
    }
    swiftwin_core::paths::find_on_path(&exe_name("rcodesign"))
}

/// Download `url` with retries. Each attempt starts at a different resolved
/// address (GitHub's release CDN has several; one stalling edge must not
/// sink the download), and connect/read timeouts make a stalled connection
/// fail fast instead of hanging for minutes.
pub fn http_get(url: &str) -> Result<Vec<u8>> {
    http_get_with(url, 4, Duration::from_secs(1))
}

fn http_get_with(url: &str, attempts: usize, backoff: Duration) -> Result<Vec<u8>> {
    let mut last = String::new();
    for attempt in 0..attempts {
        if attempt > 0 {
            std::thread::sleep(backoff * (1 << (attempt - 1)) as u32);
        }
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(30))
            .timeout(Duration::from_secs(900))
            .resolver(move |netloc: &str| -> std::io::Result<Vec<std::net::SocketAddr>> {
                use std::net::ToSocketAddrs;
                Ok(rotate(netloc.to_socket_addrs()?.collect(), attempt))
            })
            .build();
        let res = agent.get(url).set("User-Agent", "swiftwin").call();
        match res {
            Ok(r) => {
                let mut buf = Vec::new();
                match r.into_reader().read_to_end(&mut buf) {
                    Ok(_) => return Ok(buf),
                    Err(e) => last = format!("reading body: {e}"),
                }
            }
            // 4xx (except 429) will not get better by retrying.
            Err(ureq::Error::Status(code, _)) if code != 429 && code < 500 => {
                return Err(SwiftWinError::Http(format!("GET {url}: HTTP {code}")));
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(SwiftWinError::Http(format!("GET {url}: failed after {attempts} attempts: {last}")))
}

/// Move the first `n` addresses to the back, so retry `n` starts elsewhere.
fn rotate<T>(mut addrs: Vec<T>, n: usize) -> Vec<T> {
    if !addrs.is_empty() {
        let k = n % addrs.len();
        addrs.rotate_left(k);
    }
    addrs
}

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// Extract the file named `name` (anywhere in the zip) to `dest`.
pub fn extract_named(zip_bytes: &[u8], name: &str, dest: &Path) -> Result<()> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| SwiftWinError::invalid(format!("zip: {e}")))?;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| SwiftWinError::invalid(format!("zip: {e}")))?;
        if f.name().ok().is_some_and(|n| n.rsplit('/').next() == Some(name)) {
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p)?;
            }
            let mut out = std::fs::File::create(dest)?;
            std::io::copy(&mut f, &mut out)?;
            return Ok(());
        }
    }
    Err(SwiftWinError::NotFound(format!("{name} not found in archive")))
}

/// Download rcodesign into SwiftWin's tools folder (Windows release is a zip),
/// checked against the pinned SHA-256.
pub fn install_rcodesign() -> Result<PathBuf> {
    if !(cfg!(windows) && cfg!(target_arch = "x86_64")) {
        return Err(SwiftWinError::Unsupported("automatic rcodesign install is implemented for Windows x64; install it with `cargo install apple-codesign`".into()));
    }
    let url = rcodesign_url();
    let data = http_get(&url)?;
    check_pinned("rcodesign", &data, RCODESIGN_WINDOWS_SHA256)?;
    let dest = swiftwin_core::paths::tools_dir().join(exe_name("rcodesign"));
    extract_named(&data, &exe_name("rcodesign"), &dest)?;
    Ok(dest)
}

/// Unpack a zip into `dest` (used for the WDA runner).
pub fn unzip_all(zip_bytes: &[u8], dest: &Path) -> Result<()> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| SwiftWinError::invalid(format!("zip: {e}")))?;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| SwiftWinError::invalid(format!("zip: {e}")))?;
        let Some(rel) = f.enclosed_name() else { continue };
        let out = dest.join(rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p)?;
            }
            let mut w = std::fs::File::create(&out)?;
            std::io::copy(&mut f, &mut w)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Serves `statuses` in order, one per connection, then stops. Counts requests.
    fn serve(statuses: &'static [u16]) -> (String, Arc<AtomicUsize>) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/f.zip", l.local_addr().unwrap().port());
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        std::thread::spawn(move || {
            for (s, code) in l.incoming().flatten().zip(statuses) {
                h.fetch_add(1, Ordering::SeqCst);
                let mut s = s;
                let mut req = Vec::new();
                let mut b = [0u8; 1];
                while !req.ends_with(b"\r\n\r\n") && s.read(&mut b).unwrap_or(0) == 1 {
                    req.push(b[0]);
                }
                let body = if *code == 200 { "payload" } else { "" };
                let _ = write!(s, "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (url, hits)
    }

    #[test]
    fn retries_server_errors_then_succeeds() {
        let (url, hits) = serve(&[503, 500, 200]);
        assert_eq!(http_get_with(&url, 4, Duration::from_millis(1)).unwrap(), b"payload");
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn does_not_retry_client_errors() {
        let (url, hits) = serve(&[404, 200]);
        let e = http_get_with(&url, 4, Duration::from_millis(1)).unwrap_err().to_string();
        assert!(e.contains("404"), "{e}");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn gives_up_after_all_attempts() {
        let (url, _) = serve(&[503, 503]);
        let e = http_get_with(&url, 2, Duration::from_millis(1)).unwrap_err().to_string();
        assert!(e.contains("after 2 attempts"), "{e}");
    }

    #[test]
    fn rotates_addresses_per_attempt() {
        assert_eq!(rotate(vec![1, 2, 3, 4], 0), vec![1, 2, 3, 4]);
        assert_eq!(rotate(vec![1, 2, 3, 4], 1), vec![2, 3, 4, 1]);
        assert_eq!(rotate(vec![1, 2, 3, 4], 5), vec![2, 3, 4, 1]);
        assert!(rotate(Vec::<u8>::new(), 3).is_empty());
    }

    #[test]
    fn downloads_are_pinned() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        for h in [RCODESIGN_WINDOWS_SHA256, WDA_SHA256] {
            assert!(h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{h}");
        }
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(check_pinned("x", b"abc", abc).is_ok());
        let e = check_pinned("WebDriverAgent", b"tampered", abc).unwrap_err().to_string();
        assert!(e.contains("does not match the pinned checksum"), "{e}");
    }
}
