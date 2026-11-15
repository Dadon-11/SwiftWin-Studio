//! Code signing `.app` bundles.
//!
//! Real signing shells out to `rcodesign` (apple-codesign, MPL-2.0), a pure
//! Rust implementation of Apple code signing that runs on Windows:
//! `rcodesign sign --pem-file key.pem --pem-file cert.pem --timestamp-url none --entitlements-xml-file entitlements.plist C:\…\App.app`
//! (checked against rcodesign 0.29.0 on Windows by signing an arm64 iOS bundle).
//! Nested bundles (frameworks, `.xctest` plugins) are signed recursively; the
//! entitlements apply to the main executable only, as with Xcode.
//!
//! Two Windows details:
//! * `--entitlements-xml-file` is a *scoped* setting (`<scope>:<value>`, split
//!   at the first colon), so `C:\tmp\e.plist` is read as scope `C` and the main
//!   executable silently gets **no** entitlements. A `main:` prefix does not
//!   help (it is taken as a path named `main`). rcodesign therefore runs inside
//!   the temp folder and gets the plain file name.
//! * `--timestamp-url none`, as Xcode's development signing does
//!   (`--timestamp=none`): no round trip to timestamp.apple.com over plain HTTP.

use std::path::{Path, PathBuf};
use std::process::Command;

use plist::Dictionary;
use swiftwin_core::{Result, SwiftWinError};

use crate::profile::to_xml;

pub struct SignRequest<'a> {
    pub app: &'a Path,
    pub cert_pem: &'a str,
    pub key_pem: &'a str,
    pub entitlements: &'a Dictionary,
    /// Raw `.mobileprovision` bytes, embedded as `embedded.mobileprovision`.
    pub profile: &'a [u8],
}

pub trait CodeSigner: Send + Sync {
    fn describe(&self) -> String;
    fn sign(&self, req: &SignRequest) -> Result<()>;
}

/// Remove old signatures so re-signing starts clean.
pub fn strip_signatures(app: &Path) -> Result<()> {
    fn walk(d: &Path) -> std::io::Result<()> {
        for e in std::fs::read_dir(d)? {
            let p = e?.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "_CodeSignature") {
                    std::fs::remove_dir_all(&p)?;
                } else {
                    walk(&p)?;
                }
            }
        }
        Ok(())
    }
    walk(app)?;
    Ok(())
}

pub struct Rcodesign {
    pub exe: PathBuf,
}

/// File names inside the temp folder rcodesign runs in (no drive letter, no colon).
const KEY_FILE: &str = "key.pem";
const CERT_FILE: &str = "cert.pem";
const ENT_FILE: &str = "entitlements.plist";

impl Rcodesign {
    /// Arguments for a run whose working directory holds `key.pem`, `cert.pem`
    /// and `entitlements.plist`. `app` must be absolute.
    pub fn args(app: &Path) -> Vec<String> {
        vec![
            "sign".into(),
            "--pem-file".into(),
            KEY_FILE.into(),
            "--pem-file".into(),
            CERT_FILE.into(),
            "--timestamp-url".into(),
            "none".into(),
            "--entitlements-xml-file".into(),
            ENT_FILE.into(),
            app.to_string_lossy().into_owned(),
        ]
    }
}

/// A new folder in the temp dir with a random name, created here (never one that already
/// exists, which someone else could have made to read what we write into it) and, on Unix,
/// readable by this user only. On Windows `%TEMP%` is already private to the user.
pub(crate) fn private_temp_dir(prefix: &str) -> Result<PathBuf> {
    use rand::Rng;
    for _ in 0..8 {
        let name: String = (0..16).map(|_| format!("{:x}", rand::thread_rng().gen_range(0..16u8))).collect();
        let dir = std::env::temp_dir().join(format!("{prefix}-{name}"));
        #[allow(unused_mut)]
        let mut b = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        match b.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(SwiftWinError::other("could not create a private temp folder"))
}

fn absolute(p: &Path) -> Result<PathBuf> {
    Ok(if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir()?.join(p) })
}

impl CodeSigner for Rcodesign {
    fn describe(&self) -> String {
        format!("rcodesign ({})", self.exe.display())
    }

    // iOS 27.0.1 accepted and ran a WDA runner re-signed here (free-Apple-ID certificate).
    fn sign(&self, req: &SignRequest) -> Result<()> {
        let app = absolute(req.app)?;
        strip_signatures(&app)?;
        std::fs::write(app.join("embedded.mobileprovision"), req.profile)?;
        // Key material lives in a private temp folder only for the duration of the call.
        let tmp = private_temp_dir("swiftwin-sign")?;
        let result = (|| {
            std::fs::write(tmp.join(KEY_FILE), req.key_pem)?;
            std::fs::write(tmp.join(CERT_FILE), req.cert_pem)?;
            std::fs::write(tmp.join(ENT_FILE), to_xml(req.entitlements)?)?;
            let out = Command::new(&self.exe).args(Self::args(&app)).current_dir(&tmp).output().map_err(|e| {
                SwiftWinError::ToolMissing { tool: "rcodesign".into(), hint: format!("{e}; run `swiftwin doctor --fix`") }
            })?;
            if !out.status.success() {
                return Err(SwiftWinError::tool("rcodesign", String::from_utf8_lossy(&out.stderr).trim()));
            }
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&tmp);
        result
    }
}

/// Used by `--mock` and tests: embeds the profile and writes placeholder
/// signature files so the rest of the pipeline sees a "signed" bundle.
pub struct MockSigner;

impl CodeSigner for MockSigner {
    fn describe(&self) -> String {
        "mock signer (no real signature)".into()
    }
    fn sign(&self, req: &SignRequest) -> Result<()> {
        strip_signatures(req.app)?;
        if !req.key_pem.contains("PRIVATE KEY") || !req.cert_pem.contains("CERTIFICATE") {
            return Err(SwiftWinError::invalid("mock signer: key/certificate are not PEM"));
        }
        std::fs::write(req.app.join("embedded.mobileprovision"), req.profile)?;
        let sig = req.app.join("_CodeSignature");
        std::fs::create_dir_all(&sig)?;
        std::fs::write(sig.join("CodeResources"), to_xml(&Dictionary::new())?)?;
        std::fs::write(sig.join("entitlements.plist"), to_xml(req.entitlements)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_folder_is_new_and_private() {
        let a = private_temp_dir("swiftwin-test-sign").unwrap();
        let b = private_temp_dir("swiftwin-test-sign").unwrap();
        assert_ne!(a, b, "random names");
        assert!(a.is_dir() && std::fs::read_dir(&a).unwrap().next().is_none(), "created empty, not reused");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&a).unwrap().permissions().mode() & 0o777, 0o700);
        }
        std::fs::remove_dir_all(a).ok();
        std::fs::remove_dir_all(b).ok();
    }

    #[test]
    fn rcodesign_args() {
        let a = Rcodesign::args(Path::new(r"C:\work\A.app"));
        assert_eq!(
            a.join(" "),
            r"sign --pem-file key.pem --pem-file cert.pem --timestamp-url none --entitlements-xml-file entitlements.plist C:\work\A.app"
        );
        // Scoped settings are split at the first ':' – the entitlements value must not contain one.
        let ent = &a[a.iter().position(|x| x == "--entitlements-xml-file").unwrap() + 1];
        assert!(!ent.contains(':'));
        assert!(absolute(Path::new("A.app")).unwrap().is_absolute());
    }
}
