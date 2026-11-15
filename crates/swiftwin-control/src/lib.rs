//! Phone = device services + UI controller, chosen for mock or real hardware.
//!
//! * `wda` – HTTP client for WebDriverAgent.
//! * `controller` – starts/keeps WDA alive on a real phone (`WdaController`).
//! * `fakewda` – a WDA-compatible HTTP server backed by `MockDevice` for tests.

pub mod controller;
pub mod fakewda;
pub mod wda;

use std::path::Path;
use std::sync::Arc;

use swiftwin_core::config::Config;
use swiftwin_core::{AppInfo, Result, Screenshot, SwiftWinError};
use swiftwin_device::{Controller, Device, MockDevice, RealDevice, RealOptions};

pub use controller::WdaController;

/// Everything SwiftWin can do with one connected phone.
#[derive(Clone)]
pub struct Phone {
    pub device: Arc<dyn Device>,
    pub control: Arc<dyn Controller>,
    /// Present on real phones: lifecycle of WebDriverAgent.
    pub wda: Option<Arc<WdaController>>,
    pub is_mock: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ConnectOptions {
    /// Use the simulated iPhone.
    pub mock: bool,
    /// Pick a specific device when several are connected.
    pub udid: Option<String>,
}

impl Phone {
    pub fn mock() -> Phone {
        let m = MockDevice::new();
        Phone { device: Arc::new(m.clone()), control: Arc::new(m), wda: None, is_mock: true }
    }

    /// Build a phone from a device; real devices get a `WdaController`.
    pub fn from_device(device: Arc<dyn Device>, wda_bundle_id: &str) -> Phone {
        let wda = Arc::new(WdaController::new(device.clone(), wda_bundle_id));
        Phone { device, control: wda.clone(), wda: Some(wda), is_mock: false }
    }

    /// Best screenshot available: WDA's (fast) on real phones, else the device service.
    pub fn screenshot(&self) -> Result<Screenshot> {
        match &self.wda {
            Some(w) => w.screenshot(),
            None => self.device.screenshot(),
        }
    }

    /// A live MJPEG stream of the screen at up to `fps` (real phones: WebDriverAgent's,
    /// forwarded over USB). Returns the local `host:port` to read it from.
    pub fn video_stream(&self, fps: u32) -> Result<String> {
        match &self.wda {
            Some(w) => w.video_stream(fps),
            None => Err(SwiftWinError::Unsupported("the simulated iPhone has no video stream".into())),
        }
    }

    /// Install a signed WebDriverAgent runner and remember its bundle id.
    pub fn install_wda(&self, package: &Path) -> Result<AppInfo> {
        let info = self.device.install(package)?;
        if !self.is_mock {
            let mut c = Config::load();
            c.wda_bundle_id = Some(info.bundle_id.clone());
            c.save()?;
        }
        Ok(info)
    }
}

/// Connect to a phone. With `mock` (or `SWIFTWIN_MOCK=1`) this never touches USB.
pub fn connect(opts: &ConnectOptions) -> Result<Phone> {
    if swiftwin_core::mock_requested(opts.mock) {
        return Ok(Phone::mock());
    }
    let real = Arc::new(RealDevice::open(opts.udid.as_deref(), RealOptions::default())?);
    Ok(Phone::from_device(real, &Config::load().wda_bundle_id()))
}
