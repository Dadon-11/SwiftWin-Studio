//! Doctor environments: the real machine + phone, and the mock phone.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use swiftwin_core::config::Config;
use swiftwin_core::{Connection, DeviceInfo, Result, SwiftWinError};
use swiftwin_device::real::usbmux::Usbmux;
use swiftwin_device::real::{pmd, DeviceSummary};
use swiftwin_device::{Device, MockDevice, RealDevice, RealOptions};

use crate::DoctorEnv;

/// Probes the real PC and phone.
pub struct RealEnv {
    mux: Usbmux,
    devices: RefCell<HashMap<String, Arc<RealDevice>>>,
}

impl Default for RealEnv {
    fn default() -> Self {
        RealEnv { mux: Usbmux::platform(), devices: RefCell::new(HashMap::new()) }
    }
}

impl RealEnv {
    fn dev(&self, udid: &str) -> Result<Arc<RealDevice>> {
        if let Some(d) = self.devices.borrow().get(udid) {
            return Ok(d.clone());
        }
        let d = Arc::new(RealDevice::open(Some(udid), RealOptions { mux: self.mux.clone(), runner: None })?);
        self.devices.borrow_mut().insert(udid.into(), d.clone());
        Ok(d)
    }
}

// Every probe below ran against an iPhone with iOS 27.0.1, except the App Store Connect one.
// UNTESTED-ON-DEVICE: the App Store Connect signing probe (free Apple ID was used).
impl DoctorEnv for RealEnv {
    fn usbmuxd(&self) -> Result<String> {
        self.mux.list_devices()?;
        Ok(match &self.mux.addr {
            swiftwin_device::real::usbmux::MuxAddr::Tcp(a) => format!("reachable at {a}"),
            #[cfg(unix)]
            swiftwin_device::real::usbmux::MuxAddr::Unix(p) => format!("reachable at {}", p.display()),
        })
    }
    fn devices(&self) -> Result<Vec<DeviceSummary>> {
        swiftwin_device::real::list_devices(&self.mux)
    }
    fn python(&self) -> Option<String> {
        let py = pmd::find_python()?;
        let (a, b) = pmd::python_version(&py)?;
        Some(format!("Python {a}.{b}"))
    }
    fn pymobiledevice3(&self) -> Option<String> {
        pmd::locate().map(|p| p.join(" "))
    }
    fn device_info(&self, udid: &str) -> Result<DeviceInfo> {
        self.dev(udid)?.info()
    }
    fn tunnel(&self, udid: &str) -> Result<()> {
        self.dev(udid)?.pmd()?.tunnel_check()
    }
    fn developer_image(&self, udid: &str) -> Result<()> {
        self.dev(udid)?.ensure_developer_services()
    }
    fn screenshot(&self, udid: &str) -> Result<(u32, u32)> {
        let s = self.dev(udid)?.screenshot()?;
        Ok((s.width, s.height))
    }
    fn wda(&self, udid: &str) -> Result<String> {
        let dev: Arc<dyn Device> = self.dev(udid)?;
        let phone = swiftwin_control::Phone::from_device(dev, &Config::load().wda_bundle_id());
        let w = phone.wda.clone().expect("real phone has WDA");
        let c = w.client()?;
        let tree = swiftwin_device::Controller::ui_tree(c.as_ref())?;
        Ok(format!("ready at {} (front app: {})", c.base_url(), tree.label.unwrap_or_else(|| "?".into())))
    }
    fn signing(&self) -> Result<String> {
        let cfg = Config::load();
        if cfg.uses_apple_id() {
            let email = cfg.apple_id.clone().unwrap_or_default();
            let ps = swiftwin_signer::plumesign::Plumesign::find()?;
            if !ps.signed_in_as(&email)? {
                return Err(SwiftWinError::Auth(format!(
                    "{email} is not signed in to plumesign: sign in from SwiftWin Studio (avatar menu > Signing) or with `swiftwin signing apple-id --email {email} --login`"
                )));
            }
            return Ok(format!("free Apple ID {email} via plumesign · bundle ids under {}", cfg.id_prefix.unwrap_or_default()));
        }
        let store: Arc<dyn swiftwin_signer::SecretStore> = Arc::from(swiftwin_signer::secrets::platform_store());
        let svc = swiftwin_signer::SigningService::from_config(&cfg, store)?;
        let certs = svc.asc.list_dev_certificates()?;
        Ok(format!(
            "team {} · App Store Connect OK · {} development certificate(s)",
            cfg.team_id.unwrap_or_else(|| "?".into()),
            certs.len()
        ))
    }
    fn swift(&self) -> Result<String> {
        let exe = swiftwin_builder::compile::find_swiftc(Config::load().swift_bin.as_deref())
            .ok_or_else(|| SwiftWinError::NotFound("swiftc not found".into()))?;
        // swift_command puts Swift's runtime DLLs on PATH; without them swiftc exits with
        // 0xC0000135 (DLL not found) and prints nothing, e.g. in a shell started before Swift was installed.
        let out = swiftwin_builder::compile::swift_command(&exe).arg("--version").output()?;
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        if !out.status.success() {
            return Err(SwiftWinError::tool("swiftc", format!("`{} --version` failed ({}): {}", exe.display(), out.status, text.trim())));
        }
        let version = text.lines().next().unwrap_or("swiftc").trim().to_string();
        // iOS apps are linked by ld64.lld outside swiftc (see swiftwin_builder::compile).
        let ld64 = swiftwin_builder::compile::find_ld64(&exe).ok_or_else(|| {
            SwiftWinError::NotFound(format!("{version}, but no ld64.lld linker: run `winget install LLVM.LLVM` (or set SWIFTWIN_LD64)"))
        })?;
        Ok(format!("{version} · linker {}", ld64.display()))
    }
    fn sdk(&self) -> Result<String> {
        let s = swiftwin_sdk::active(&swiftwin_core::paths::sdk_dir(), Config::load().sdk.as_deref())?;
        Ok(format!("{} at {}", s.name, s.sdk_path.display()))
    }
    fn rcodesign(&self) -> Option<String> {
        let rcodesign = swiftwin_signer::tools::find_rcodesign()?.display().to_string();
        if !Config::load().uses_apple_id() {
            return Some(rcodesign);
        }
        // Free Apple ID: plumesign signs and installs; rcodesign finishes WebDriverAgent.
        let plumesign = swiftwin_signer::plumesign::find_plumesign()?;
        Some(format!("plumesign {} · rcodesign {rcodesign}", plumesign.display()))
    }
    fn uxplay(&self) -> Option<String> {
        swiftwin_mirror::airplay::find_uxplay().map(|p| p.display().to_string())
    }
    fn sandboxed_copies(&self) -> Vec<String> {
        swiftwin_core::paths::sandboxed_copies().iter().map(|p| p.display().to_string()).collect()
    }
}

/// The simulated phone: every check passes (AirPlay is optional).
pub struct MockEnv {
    dev: MockDevice,
}

impl MockEnv {
    pub fn new() -> Self {
        MockEnv { dev: MockDevice::new() }
    }
}

impl Default for MockEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl DoctorEnv for MockEnv {
    fn usbmuxd(&self) -> Result<String> {
        Ok("mock (no USB needed)".into())
    }
    fn devices(&self) -> Result<Vec<DeviceSummary>> {
        let i = self.dev.info()?;
        Ok(vec![DeviceSummary { udid: i.udid, connection: Connection::Usb, paired: true }])
    }
    fn python(&self) -> Option<String> {
        Some("mock".into())
    }
    fn pymobiledevice3(&self) -> Option<String> {
        Some("mock".into())
    }
    fn device_info(&self, _: &str) -> Result<DeviceInfo> {
        self.dev.info()
    }
    fn tunnel(&self, _: &str) -> Result<()> {
        Ok(())
    }
    fn developer_image(&self, _: &str) -> Result<()> {
        Ok(())
    }
    fn screenshot(&self, _: &str) -> Result<(u32, u32)> {
        let s = self.dev.screenshot()?;
        Ok((s.width, s.height))
    }
    fn wda(&self, _: &str) -> Result<String> {
        Ok("mock controller (no WebDriverAgent needed)".into())
    }
    fn signing(&self) -> Result<String> {
        Ok("mock signer".into())
    }
    fn swift(&self) -> Result<String> {
        Ok("mock compiler".into())
    }
    fn sdk(&self) -> Result<String> {
        Ok("mock SDK".into())
    }
    fn rcodesign(&self) -> Option<String> {
        Some("mock".into())
    }
    fn uxplay(&self) -> Option<String> {
        None
    }
}
