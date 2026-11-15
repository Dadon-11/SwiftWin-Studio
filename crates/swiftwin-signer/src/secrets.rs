//! Secret storage. On Windows: the Windows Credential Manager (generic
//! credentials, target `swiftwin:<name>`), via the `keyring` crate. Secrets are
//! never written to the repo or to config.json.

use std::collections::HashMap;
use std::sync::Mutex;

#[allow(unused_imports)]
use swiftwin_core::{Result, SwiftWinError};

pub const SERVICE: &str = "swiftwin";
pub const ASC_P8: &str = "asc-p8";
pub const DEV_KEY: &str = "dev-signing-key";
pub const GITHUB_TOKEN: &str = "github-token";

pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>>;
    fn set(&self, name: &str, value: &str) -> Result<()>;
    fn delete(&self, name: &str) -> Result<()>;
}

/// In-memory store for tests and `--mock`.
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(name).cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<()> {
        self.0.lock().unwrap().insert(name.into(), value.into());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<()> {
        self.0.lock().unwrap().remove(name);
        Ok(())
    }
}

/// Windows Credential Manager.
#[cfg(windows)]
pub struct CredentialManager;

#[cfg(windows)]
impl CredentialManager {
    fn entry(name: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, name).map_err(|e| SwiftWinError::other(format!("Credential Manager: {e}")))
    }
}

#[cfg(windows)]
impl CredentialManager {
    fn raw_get(name: &str) -> Result<Option<String>> {
        match Self::entry(name)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SwiftWinError::other(format!("Credential Manager read {name}: {e}"))),
        }
    }
    fn raw_set(name: &str, value: &str) -> Result<()> {
        Self::entry(name)?.set_password(value).map_err(|e| SwiftWinError::other(format!("Credential Manager write {name}: {e}")))
    }
    fn raw_delete(name: &str) -> Result<()> {
        match Self::entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SwiftWinError::other(format!("Credential Manager delete {name}: {e}"))),
        }
    }
}

// UNTESTED-ON-DEVICE: only compiled on CI, never exercised with the owner's account.
// A generic credential blob holds at most 2560 bytes, so long values (an RSA
// private key PEM) are split over `name#0..n` with `name` = "chunks:n".
#[cfg(windows)]
impl SecretStore for CredentialManager {
    fn get(&self, name: &str) -> Result<Option<String>> {
        let Some(head) = Self::raw_get(name)? else { return Ok(None) };
        let Some(n) = head.strip_prefix("chunks:").and_then(|n| n.parse::<usize>().ok()) else { return Ok(Some(head)) };
        let mut out = String::new();
        for i in 0..n {
            out.push_str(&Self::raw_get(&format!("{name}#{i}"))?.ok_or_else(|| SwiftWinError::other(format!("secret {name} is incomplete")))?);
        }
        Ok(Some(out))
    }
    fn set(&self, name: &str, value: &str) -> Result<()> {
        self.delete(name)?;
        let chars: Vec<char> = value.chars().collect();
        if chars.len() <= 1000 {
            return Self::raw_set(name, value);
        }
        let chunks: Vec<String> = chars.chunks(1000).map(|c| c.iter().collect()).collect();
        for (i, c) in chunks.iter().enumerate() {
            Self::raw_set(&format!("{name}#{i}"), c)?;
        }
        Self::raw_set(name, &format!("chunks:{}", chunks.len()))
    }
    fn delete(&self, name: &str) -> Result<()> {
        if let Some(n) = Self::raw_get(name)?.and_then(|h| h.strip_prefix("chunks:").and_then(|n| n.parse::<usize>().ok())) {
            for i in 0..n {
                Self::raw_delete(&format!("{name}#{i}"))?;
            }
        }
        Self::raw_delete(name)
    }
}

/// Non-Windows development fallback: files in `<data dir>/secrets` (mode 0600).
/// SwiftWin targets Windows; this exists so the CLI works on a Linux dev box.
#[cfg(not(windows))]
pub struct FileStore;

#[cfg(not(windows))]
impl SecretStore for FileStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        let p = swiftwin_core::paths::data_dir().join("secrets").join(name);
        Ok(std::fs::read_to_string(p).ok())
    }
    fn set(&self, name: &str, value: &str) -> Result<()> {
        let d = swiftwin_core::paths::data_dir().join("secrets");
        std::fs::create_dir_all(&d)?;
        let p = d.join(name);
        std::fs::write(&p, value)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<()> {
        let _ = std::fs::remove_file(swiftwin_core::paths::data_dir().join("secrets").join(name));
        Ok(())
    }
}

/// The platform store: Credential Manager on Windows.
pub fn platform_store() -> Box<dyn SecretStore> {
    #[cfg(windows)]
    {
        Box::new(CredentialManager)
    }
    #[cfg(not(windows))]
    {
        Box::new(FileStore)
    }
}
