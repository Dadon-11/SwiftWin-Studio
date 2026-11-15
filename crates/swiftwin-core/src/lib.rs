//! Shared types, errors and small utilities used by every SwiftWin crate.

pub mod config;
pub mod error;
pub mod paths;
pub mod time;
pub mod types;

pub use error::{Result, SwiftWinError};
pub use types::*;

/// Environment variable that forces the simulated iPhone (`MockDevice`).
pub const MOCK_ENV: &str = "SWIFTWIN_MOCK";

/// Returns true when the caller asked for the mock device, either with an
/// explicit `--mock` flag or with `SWIFTWIN_MOCK=1` in the environment.
pub fn mock_requested(flag: bool) -> bool {
    if flag {
        return true;
    }
    match std::env::var(MOCK_ENV) {
        Ok(v) => matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_flag_wins() {
        assert!(mock_requested(true));
    }
}
