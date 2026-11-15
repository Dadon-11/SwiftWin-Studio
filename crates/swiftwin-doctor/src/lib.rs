//! `swiftwin doctor`: the first-connection checklist.
//!
//! Each step prints PASS / FAIL / WARN / SKIP with the exact error and a
//! concrete fix, in the order things must work: USB driver → device visible →
//! trust → helper tools → Developer Mode → iOS version → tunnel → developer
//! disk image → screenshot → WebDriverAgent → signing → build toolchain →
//! AirPlay. A failed step skips the steps that depend on it, so the first
//! FAIL is the thing to fix.

pub mod env;
pub mod setup;

use serde::Serialize;
use swiftwin_core::{DeviceInfo, Result};
use swiftwin_device::real::DeviceSummary;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Status {
    Pass,
    Fail,
    Warn,
    Skip,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub title: &'static str,
    pub status: Status,
    pub detail: String,
    pub fix: Option<String>,
}

/// Everything the doctor needs to probe. Real, mock and test implementations.
pub trait DoctorEnv {
    fn usbmuxd(&self) -> Result<String>;
    fn devices(&self) -> Result<Vec<DeviceSummary>>;
    fn python(&self) -> Option<String>;
    fn pymobiledevice3(&self) -> Option<String>;
    fn device_info(&self, udid: &str) -> Result<DeviceInfo>;
    fn tunnel(&self, udid: &str) -> Result<()>;
    fn developer_image(&self, udid: &str) -> Result<()>;
    fn screenshot(&self, udid: &str) -> Result<(u32, u32)>;
    fn wda(&self, udid: &str) -> Result<String>;
    fn signing(&self) -> Result<String>;
    fn swift(&self) -> Result<String>;
    fn sdk(&self) -> Result<String>;
    fn rcodesign(&self) -> Option<String>;
    fn uxplay(&self) -> Option<String>;
    /// Private copies of SwiftWin's folder in a Microsoft Store app's storage
    /// (see `swiftwin_core::paths::sandboxed_copies`).
    fn sandboxed_copies(&self) -> Vec<String> {
        Vec::new()
    }
}

pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    pub fn ok(&self) -> bool {
        !self.checks.iter().any(|c| c.status == Status::Fail)
    }
    pub fn first_failure(&self) -> Option<&Check> {
        self.checks.iter().find(|c| c.status == Status::Fail)
    }
    pub fn get(&self, id: &str) -> Option<&Check> {
        self.checks.iter().find(|c| c.id == id)
    }

    /// Human-readable text, one line per step plus indented details/fixes.
    pub fn to_text(&self) -> String {
        let n = self.checks.len();
        let mut s = String::new();
        for (i, c) in self.checks.iter().enumerate() {
            let tag = match c.status {
                Status::Pass => "PASS",
                Status::Fail => "FAIL",
                Status::Warn => "WARN",
                Status::Skip => "SKIP",
            };
            s.push_str(&format!("[{:>2}/{n}] {tag}  {:<44} {}\n", i + 1, c.title, c.detail));
            if let Some(f) = &c.fix {
                if c.status != Status::Pass {
                    for (j, line) in f.lines().enumerate() {
                        s.push_str(&format!("{}{}\n", if j == 0 { "        fix:  " } else { "              " }, line));
                    }
                }
            }
        }
        let fails = self.checks.iter().filter(|c| c.status == Status::Fail).count();
        let warns = self.checks.iter().filter(|c| c.status == Status::Warn).count();
        s.push_str(&match self.first_failure() {
            None if warns == 0 => "\nAll checks passed. Your iPhone is ready for SwiftWin.\n".to_string(),
            None => format!("\nNo failures ({warns} warning(s)). SwiftWin can run; see the warnings above.\n"),
            Some(f) => format!("\n{fails} check(s) failed. Fix the first one (\"{}\") and run `swiftwin doctor` again.\n", f.title),
        });
        s
    }
}

struct Runner<'a> {
    env: &'a dyn DoctorEnv,
    checks: Vec<Check>,
}

impl Runner<'_> {
    fn push(&mut self, id: &'static str, title: &'static str, status: Status, detail: impl Into<String>, fix: Option<&str>) -> bool {
        self.checks.push(Check { id, title, status, detail: detail.into(), fix: fix.map(str::to_string) });
        status != Status::Fail && status != Status::Skip
    }
    fn skip(&mut self, id: &'static str, title: &'static str, because: &str) {
        self.push(id, title, Status::Skip, format!("skipped: {because}"), None);
    }
    /// Run `f` if `ok`, else record a skip.
    fn step(&mut self, ok: bool, because: &str, id: &'static str, title: &'static str, f: impl FnOnce(&mut Self) -> bool) -> bool {
        if ok {
            f(self)
        } else {
            self.skip(id, title, because);
            false
        }
    }
}

const FIX_DRIVER: &str = "Install \"Apple Devices\" from the Microsoft Store, or iTunes (the web version from apple.com, needed if you\nsign with Sideloadly), then restart the PC. In services.msc, \"Apple Mobile Device Service\" must be Running.";
const FIX_VISIBLE: &str = "Plug the iPhone in with a data-capable USB cable (try another cable/port), unlock it,\nand make sure the Apple Devices app (or iTunes) shows it.";
const FIX_TRUST: &str = "Unlock the iPhone and tap \"Trust\" on \"Trust This Computer?\" then enter the passcode.\nNo prompt? Run `swiftwin pair`. Still nothing: Settings > General > Transfer or Reset > Reset Location & Privacy, replug.";
const FIX_HELPERS: &str = "Run `swiftwin doctor --fix` (installs pymobiledevice3 into SwiftWin's own Python environment).\nNeeds Python 3.11+: `winget install Python.Python.3.12`.";
const FIX_DEVMODE: &str = "On the iPhone: Settings > Privacy & Security > Developer Mode > On, restart, then confirm \"Turn On\".\nNo Developer Mode entry? Run `swiftwin doctor --fix` to make it appear.";
const FIX_TUNNEL: &str = "iOS 17.4+ uses pymobiledevice3's built-in tunnel (no admin): unplug/replug, keep the phone unlocked and retry.\nOnly on iOS 17.0–17.3: run (as Administrator) `pymobiledevice3 remote tunneld` in another window and retry.";
const FIX_DDI: &str = "Keep the phone unlocked and connected to the internet (the image is personalised by Apple),\nthen retry. Developer Mode must be on.";
const FIX_WDA: &str = "Run `swiftwin wda prepare` (needs signing set up). If the app opens and closes, trust the developer:\nSettings > General > VPN & Device Management.";
const FIX_SIGNING: &str = "Paid developer account: create an App Store Connect API key (Users and Access > Integrations > App Store Connect API, role Admin),\nthen `swiftwin signing setup --key-id ... --issuer-id ... --p8 AuthKey_XXXX.p8 --team-id ...`.\nFree Apple ID: sign in from SwiftWin Studio (avatar menu > Signing), or `swiftwin signing apple-id --email you@example.com --login`.";
const FIX_SWIFT: &str = "Install the Swift toolchain for Windows: https://www.swift.org/install/windows/ (or `winget install Swift.Toolchain`),\nand LLVM for the ld64.lld linker: `winget install LLVM.LLVM`.";
const FIX_SDK: &str = "Download Xcode 26.3 (.xip, it matches Swift 6.2.4) at https://developer.apple.com/download/all/ with your Apple ID,\nthen `swiftwin sdk import C:\\path\\to\\Xcode_26.3_Apple_silicon.xip`.";
const FIX_RCODESIGN: &str = "Run `swiftwin doctor --fix` (downloads rcodesign, and plumesign when signing with a free Apple ID).";
const FIX_UXPLAY: &str = "Optional. Smooth AirPlay mirroring needs UxPlay + Bonjour (see docs/dependencies.md). USB mirroring works without it.";

/// Fix for a private copy of SwiftWin's folder (`copy`) next to the real one (`real`).
fn fix_sandboxed(copy: &str, real: &str) -> String {
    format!(
        "Programs started from a Microsoft Store app (such as the Claude desktop app) saved SwiftWin files in that app's\n\
         private storage, so they and the SwiftWin app see different files. In a normal PowerShell window (not one\n\
         started from that app), merge the copy into the real folder (existing files win) and remove it:\n\
         robocopy \"{copy}\" \"{real}\" /E /MOVE /XC /XN /XO\n\
         Remove-Item -Recurse -Force \"{copy}\""
    )
}

pub fn run(env: &dyn DoctorEnv) -> Report {
    let mut r = Runner { env, checks: Vec::new() };

    // 0. SwiftWin's folder split by a Store app's sandbox (shown only when it happened)
    for copy in r.env.sandboxed_copies() {
        // `...\Packages\<package>\LocalCache\Local\<name>` is the private copy of `%LOCALAPPDATA%\<name>`.
        let name = std::path::Path::new(&copy).file_name().map(|n| n.to_os_string()).unwrap_or_default();
        let real = std::env::var_os("LOCALAPPDATA").map(|l| std::path::PathBuf::from(l).join(&name).display().to_string()).unwrap_or_default();
        let fix = fix_sandboxed(&copy, &real);
        r.checks.push(Check { id: "sandboxed_data", title: "SwiftWin folder not split by an app sandbox", status: Status::Warn, detail: format!("private copy at {copy}"), fix: Some(fix) });
    }

    // 1. USB driver
    let driver = match r.env.usbmuxd() {
        Ok(d) => r.push("usb_driver", "Apple USB driver (Mobile Device Service)", Status::Pass, d, None),
        Err(e) => r.push("usb_driver", "Apple USB driver (Mobile Device Service)", Status::Fail, e.to_string(), Some(FIX_DRIVER)),
    };

    // 2. Device visible
    let mut device: Option<DeviceSummary> = None;
    let visible = r.step(driver, "USB driver missing", "device_visible", "iPhone visible over USB", |r| {
        match r.env.devices() {
            Ok(list) => {
                let usb: Vec<_> = list.iter().filter(|d| d.connection == swiftwin_core::Connection::Usb).cloned().collect();
                match usb.first() {
                    Some(d) => {
                        device = Some(d.clone());
                        let more = if usb.len() > 1 { format!(" (+{} more; use --udid)", usb.len() - 1) } else { String::new() };
                        r.push("device_visible", "iPhone visible over USB", Status::Pass, format!("{}{more}", d.udid), None)
                    }
                    None => r.push("device_visible", "iPhone visible over USB", Status::Fail, "no iPhone found over USB", Some(FIX_VISIBLE)),
                }
            }
            Err(e) => r.push("device_visible", "iPhone visible over USB", Status::Fail, e.to_string(), Some(FIX_VISIBLE)),
        }
    });
    let udid = device.as_ref().map(|d| d.udid.clone()).unwrap_or_default();

    // 3. Trust
    let trusted = r.step(visible, "no device", "trust", "This PC is trusted (paired)", |r| {
        if device.as_ref().is_some_and(|d| d.paired) {
            r.push("trust", "This PC is trusted (paired)", Status::Pass, "pairing record present", None)
        } else {
            r.push("trust", "This PC is trusted (paired)", Status::Fail, "the iPhone has not trusted this PC yet", Some(FIX_TRUST))
        }
    });

    // 4. Helper tools
    let helpers = match (r.env.python(), r.env.pymobiledevice3()) {
        (_, Some(p)) => r.push("helpers", "Device helper (pymobiledevice3)", Status::Pass, p, None),
        (Some(py), None) => r.push("helpers", "Device helper (pymobiledevice3)", Status::Fail, format!("{py} found, pymobiledevice3 missing"), Some(FIX_HELPERS)),
        (None, None) => r.push("helpers", "Device helper (pymobiledevice3)", Status::Fail, "Python 3.11+ and pymobiledevice3 missing", Some(FIX_HELPERS)),
    };

    // 5. Developer Mode + 6. iOS version
    let mut info: Option<DeviceInfo> = None;
    let dev_mode = r.step(trusted && helpers, "needs trust + helper", "developer_mode", "Developer Mode on", |r| {
        match r.env.device_info(&udid) {
            Ok(i) => {
                let st = i.developer_mode;
                info = Some(i);
                match st {
                    Some(true) => r.push("developer_mode", "Developer Mode on", Status::Pass, "enabled", None),
                    Some(false) => r.push("developer_mode", "Developer Mode on", Status::Fail, "Developer Mode is off", Some(FIX_DEVMODE)),
                    None => r.push("developer_mode", "Developer Mode on", Status::Warn, "could not read the Developer Mode state", Some(FIX_DEVMODE)),
                }
            }
            Err(e) => r.push("developer_mode", "Developer Mode on", Status::Fail, e.to_string(), Some(FIX_DEVMODE)),
        }
    });
    let ios_ok = r.step(info.is_some(), "device info unavailable", "ios_version", "iOS version supported", |r| {
        let i = info.as_ref().unwrap();
        let d = format!("{} · {} · iOS {}", i.name, i.model, i.ios_version);
        match i.ios_major() {
            Some(m) if m >= 17 => r.push("ios_version", "iOS version supported", Status::Pass, d, None),
            Some(m) if m >= 15 => r.push("ios_version", "iOS version supported", Status::Warn, format!("{d} (works, but only iOS 17+ was targeted)"), None),
            _ => r.push("ios_version", "iOS version supported", Status::Fail, format!("{d} – SwiftWin needs iOS 15 or newer"), Some("Update the iPhone in Settings > General > Software Update.")),
        }
    });
    let ios17 = info.as_ref().and_then(|i| i.ios_major()).is_some_and(|m| m >= 17);

    // 7. Tunnel (iOS 17+)
    let ready = dev_mode && ios_ok;
    let tunnel = r.step(ready, "needs Developer Mode", "tunnel", "Developer tunnel (iOS 17+)", |r| {
        if !ios17 {
            return r.push("tunnel", "Developer tunnel (iOS 17+)", Status::Pass, "not needed before iOS 17", None);
        }
        match r.env.tunnel(&udid) {
            Ok(()) => r.push("tunnel", "Developer tunnel (iOS 17+)", Status::Pass, "developer services reachable", None),
            Err(e) => r.push("tunnel", "Developer tunnel (iOS 17+)", Status::Fail, e.to_string(), Some(FIX_TUNNEL)),
        }
    });
    // 8. DDI
    let ddi = r.step(ready, "needs Developer Mode", "developer_image", "Developer disk image mounted", |r| match r.env.developer_image(&udid) {
        Ok(()) => r.push("developer_image", "Developer disk image mounted", Status::Pass, "mounted", None),
        Err(e) => r.push("developer_image", "Developer disk image mounted", Status::Fail, e.to_string(), Some(FIX_DDI)),
    });
    // 9. Screenshot
    r.step(tunnel && ddi, "needs tunnel + disk image", "screenshot", "Screenshot over USB", |r| match r.env.screenshot(&udid) {
        Ok((w, h)) => r.push("screenshot", "Screenshot over USB", Status::Pass, format!("{w}x{h} px"), None),
        Err(e) => r.push("screenshot", "Screenshot over USB", Status::Fail, e.to_string(), Some(FIX_DDI)),
    });

    // 10. WDA (its fix depends on whether signing works, so probe signing first)
    let signing_res = r.env.signing();
    let signing = signing_res.is_ok();
    r.step(tunnel && ddi, "needs tunnel + disk image", "wda", "WebDriverAgent (tap/swipe/type)", |r| match r.env.wda(&udid) {
        Ok(d) => r.push("wda", "WebDriverAgent (tap/swipe/type)", Status::Pass, d, None),
        Err(e) => {
            let fix = if signing { FIX_WDA.to_string() } else { format!("Set up signing first (see below), then:\n{FIX_WDA}") };
            r.checks.push(Check { id: "wda", title: "WebDriverAgent (tap/swipe/type)", status: Status::Fail, detail: e.to_string(), fix: Some(fix) });
            false
        }
    });
    // 11. Signing
    match signing_res {
        Ok(d) => r.push("signing", "Signing (App Store Connect key or Apple ID)", Status::Pass, d, None),
        Err(e) => r.push("signing", "Signing (App Store Connect key or Apple ID)", Status::Fail, e.to_string(), Some(FIX_SIGNING)),
    };

    // 12. Toolchain
    match r.env.swift() {
        Ok(v) => r.push("swift", "Swift toolchain", Status::Pass, v, None),
        Err(e) => r.push("swift", "Swift toolchain", Status::Fail, e.to_string(), Some(FIX_SWIFT)),
    };
    match r.env.sdk() {
        Ok(v) => r.push("sdk", "iOS SDK (from your Xcode download)", Status::Pass, v, None),
        Err(e) => r.push("sdk", "iOS SDK (from your Xcode download)", Status::Fail, e.to_string(), Some(FIX_SDK)),
    };
    match r.env.rcodesign() {
        Some(p) => r.push("rcodesign", "Code signing tools", Status::Pass, p, None),
        None => r.push("rcodesign", "Code signing tools", Status::Fail, "not installed", Some(FIX_RCODESIGN)),
    };
    // 13. AirPlay (optional)
    match r.env.uxplay() {
        Some(p) => r.push("airplay", "AirPlay receiver (optional)", Status::Pass, p, None),
        None => r.push("airplay", "AirPlay receiver (optional)", Status::Warn, "UxPlay not installed – USB mirroring only", Some(FIX_UXPLAY)),
    };
    Report { checks: r.checks }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swiftwin_core::{Connection, SwiftWinError};

    /// Configurable fake: every probe succeeds unless listed in `fail`.
    #[derive(Default)]
    struct Fake {
        fail: Vec<&'static str>,
        paired: bool,
        dev_mode: Option<bool>,
        ios: &'static str,
        no_devices: bool,
        sandboxed: Vec<&'static str>,
    }

    impl Fake {
        fn good() -> Fake {
            Fake { paired: true, dev_mode: Some(true), ios: "26.1", ..Default::default() }
        }
        fn r<T>(&self, id: &str, v: T) -> Result<T> {
            if self.fail.contains(&id) {
                Err(SwiftWinError::other(format!("{id} broke")))
            } else {
                Ok(v)
            }
        }
    }

    impl DoctorEnv for Fake {
        fn usbmuxd(&self) -> Result<String> {
            self.r("usbmuxd", "127.0.0.1:27015".into())
        }
        fn devices(&self) -> Result<Vec<DeviceSummary>> {
            if self.no_devices {
                return Ok(vec![]);
            }
            self.r("devices", vec![DeviceSummary { udid: "UDID-1".into(), connection: Connection::Usb, paired: self.paired }])
        }
        fn python(&self) -> Option<String> {
            Some("Python 3.12".into())
        }
        fn pymobiledevice3(&self) -> Option<String> {
            (!self.fail.contains(&"pmd")).then(|| "pymobiledevice3 11.49.5".into())
        }
        fn device_info(&self, udid: &str) -> Result<DeviceInfo> {
            self.r(
                "info",
                DeviceInfo {
                    udid: udid.into(),
                    name: "Ada's iPhone".into(),
                    model: "iPhone17,1".into(),
                    ios_version: self.ios.into(),
                    connection: Connection::Usb,
                    paired: self.paired,
                    developer_mode: self.dev_mode,
                    screen: None,
                },
            )
        }
        fn tunnel(&self, _: &str) -> Result<()> {
            self.r("tunnel", ())
        }
        fn developer_image(&self, _: &str) -> Result<()> {
            self.r("ddi", ())
        }
        fn screenshot(&self, _: &str) -> Result<(u32, u32)> {
            self.r("screenshot", (1206, 2622))
        }
        fn wda(&self, _: &str) -> Result<String> {
            self.r("wda", "ready".into())
        }
        fn signing(&self) -> Result<String> {
            self.r("signing", "team ABCDE12345".into())
        }
        fn swift(&self) -> Result<String> {
            self.r("swift", "Swift 6.2".into())
        }
        fn sdk(&self) -> Result<String> {
            self.r("sdk", "iPhoneOS26.1".into())
        }
        fn rcodesign(&self) -> Option<String> {
            Some("rcodesign 0.29.0".into())
        }
        fn uxplay(&self) -> Option<String> {
            None
        }
        fn sandboxed_copies(&self) -> Vec<String> {
            self.sandboxed.iter().map(|s| s.to_string()).collect()
        }
    }

    fn statuses(r: &Report) -> Vec<(&str, Status)> {
        r.checks.iter().map(|c| (c.id, c.status)).collect()
    }

    #[test]
    fn healthy_setup_passes_in_order() {
        let r = run(&Fake::good());
        assert!(r.ok(), "{}", r.to_text());
        let ids: Vec<&str> = r.checks.iter().map(|c| c.id).collect();
        assert_eq!(
            ids,
            ["usb_driver", "device_visible", "trust", "helpers", "developer_mode", "ios_version", "tunnel", "developer_image", "screenshot", "wda", "signing", "swift", "sdk", "rcodesign", "airplay"]
        );
        assert_eq!(r.get("airplay").unwrap().status, Status::Warn);
        assert!(r.to_text().contains("No failures (1 warning(s))"));
    }

    #[test]
    fn missing_driver_skips_device_steps() {
        let r = run(&Fake { fail: vec!["usbmuxd"], ..Fake::good() });
        let s = statuses(&r);
        assert_eq!(s[0], ("usb_driver", Status::Fail));
        assert_eq!(s[1], ("device_visible", Status::Skip));
        assert_eq!(r.get("screenshot").unwrap().status, Status::Skip);
        assert_eq!(r.first_failure().unwrap().id, "usb_driver");
        let t = r.to_text();
        assert!(t.contains("FAIL  Apple USB driver"));
        assert!(t.contains("fix:  Install \"Apple Devices\""));
        assert!(t.contains("usbmuxd broke"), "exact error is shown");
    }

    #[test]
    fn untrusted_and_developer_mode_off() {
        let r = run(&Fake { paired: false, ..Fake::good() });
        assert_eq!(r.get("trust").unwrap().status, Status::Fail);
        assert_eq!(r.get("developer_mode").unwrap().status, Status::Skip);
        let r = run(&Fake { dev_mode: Some(false), ..Fake::good() });
        assert_eq!(r.get("developer_mode").unwrap().status, Status::Fail);
        assert_eq!(r.get("tunnel").unwrap().status, Status::Skip);
        assert_eq!(r.get("wda").unwrap().status, Status::Skip);
        let r = run(&Fake { no_devices: true, ..Fake::good() });
        assert_eq!(r.get("device_visible").unwrap().detail, "no iPhone found over USB");
    }

    #[test]
    fn tunnel_only_for_ios17_and_wda_hint_mentions_signing() {
        let r = run(&Fake { ios: "16.7", fail: vec!["tunnel"], ..Fake::good() });
        assert_eq!(r.get("tunnel").unwrap().status, Status::Pass);
        assert_eq!(r.get("ios_version").unwrap().status, Status::Warn);
        let r = run(&Fake { fail: vec!["tunnel"], ..Fake::good() });
        assert_eq!(r.get("tunnel").unwrap().status, Status::Fail);
        assert_eq!(r.get("screenshot").unwrap().status, Status::Skip);
        let r = run(&Fake { fail: vec!["wda", "signing"], ..Fake::good() });
        assert!(r.get("wda").unwrap().fix.as_ref().unwrap().starts_with("Set up signing first"));
        let r = run(&Fake { ios: "14.8", ..Fake::good() });
        assert_eq!(r.get("ios_version").unwrap().status, Status::Fail);
        let r = run(&Fake { fail: vec!["pmd"], ..Fake::good() });
        assert_eq!(r.get("helpers").unwrap().status, Status::Fail);
        assert_eq!(r.get("developer_mode").unwrap().status, Status::Skip);
    }

    #[test]
    fn sandboxed_copy_is_a_warning_with_the_merge_command() {
        let copy = r"C:\Users\me\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\SwiftWin";
        let r = run(&Fake { sandboxed: vec![copy], ..Fake::good() });
        let c = &r.checks[0];
        assert_eq!((c.id, c.status), ("sandboxed_data", Status::Warn));
        assert!(c.detail.contains(copy));
        assert!(c.fix.as_ref().unwrap().contains(&format!("robocopy \"{copy}\"")));
        assert!(r.ok(), "a warning, not a failure");
        assert_eq!(r.checks.len(), 16);
        assert!(run(&Fake::good()).get("sandboxed_data").is_none(), "only shown when it happened");
    }

    #[test]
    fn mock_env_passes() {
        let r = run(&env::MockEnv::new());
        assert!(r.ok(), "{}", r.to_text());
        assert!(r.checks.iter().filter(|c| c.id != "airplay").all(|c| c.status == Status::Pass));
    }
}
