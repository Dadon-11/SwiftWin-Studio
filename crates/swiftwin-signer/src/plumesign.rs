//! Signing with a **free** Apple ID through `plumesign`, the command-line half of
//! Impactor (CLARATION/Impactor, MIT). It logs in to Apple's developer services
//! with the user's Apple ID (the password and 2FA code are typed into plumesign
//! itself, never into SwiftWin), signs an unsigned `.ipa` with a free development
//! certificate, registers the phone and installs the app over usbmuxd:
//!
//! `plumesign sign --package X.ipa --apple-id --custom-identifier <id> --register-and-install --udid <DeviceID>`
//!
//! Checked against the v2.6.3 and v2.7.2 sources:
//! * `--udid` takes usbmuxd's numeric **DeviceID**, not the phone's UDID
//!   (`plume_utils/src/device.rs`, `get_device_for_id`); without it plumesign shows a menu.
//! * The installed bundle id is exactly `--custom-identifier`.
//! * It uses the account selected in `%APPDATA%\PlumeImpactor\accounts.json`, shared
//!   with the Impactor app, and the only team of a free account (no prompt).
//! * `account login` reads the password and the 2FA code with `read_line` from stdin, so the
//!   desktop app drives it through a pipe with its own password field ([`LoginSession`]).
//! * Bundles of unknown type such as `PlugIns/*.xctest` are left **unsigned**, so
//!   WebDriverAgent needs a second pass with rcodesign ([`identity_for`]).
//! * When the account hits Apple's certificate limit, plumesign revokes an existing
//!   development certificate and creates a new one.
//!
//! Apple's limits for free accounts apply: apps stop launching after 7 days,
//! 3 sideloaded apps per phone, 10 new App IDs per week. SwiftWin reuses the same
//! bundle ids, so re-installing does not spend new App IDs.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use swiftwin_core::{Result, SwiftWinError};

use crate::profile::Profile;
use crate::Identity;

pub const PLUMESIGN_VERSION: &str = "2.7.2";
/// SHA-256 of `plumesign-windows-x86_64.exe` from the v2.7.2 release (the release
/// publishes no checksum files), taken on 2026-11-04. 2.7.0+ is needed: since 2026-10-12
/// Apple rejects the older sign-in with SRP error -80045.
const PLUMESIGN_SHA256: &str = "b965e51a613341cb8c7e0f3594b24c09243a235b4187f669ea958a548e6a2fe0";

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "plumesign.exe"
    } else {
        "plumesign"
    }
}

pub fn plumesign_url() -> String {
    format!("https://github.com/CLARATION/Impactor/releases/download/v{PLUMESIGN_VERSION}/plumesign-windows-x86_64.exe")
}

/// `SWIFTWIN_PLUMESIGN`, `<data>/tools/plumesign(.exe)`, then PATH.
pub fn find_plumesign() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SWIFTWIN_PLUMESIGN") {
        return Some(PathBuf::from(p));
    }
    let local = swiftwin_core::paths::tools_dir().join(exe_name());
    if local.exists() {
        return Some(local);
    }
    swiftwin_core::paths::find_on_path(exe_name())
}

/// Download plumesign into SwiftWin's tools folder, checked against the pinned SHA-256.
pub fn install_plumesign() -> Result<PathBuf> {
    if !(cfg!(windows) && cfg!(target_arch = "x86_64")) {
        return Err(SwiftWinError::Unsupported(format!(
            "automatic plumesign install is implemented for Windows x64; get plumesign {PLUMESIGN_VERSION} from https://github.com/CLARATION/Impactor/releases"
        )));
    }
    let data = crate::tools::http_get(&plumesign_url())?;
    verify(&data)?;
    let dest = swiftwin_core::paths::tools_dir().join(exe_name());
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&dest, data)?;
    Ok(dest)
}

fn verify(data: &[u8]) -> Result<()> {
    let got = crate::tools::sha256_hex(data);
    if got != PLUMESIGN_SHA256 {
        return Err(SwiftWinError::invalid(format!(
            "plumesign download does not match the pinned checksum (SHA-256 {got}); run `swiftwin doctor --fix` again"
        )));
    }
    Ok(())
}

pub struct Plumesign {
    pub exe: PathBuf,
}

impl Plumesign {
    pub fn find() -> Result<Plumesign> {
        let exe = find_plumesign().ok_or_else(|| SwiftWinError::ToolMissing {
            tool: "plumesign".into(),
            hint: "run `swiftwin doctor --fix` (or `swiftwin signing apple-id`) to download it".into(),
        })?;
        Ok(Plumesign { exe })
    }

    /// `device_id` is usbmuxd's DeviceID. With `output`, the signed `.ipa` is also saved.
    pub fn sign_install_args(ipa: &Path, bundle_id: &str, device_id: u64, output: Option<&Path>) -> Vec<String> {
        let mut a = vec![
            "sign".into(),
            "--package".into(),
            ipa.to_string_lossy().into_owned(),
            "--apple-id".into(),
            "--custom-identifier".into(),
            bundle_id.into(),
            "--register-and-install".into(),
            "--udid".into(),
            device_id.to_string(),
        ];
        if let Some(o) = output {
            a.push("--output".into());
            a.push(o.to_string_lossy().into_owned());
        }
        a
    }

    /// plumesign logs at "debug" by default; info is enough to see each step.
    fn command(&self) -> Command {
        let mut c = Command::new(&self.exe);
        c.env("RUST_LOG", "info");
        c
    }

    /// Sign `ipa` as `bundle_id` with the saved Apple ID session, register the phone
    /// and install. stdin is closed so a prompt fails fast instead of hanging. A dropped
    /// connection (to Apple or the anisette server) is retried twice: plumesign looks up existing
    /// App IDs and certificates before creating any, so a second run picks up where it stopped.
    /// Confirmed against Apple and an iPhone with iOS 27.0.1 (`swiftwin wda prepare`).
    pub fn sign_and_install(&self, ipa: &Path, bundle_id: &str, device_id: u64, output: Option<&Path>) -> Result<String> {
        let mut attempt = 1;
        loop {
            let out = self
                .command()
                .args(Self::sign_install_args(ipa, bundle_id, device_id, output))
                .stdin(Stdio::null())
                .output()
                .map_err(|e| SwiftWinError::ToolMissing { tool: "plumesign".into(), hint: format!("{e}; run `swiftwin doctor --fix`") })?;
            let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            if out.status.success() {
                return Ok(log);
            }
            if attempt < 3 && is_connection_error(&log) {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_secs(3));
                continue;
            }
            return Err(SwiftWinError::tool("plumesign", failure_summary(&log)));
        }
    }

    /// `plumesign account list` output: ` [First] you@example.com (selected)` per account,
    /// or "No accounts found" (logged to stderr; exit code 0 either way).
    pub fn accounts(&self) -> Result<String> {
        let out = self.command().args(["account", "list"]).stdin(Stdio::null()).output()?;
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        if !out.status.success() {
            return Err(SwiftWinError::tool("plumesign", failure_summary(&text)));
        }
        Ok(text)
    }

    /// Is `email` the selected plumesign account? (Only the saved session is checked,
    /// not whether Apple still accepts it.)
    pub fn signed_in_as(&self, email: &str) -> Result<bool> {
        Ok(selected_account(&self.accounts()?, email))
    }

    /// Interactive `plumesign account login -u <email>` in the user's own terminal:
    /// plumesign asks for the password and the 2FA code itself.
    pub fn login(&self, email: &str) -> Result<()> {
        let status = self.command().args(["account", "login", "-u", email]).status()?;
        if !status.success() {
            return Err(SwiftWinError::Auth(format!("plumesign login failed ({status})")));
        }
        Ok(())
    }
}

/// Where an in-app sign-in stands ([`LoginSession`]).
#[derive(Debug, Clone, PartialEq)]
pub enum LoginState {
    Running,
    /// plumesign asks for the 2FA code.
    NeedCode,
    Done,
    Failed(String),
}

/// `plumesign account login -u <email>` driven through its stdin, so the app can ask for the
/// password and the 2FA code in its own dialog. Checked against plumesign 2.7.2's
/// `commands/account.rs`: without `-p` it reads the password with `read_line`, then the code
/// with `read_line` after "Enter the code sent …" / "Enter 2FA code: ", and ends with
/// "Successfully logged in and account saved." The password goes only to plumesign's stdin:
/// never to a command line, a log or a file.
// UNTESTED-ON-DEVICE: against Apple (a fake plumesign in tests).
pub struct LoginSession {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    out: std::sync::Arc<std::sync::Mutex<String>>,
    codes_sent: usize,
}

/// plumesign's 2FA prompts (one per code it waits for).
fn code_prompts(out: &str) -> usize {
    out.matches("Enter the code sent").count() + out.matches("Enter 2FA code").count()
}

impl LoginSession {
    pub fn start(exe: &Path, email: &str, password: &str) -> Result<LoginSession> {
        use std::io::{Read, Write};
        let mut child = Command::new(exe)
            .args(["account", "login", "-u", email])
            .env("RUST_LOG", "info")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SwiftWinError::ToolMissing { tool: "plumesign".into(), hint: e.to_string() })?;
        let out = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let pipes: [Box<dyn Read + Send>; 2] = [Box::new(child.stdout.take().unwrap()), Box::new(child.stderr.take().unwrap())];
        for mut pipe in pipes {
            let out = out.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                while let Ok(n) = pipe.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    out.lock().unwrap().push_str(&String::from_utf8_lossy(&buf[..n]));
                }
            });
        }
        let mut stdin = child.stdin.take();
        if let Some(i) = stdin.as_mut() {
            writeln!(i, "{password}")?;
        }
        Ok(LoginSession { child, stdin, out, codes_sent: 0 })
    }

    pub fn send_code(&mut self, code: &str) -> Result<()> {
        use std::io::Write;
        let i = self.stdin.as_mut().ok_or_else(|| SwiftWinError::invalid("plumesign is not waiting for a code"))?;
        writeln!(i, "{}", code.trim())?;
        self.codes_sent += 1;
        Ok(())
    }

    pub fn state(&mut self) -> LoginState {
        let out = self.out.lock().unwrap().clone();
        match self.child.try_wait() {
            Ok(Some(status)) => {
                // Let the reader threads catch the last lines.
                std::thread::sleep(std::time::Duration::from_millis(100));
                let out = self.out.lock().unwrap().clone();
                if status.success() && out.contains("Successfully logged in") {
                    LoginState::Done
                } else {
                    LoginState::Failed(failure_summary(&out))
                }
            }
            Ok(None) if code_prompts(&out) > self.codes_sent => LoginState::NeedCode,
            Ok(None) => LoginState::Running,
            Err(e) => LoginState::Failed(e.to_string()),
        }
    }

    /// Poll until plumesign wants a code, finishes or `timeout` passes (then `Running`).
    pub fn wait(&mut self, timeout: std::time::Duration) -> LoginState {
        let end = std::time::Instant::now() + timeout;
        loop {
            let st = self.state();
            if st != LoginState::Running || std::time::Instant::now() > end {
                return st;
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
    }
}

impl Drop for LoginSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A request that never got an answer, e.g. on the owner's PC:
/// `Request Error error sending request for url (https://mac-ani.jkcoxson.com/v3/client_info)` …
/// `tcp connect error` … `An established connection was aborted by the software in your host machine. (os error 10053)`.
pub fn is_connection_error(log: &str) -> bool {
    let l = log.to_ascii_lowercase();
    ["error sending request", "tcp connect error", "os error 10053", "os error 10054", "os error 10060", "connection reset", "timed out"]
        .iter()
        .any(|p| l.contains(p))
}

/// The last few meaningful lines of plumesign's output, for an error message.
fn failure_summary(log: &str) -> String {
    let lines: Vec<&str> = log.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let tail = &lines[lines.len().saturating_sub(6)..];
    if tail.is_empty() {
        "plumesign failed without output".into()
    } else {
        tail.join("\n")
    }
}

fn selected_account(list: &str, email: &str) -> bool {
    let email = email.to_ascii_lowercase();
    list.lines().map(str::to_ascii_lowercase).any(|l| l.contains(&email) && l.contains("(selected)"))
}

/// plumesign's data folder (`%APPDATA%\PlumeImpactor`), shared with the Impactor app.
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("PlumeImpactor"))
}

/// The identity plumesign signed `app` with, so SwiftWin can finish what plumesign skips
/// (the `.xctest` inside WebDriverAgent): the profile embedded in `app`, plumesign's
/// private key for that team (`keys\<TEAM>\key.pem`) and the profile's certificate
/// whose public key matches it.
// Confirmed with plumesign 2.7.2 on Windows: `%APPDATA%\PlumeImpactor\keys\<TEAM>\key.pem`.
pub fn identity_for(app: &Path) -> Result<(Identity, Profile)> {
    let profile = Profile::parse(&std::fs::read(app.join("embedded.mobileprovision"))?)?;
    let dir = data_dir().ok_or_else(|| SwiftWinError::NotFound("APPDATA is not set".into()))?;
    let key_path = dir.join("keys").join(&profile.team_id).join("key.pem");
    let key_pem = std::fs::read_to_string(&key_path)
        .map_err(|e| SwiftWinError::NotFound(format!("plumesign's signing key {}: {e}", key_path.display())))?;
    let cert = matching_certificate(&profile.certificates, &key_pem)?;
    let identity = Identity { cert_id: "plumesign".into(), cert_pem: der_to_pem("CERTIFICATE", &cert), key_pem };
    Ok((identity, profile))
}

/// The certificate that embeds the key's public key (a certificate stores its
/// SubjectPublicKeyInfo verbatim, in canonical DER).
fn matching_certificate(certs: &[Vec<u8>], key_pem: &str) -> Result<Vec<u8>> {
    use rsa::pkcs1::DecodeRsaPrivateKey;
    use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey};
    let key = rsa::RsaPrivateKey::from_pkcs8_pem(key_pem)
        .or_else(|_| rsa::RsaPrivateKey::from_pkcs1_pem(key_pem))
        .map_err(|e| SwiftWinError::invalid(format!("plumesign key: {e}")))?;
    let spki = key.to_public_key().to_public_key_der().map_err(|e| SwiftWinError::other(e.to_string()))?.into_vec();
    certs
        .iter()
        .find(|c| c.windows(spki.len()).any(|w| w == spki.as_slice()))
        .cloned()
        .ok_or_else(|| SwiftWinError::NotFound("none of the profile's certificates matches plumesign's key".into()))
}

fn der_to_pem(label: &str, der: &[u8]) -> String {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut s = format!("-----BEGIN {label}-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        s.push_str(std::str::from_utf8(chunk).expect("base64 is ASCII"));
        s.push('\n');
    }
    s.push_str(&format!("-----END {label}-----\n"));
    s
}

/// A random, stable bundle-id prefix for one user (`dev.swiftwin.u1a2b3c4d`).
pub fn new_id_prefix() -> String {
    format!("dev.swiftwin.u{:08x}", rand::random::<u32>())
}

/// Bundle id used for the WebDriverAgent runner with a free Apple ID.
pub fn wda_runner_id(prefix: &str) -> String {
    format!("{prefix}.WebDriverAgentRunner.xctrunner")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_install_args_never_prompt() {
        let a = Plumesign::sign_install_args(Path::new(r"C:\p\Hello.ipa"), "dev.swiftwin.u1.hello", 7, None);
        assert_eq!(
            a,
            [
                "sign",
                "--package",
                r"C:\p\Hello.ipa",
                "--apple-id",
                "--custom-identifier",
                "dev.swiftwin.u1.hello",
                "--register-and-install",
                "--udid",
                "7"
            ]
        );
        let a = Plumesign::sign_install_args(Path::new("W.ipa"), "x", 1, Some(Path::new("signed.ipa")));
        assert_eq!(a[a.len() - 2..], ["--output", "signed.ipa"]);
    }

    #[test]
    fn selected_account_is_detected() {
        let list = "[2026-11-04T01:00:00Z INFO  plumesign] Saved accounts:\n [First] You@Example.com (selected)\n [Other] b@example.com\n";
        assert!(selected_account(list, "you@example.com"));
        assert!(!selected_account(list, "b@example.com"), "saved but not selected");
        assert!(!selected_account("No accounts found. Login first.", "you@example.com"));
    }

    #[test]
    fn finds_the_certificate_for_the_key() {
        use rsa::pkcs8::EncodePublicKey;
        let kp = crate::csr::KeyPair::generate().unwrap();
        let key_pem = kp.to_pkcs8_pem().unwrap();
        let spki = kp.public_key_der().unwrap();
        // A certificate carries the SubjectPublicKeyInfo verbatim between other fields.
        let mine = [b"head".to_vec(), spki.clone(), b"tail".to_vec()].concat();
        let other = crate::csr::KeyPair::generate().unwrap();
        let theirs = [b"head".to_vec(), other.key.to_public_key().to_public_key_der().unwrap().into_vec()].concat();
        assert_eq!(matching_certificate(&[theirs.clone(), mine.clone()], &key_pem).unwrap(), mine);
        assert!(matching_certificate(&[theirs], &key_pem).is_err());
    }

    #[test]
    fn pem_wraps_at_64_columns() {
        let pem = der_to_pem("CERTIFICATE", &[0u8; 100]);
        assert!(pem.starts_with("-----BEGIN CERTIFICATE-----\n") && pem.ends_with("-----END CERTIFICATE-----\n"));
        assert!(pem.lines().all(|l| l.len() <= 64));
    }

    #[test]
    fn checksum_is_enforced() {
        assert!(verify(b"not plumesign").is_err());
        assert_eq!(PLUMESIGN_SHA256.len(), 64);
    }

    #[test]
    fn ids_are_reverse_dns() {
        let p = new_id_prefix();
        assert!(p.starts_with("dev.swiftwin.u") && p.len() == "dev.swiftwin.u".len() + 8, "{p}");
        assert_ne!(p, new_id_prefix());
        assert_eq!(wda_runner_id("dev.swiftwin.u1"), "dev.swiftwin.u1.WebDriverAgentRunner.xctrunner");
    }

    /// A fake plumesign that reads the password and a 2FA code the way 2.7.2 does.
    #[cfg(windows)]
    #[test]
    fn login_session_passes_password_and_code_through_stdin() {
        use std::time::Duration;
        let dir = std::env::temp_dir().join(format!("swiftwin-fake-plumesign-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("plumesign.cmd");
        let script = [
            "@echo off",
            "set /p PW=Enter password: ",
            r#"if not "%PW%"=="right pw" (echo Error: Bad credentials 1>&2 & exit /b 1)"#,
            "echo [INFO plumesign] Enter 2FA code: 1>&2",
            "set /p CODE=",
            r#"if not "%CODE%"=="123456" (echo Error: Incorrect verification code 1>&2 & exit /b 1)"#,
            "echo [INFO plumesign] Successfully logged in and account saved. 1>&2",
        ];
        std::fs::write(&exe, script.join("\r\n"))
        .unwrap();
        let t = Duration::from_secs(20);
        let mut s = LoginSession::start(&exe, "you@example.com", "right pw").unwrap();
        assert_eq!(s.wait(t), LoginState::NeedCode);
        s.send_code("123456").unwrap();
        assert_eq!(s.wait(t), LoginState::Done);

        let mut s = LoginSession::start(&exe, "you@example.com", "wrong").unwrap();
        assert!(matches!(s.wait(t), LoginState::Failed(m) if m.contains("Bad credentials")));
        let mut s = LoginSession::start(&exe, "you@example.com", "right pw").unwrap();
        assert_eq!(s.wait(t), LoginState::NeedCode);
        s.send_code("000000").unwrap();
        assert!(matches!(s.wait(t), LoginState::Failed(m) if m.contains("Incorrect verification code")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failure_summary_keeps_the_tail() {
        let log = (1..=10).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        assert_eq!(failure_summary(&log), "line 5\nline 6\nline 7\nline 8\nline 9\nline 10");
        assert_eq!(failure_summary("\n\n"), "plumesign failed without output");
    }

    /// plumesign 2.7.2's output for a dropped connection on the owner's PC.
    #[test]
    fn dropped_connections_are_recognised() {
        let log = "Caused by:\n0: Request Error error sending request for url (https://mac-ani.jkcoxson.com/v3/client_info)\n\
                   3: tcp connect error\n4: An established connection was aborted by the software in your host machine. (os error 10053)";
        assert!(is_connection_error(log));
        assert!(!is_connection_error("An App ID with Identifier 'com.x' is not available. Please enter a different string."));
    }
}
