//! `WdaController`: a `Controller` that starts WebDriverAgent on the phone when
//! needed, forwards its port over USB, and restarts it if it dies.
//!
//! The runner is found by bundle id: the configured one, or else any installed app
//! that is a WebDriverAgent runner (`WebDriverAgentRunner-Runner`), so a runner that
//! another tool signed (e.g. Sideloadly, under the user's own bundle id) works too.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use swiftwin_core::config::{WDA_MJPEG_PORT, WDA_PORT};
use swiftwin_core::{Alert, AppInfo, Button, LogQuery, Point, Result, Screenshot, SwiftWinError, UiNode};
use swiftwin_device::{Controller, Device, RunningProcess};

use crate::wda::WdaClient;

/// JPEG quality of the video stream (WDA's default): full-size frames stay sharp at ~150 KB.
const MJPEG_QUALITY: u32 = 25;

/// `CFBundleName` of appium's WebDriverAgent runner app; re-signers keep it.
pub const RUNNER_NAME: &str = "WebDriverAgentRunner-Runner";

pub struct WdaController {
    device: Arc<dyn Device>,
    bundle_id: Mutex<String>,
    start_timeout: Duration,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    client: Option<Arc<WdaClient>>,
    process: Option<Box<dyn RunningProcess>>,
}

impl WdaController {
    pub fn new(device: Arc<dyn Device>, wda_bundle_id: impl Into<String>) -> Self {
        WdaController {
            device,
            bundle_id: Mutex::new(wda_bundle_id.into()),
            start_timeout: Duration::from_secs(90),
            state: Mutex::new(State::default()),
        }
    }

    pub fn with_start_timeout(mut self, t: Duration) -> Self {
        self.start_timeout = t;
        self
    }

    /// The runner's bundle id (the configured one until another runner was found on the phone).
    pub fn bundle_id(&self) -> String {
        self.bundle_id.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Is WDA installed on the phone?
    pub fn is_installed(&self) -> Result<bool> {
        Ok(self.installed_runner()?.is_some())
    }

    /// The installed runner's bundle id: the configured one, else a WebDriverAgent
    /// runner found among the installed apps (which then becomes the one used).
    fn installed_runner(&self) -> Result<Option<String>> {
        let apps = self.device.list_apps()?;
        let mut id = self.bundle_id.lock().unwrap_or_else(|p| p.into_inner());
        if apps.iter().any(|a| a.bundle_id == *id) {
            return Ok(Some(id.clone()));
        }
        if let Some(found) = find_runner(&apps) {
            *id = found.clone();
            return Ok(Some(found));
        }
        Ok(None)
    }

    /// Start (or reuse) WebDriverAgent and return a ready client.
    pub fn client(&self) -> Result<Arc<WdaClient>> {
        let mut st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let alive = st.process.as_mut().map(|p| p.is_running()).unwrap_or(true);
        if let Some(c) = &st.client {
            if alive && c.is_ready() {
                return Ok(c.clone());
            }
        }
        st.client = None;
        // WDA may already be running (our process after a dropped connection,
        // or one started by a previous SwiftWin run).
        let port = self.device.forward_port(WDA_PORT)?;
        let client = Arc::new(WdaClient::new(format!("http://127.0.0.1:{port}")));
        if client.is_ready() {
            st.client = Some(client.clone());
            return Ok(client);
        }
        if let Some(mut p) = st.process.take() {
            let _ = p.stop();
        }
        let Some(id) = self.installed_runner()? else {
            return Err(SwiftWinError::NotFound(format!(
                "WebDriverAgent ({}) is not installed on the phone: run `swiftwin wda prepare` (paid developer \
                 account), or sign the `swiftwin wda ipa` package with Sideloadly and run `swiftwin wda use <bundle id>`",
                self.bundle_id()
            )));
        };
        let env = vec![("USE_PORT".to_string(), WDA_PORT.to_string())];
        let mut process = self.device.start_xctest(&id, &env)?;
        let deadline = Instant::now() + self.start_timeout;
        while !client.is_ready() {
            if !process.is_running() {
                return Err(runner_exited(&id, &self.startup_output(&id)));
            }
            if Instant::now() > deadline {
                let _ = process.stop();
                return Err(SwiftWinError::Timeout(format!(
                    "WebDriverAgent ({id}) did not answer on device port {WDA_PORT} within {}s.{}",
                    self.start_timeout.as_secs(),
                    self.startup_output(&id)
                )));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        st.process = Some(process);
        st.client = Some(client.clone());
        Ok(client)
    }

    /// The runner's last output lines (what pymobiledevice3 said), for error messages.
    fn startup_output(&self, id: &str) -> String {
        let q = LogQuery { process: Some(format!("xctest:{id}")), limit: Some(12), ..Default::default() };
        match swiftwin_device::collect_logs(self.device.as_ref(), &q) {
            Ok(lines) if !lines.is_empty() => {
                let text: Vec<String> = lines.into_iter().map(|e| format!("  {}", e.message)).collect();
                format!("\nRunner output:\n{}", text.join("\n"))
            }
            _ => String::new(),
        }
    }

    /// Stop WebDriverAgent: our XCTest process, and the runner app itself, which
    /// may have been started by another SwiftWin process.
    pub fn stop(&self) -> Result<()> {
        let mut st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        st.client = None;
        if let Some(mut p) = st.process.take() {
            p.stop()?;
        }
        let _ = self.device.terminate(&self.bundle_id());
        Ok(())
    }

    /// WebDriverAgent's MJPEG screen stream at up to `fps` frames per second, full size:
    /// starts WDA if needed, sets the stream's frame rate and forwards its port over USB.
    /// Returns the local `host:port` to read the stream from.
    pub fn video_stream(&self, fps: u32) -> Result<String> {
        self.with(|c| c.set_mjpeg(fps, 100, MJPEG_QUALITY))?;
        let port = self.device.forward_port(WDA_MJPEG_PORT)?;
        Ok(format!("127.0.0.1:{port}"))
    }

    /// Fast screenshot through WDA (falls back to the device's own screenshot service).
    pub fn screenshot(&self) -> Result<Screenshot> {
        match self.client().and_then(|c| c.screenshot()) {
            Ok(s) => Ok(s),
            Err(_) => self.device.screenshot(),
        }
    }

    /// Run `f` with a ready client; on a transport error reconnect once and retry.
    fn with<T>(&self, f: impl Fn(&WdaClient) -> Result<T>) -> Result<T> {
        let c = self.client()?;
        match f(&c) {
            Err(SwiftWinError::Http(_)) => {
                self.state.lock().unwrap_or_else(|p| p.into_inner()).client = None;
                f(self.client()?.as_ref())
            }
            other => other,
        }
    }
}

/// A WebDriverAgent runner among installed apps: appium's runner is named
/// `WebDriverAgentRunner-Runner`, and re-signers usually keep that name even when they
/// change the bundle id. Ids ending in `.xctrunner` win when several match.
pub fn find_runner(apps: &[AppInfo]) -> Option<String> {
    let mut runners: Vec<&AppInfo> = apps
        .iter()
        .filter(|a| {
            a.name.eq_ignore_ascii_case(RUNNER_NAME)
                || a.bundle_id.to_ascii_lowercase().contains("webdriveragentrunner")
        })
        .collect();
    runners.sort_by_key(|a| (!a.bundle_id.ends_with(".xctrunner"), a.bundle_id.clone()));
    runners.first().map(|a| a.bundle_id.clone())
}

impl Controller for WdaController {
    fn window_size(&self) -> Result<(f64, f64)> {
        self.with(|c| c.window_size())
    }
    fn tap(&self, at: Point) -> Result<()> {
        self.with(|c| c.tap(at))
    }
    fn swipe(&self, from: Point, to: Point, d: f64) -> Result<()> {
        self.with(|c| c.swipe(from, to, d))
    }
    fn type_text(&self, text: &str) -> Result<()> {
        self.with(|c| c.type_text(text))
    }
    fn press_button(&self, b: Button) -> Result<()> {
        self.with(|c| c.press_button(b))
    }
    fn ui_tree(&self) -> Result<UiNode> {
        self.with(|c| c.ui_tree())
    }
    fn alert(&self) -> Result<Option<Alert>> {
        self.with(|c| c.alert())
    }
    fn alert_tap(&self, name: &str) -> Result<()> {
        self.with(|c| c.alert_tap(name))
    }
    fn focused(&self) -> Result<Option<UiNode>> {
        self.with(|c| c.focused())
    }
    fn set_appearance(&self, dark: bool) -> Result<()> {
        self.with(|c| c.set_appearance(dark))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, name: &str) -> AppInfo {
        AppInfo { bundle_id: id.into(), name: name.into(), version: None }
    }

    #[test]
    fn finds_a_runner_signed_under_another_bundle_id() {
        let apps = vec![
            app("com.example.hello", "Hello"),
            app("com.example.wda", RUNNER_NAME),
            app("com.other.WebDriverAgentRunner.xctrunner", "WDA"),
        ];
        assert_eq!(find_runner(&apps).as_deref(), Some("com.other.WebDriverAgentRunner.xctrunner"));
        assert_eq!(find_runner(&apps[..2]).as_deref(), Some("com.example.wda"));
        assert_eq!(find_runner(&apps[..1]), None);
    }
}

/// Why the WDA runner quit during startup; `out` is its last output lines.
fn runner_exited(id: &str, out: &str) -> SwiftWinError {
    if swiftwin_device::signing_expired(out) {
        // UNTESTED-ON-DEVICE: matched on recorded launch errors only.
        return SwiftWinError::Other(format!(
            "WebDriverAgent's 7-day signing expired: run `swiftwin wda prepare` (or Connect again in SwiftWin) to re-sign it.{out}"
        ));
    }
    SwiftWinError::tool(
        "WebDriverAgent",
        format!(
            "the XCTest runner {id} exited during startup.{out}
Check that the phone is unlocked, Developer              Mode is on, and the runner's developer is trusted (Settings > General > VPN & Device              Management). A runner signed with a free Apple ID expires after 7 days: sign and install it              again."
        ),
    )
}

#[cfg(test)]
mod runner_tests {
    #[test]
    fn an_expired_runner_says_how_to_re_sign() {
        let out = "
Runner output:
  ERROR Failed to launch: 0xe8008018 (The identity used to sign the executable is no longer valid.)";
        let e = super::runner_exited("com.x.xctrunner", out).to_string();
        assert!(e.starts_with("WebDriverAgent's 7-day signing expired") && e.contains("wda prepare"), "{e}");
        let e = super::runner_exited("com.x.xctrunner", "
Runner output:
  Device is locked").to_string();
        assert!(e.contains("exited during startup") && e.contains("unlocked"), "{e}");
    }
}
