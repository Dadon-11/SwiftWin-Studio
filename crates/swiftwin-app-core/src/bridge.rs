//! The "bridge" lets `swiftwin mcp` (started by Claude Code) send its tool
//! calls to the open SwiftWin desktop app, so the person watching the app
//! sees every tap/swipe Claude makes (AI mode).
//!
//! The app serves `AppCore` on 127.0.0.1 with a random per-run token and
//! writes `{port, token, pid}` to `<data dir>/app-bridge.json` (user-only).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bridge {
    pub port: u16,
    pub token: String,
    pub pid: u32,
}

pub fn path() -> PathBuf {
    swiftwin_core::paths::data_dir().join("app-bridge.json")
}

pub fn write(port: u16, token: &str) -> std::io::Result<()> {
    let p = path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let b = Bridge { port, token: token.into(), pid: std::process::id() };
    std::fs::write(&p, serde_json::to_vec(&b).unwrap_or_default())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn read() -> Option<Bridge> {
    serde_json::from_slice(&std::fs::read(path()).ok()?).ok()
}

/// Remove the bridge file if it is ours.
pub fn remove() {
    if read().is_some_and(|b| b.pid == std::process::id()) {
        let _ = std::fs::remove_file(path());
    }
}
