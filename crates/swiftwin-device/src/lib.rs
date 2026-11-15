//! SwiftWin's device bridge.
//!
//! Every operation on the phone goes through two traits:
//!
//! * [`Device`] – things the USB device services do: info, install, launch,
//!   screenshot, syslog, crash reports.
//! * [`Controller`] – UI automation: tap, swipe, type, buttons, UI tree.
//!   On a real phone this is WebDriverAgent (see `swiftwin-control`).
//!
//! There are two backends: [`MockDevice`] (a simulated iPhone used by all UI,
//! MCP and integration tests and by `--mock`) and `RealDevice` (USB).

pub mod bundle;
pub mod logbuf;
pub mod mock;
pub mod real;

use std::path::{Path, PathBuf};

pub use mock::MockDevice;
pub use real::{RealDevice, RealOptions};
use swiftwin_core::{Alert, AppInfo, Button, DeviceInfo, LogEntry, LogQuery, Point, Result, Screenshot, SwiftWinError, UiNode};

/// Options for launching an app.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Kill a running instance first (default on real devices).
    pub kill_existing: bool,
}

/// Health of the background device-log capture, reported with every `logs` read.
#[derive(Debug, Clone, Default)]
pub struct LogStatus {
    /// Sequence number of the next log line: `LogQuery::since` for "only what came after this read".
    pub next: u64,
    /// Seconds since the phone last sent a log line (`None`: nothing received yet).
    pub silent_secs: Option<f64>,
    /// How often the capture was restarted because it stalled or ended.
    pub restarts: u64,
    /// What went wrong last (stalled, ended, could not start), if anything.
    pub problem: Option<String>,
}

/// A long-running process on the phone side (e.g. the WebDriverAgent XCTest runner).
pub trait RunningProcess: Send {
    fn is_running(&mut self) -> bool;
    fn stop(&mut self) -> Result<()>;
}

/// USB-level operations on one iPhone.
pub trait Device: Send + Sync {
    fn info(&self) -> Result<DeviceInfo>;
    fn list_apps(&self) -> Result<Vec<AppInfo>>;
    /// Install a signed `.ipa` file or `.app` directory. Returns the installed app's identity.
    fn install(&self, package: &Path) -> Result<AppInfo>;
    fn uninstall(&self, bundle_id: &str) -> Result<()>;
    /// Launch an app, returning its process id.
    fn launch(&self, bundle_id: &str, opts: &LaunchOptions) -> Result<u32>;
    fn terminate(&self, bundle_id: &str) -> Result<()>;
    fn screenshot(&self) -> Result<Screenshot>;
    /// Read the device log. `sink` is called for each matching entry and
    /// returns `false` to stop. Without `query.follow` it returns after the backlog.
    fn syslog(&self, query: &LogQuery, sink: &mut dyn FnMut(LogEntry) -> bool) -> Result<()>;
    /// Start collecting the device log in the background, so a later `syslog` call also sees what
    /// happened before it (the phone's live log has no history). Long-running hosts call this once
    /// after connecting. The default does nothing.
    fn keep_logs(&self) -> Result<()> {
        Ok(())
    }
    /// State of the background log capture (`None`: this device keeps no log buffer).
    fn log_status(&self) -> Option<LogStatus> {
        None
    }
    /// Copy crash reports into `out_dir`; returns the files written.
    fn crash_reports(&self, out_dir: &Path) -> Result<Vec<PathBuf>>;
    /// Start an XCTest runner (used for WebDriverAgent).
    fn start_xctest(&self, runner_bundle_id: &str, env: &[(String, String)]) -> Result<Box<dyn RunningProcess>>;
    /// Forward a TCP port on the phone to a local port on this PC. Returns the local port.
    fn forward_port(&self, device_port: u16) -> Result<u16>;
}

/// UI automation on the phone. Coordinates are in device points.
pub trait Controller: Send + Sync {
    /// Screen size in points.
    fn window_size(&self) -> Result<(f64, f64)>;
    fn tap(&self, at: Point) -> Result<()>;
    fn swipe(&self, from: Point, to: Point, duration_s: f64) -> Result<()>;
    /// Type into the focused element.
    fn type_text(&self, text: &str) -> Result<()>;
    fn press_button(&self, button: Button) -> Result<()>;
    fn ui_tree(&self) -> Result<UiNode>;
    /// The alert on screen (system or app), if any.
    fn alert(&self) -> Result<Option<Alert>> {
        Ok(None)
    }
    /// Tap the button labelled `name` in the alert on screen.
    fn alert_tap(&self, _name: &str) -> Result<()> {
        Err(SwiftWinError::Unsupported("this phone has no alert support".into()))
    }
    /// The element with keyboard focus (kind, label, frame); `Ok(None)` when nothing has focus.
    fn focused(&self) -> Result<Option<UiNode>> {
        Err(SwiftWinError::Unsupported("this phone cannot report keyboard focus".into()))
    }
    /// Switch the whole phone to dark (`true`) or light appearance.
    fn set_appearance(&self, _dark: bool) -> Result<()> {
        Err(SwiftWinError::Unsupported("this phone cannot switch dark/light appearance".into()))
    }
}

/// Convenience: collect the log backlog into a Vec.
pub fn collect_logs(dev: &dyn Device, query: &LogQuery) -> Result<Vec<LogEntry>> {
    let mut q = query.clone();
    q.follow = false;
    let mut out = Vec::new();
    dev.syslog(&q, &mut |e| {
        out.push(e);
        true
    })?;
    Ok(out)
}

/// What to tell the user when iOS refuses to launch an app whose free-account signing ran out.
pub const SIGNING_EXPIRED: &str =
    "This app's 7-day signing expired: run it again from SwiftWin (Run / install_run) to re-sign. \
     (If you just installed it, trust its developer instead: Settings > General > VPN & Device Management.)";

// UNTESTED-ON-DEVICE: the strings come from Apple's MobileDevice error table and launch errors
// reported by others; not yet seen on the owner's phone (apps signed with a free Apple ID stop
// opening after 7 days).
/// True when an iOS launch error means the app's signature or provisioning profile is no longer
/// accepted: expired free-account profile (0xe8008018 family), or a profile iOS won't verify.
pub fn signing_expired(text: &str) -> bool {
    // Collapse whitespace: long messages arrive wrapped over several log lines.
    let t = text.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    [
        "0xe8008018",
        "0xe8008015",
        "identity used to sign the executable is no longer valid",
        "valid provisioning profile for this executable was not found",
        "applicationverificationfailed",
        "application verification failed",
        "could not be verified",
        "unable to verify app",
        "profile has expired",
        "provisioning profile has expired",
        "invalid code signature, inadequate entitlements or its profile has not been explicitly trusted",
    ]
    .iter()
    .any(|p| t.contains(p))
}

#[cfg(test)]
mod signing_tests {
    use super::signing_expired;

    #[test]
    fn expired_signing_is_recognised() {
        // Recorded / documented launch failures.
        for m in [
            "ERROR Failed to launch: 0xe8008018 (The identity used to sign the executable is no longer valid.)",
            "Error Domain=FBSOpenApplicationServiceErrorDomain Code=1 \"The request to open \"com.x.Hello\" failed.\" \
             UserInfo={NSLocalizedFailureReason=Unable to launch com.x.Hello because it has an invalid code signature, \
             inadequate entitlements or its profile has not been explicitly trusted by the user.}",
            "ApplicationVerificationFailed: Failed to verify code signature of /private/var/installd/Hello.app",
            "“Hello” could not be verified. It cannot be opened.",
            "A valid provisioning profile for this executable was not found.",
        ] {
            assert!(signing_expired(m), "{m}");
        }
        for m in ["Device is not paired.", "Developer Mode is disabled.", "could not read pid from launch output"] {
            assert!(!signing_expired(m), "{m}");
        }
    }
}
