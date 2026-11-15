//! `RealDevice`: an iPhone attached over USB.
//!
//! Detection, trust and basic info use a native usbmuxd/lockdown client
//! (`usbmux`, `lockdown`). Everything that needs an authenticated lockdown
//! session, the iOS 17+ RemoteXPC tunnel or the developer disk image goes
//! through pymobiledevice3 (`pmd`) for now – see docs/dependencies.md.
//!
//! Ran against an iPhone with iOS 27.0.1 on Windows (2026-11-04). Paths that have not run on a
//! phone yet are marked `// UNTESTED-ON-DEVICE` and listed in docs/untested.md.

pub mod lockdown;
pub mod models;
pub mod pmd;
pub mod usbmux;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use swiftwin_core::{
    AppInfo, Connection, DeviceInfo, LogEntry, LogLevel, LogQuery, Result, Screenshot, SwiftWinError,
};

use crate::logbuf::LogBuffer;
use crate::{Device, LaunchOptions, RunningProcess};
use pmd::{Killer, LineStream, Pmd, ProcessRunner, Runner};
use usbmux::{same_udid, udid_variants, MuxDevice, Usbmux};

/// A phone visible to usbmuxd, before opening it.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceSummary {
    pub udid: String,
    pub connection: Connection,
    /// True when this computer has a pairing record (the user tapped "Trust").
    pub paired: bool,
}

/// List phones attached to this computer (confirmed with the Apple Mobile Device Service on Windows).
pub fn list_devices(mux: &Usbmux) -> Result<Vec<DeviceSummary>> {
    let mut out = Vec::new();
    for d in mux.list_devices()? {
        let paired = has_pair_record(mux, &[d.udid.as_str()]);
        let connection = connection_of(&d);
        out.push(DeviceSummary { udid: d.udid, connection, paired });
    }
    Ok(out)
}

/// A pairing record exists for any spelling of these UDIDs: in usbmuxd (Apple's own, written when
/// the user taps Trust for the Apple Devices app / iTunes) or in pymobiledevice3's folder (written
/// by `swiftwin pair`). pymobiledevice3 uses either one, in that order.
fn has_pair_record(mux: &Usbmux, udids: &[&str]) -> bool {
    let all: Vec<String> = udids.iter().flat_map(|u| udid_variants(u)).collect();
    all.iter().any(|u| mux.read_pair_record(u).map(|r| r.is_some()).unwrap_or(false)) || pmd::has_local_pair_record(&all)
}

fn connection_of(d: &MuxDevice) -> Connection {
    if d.connection_type.eq_ignore_ascii_case("network") {
        Connection::Network
    } else {
        Connection::Usb
    }
}

/// How to reach the phone; tests swap in fakes.
pub struct RealOptions {
    pub mux: Usbmux,
    /// pymobiledevice3 runner; `None` = locate the real executable lazily.
    pub runner: Option<Arc<dyn Runner>>,
}

impl Default for RealOptions {
    fn default() -> Self {
        RealOptions { mux: Usbmux::platform(), runner: None }
    }
}

pub struct RealDevice {
    mux: Usbmux,
    dev: MuxDevice,
    /// UDID in lockdown form (with dash), used for pymobiledevice3 `--udid`.
    udid: String,
    runner: Mutex<Option<Arc<dyn Runner>>>,
    logs: Arc<LogBuffer>,
    /// Background `syslog live` capture; ends when this device is dropped (e.g. the app reconnects).
    log_capture: Arc<LogCapture>,
    ddi_mounted: AtomicBool,
    forwards: Mutex<HashMap<u16, u16>>,
}

impl RealDevice {
    /// Open the USB phone with `udid`, or the only/first USB phone.
    pub fn open(udid: Option<&str>, opts: RealOptions) -> Result<RealDevice> {
        // A phone with Wi-Fi sync on is listed twice (USB and Network): prefer USB.
        let mut devices = opts.mux.list_devices()?;
        devices.sort_by_key(|d| !d.connection_type.eq_ignore_ascii_case("usb"));
        let dev = match udid {
            Some(u) => devices
                .into_iter()
                .find(|d| same_udid(&d.udid, u))
                .ok_or_else(|| SwiftWinError::DeviceNotFound(u.to_string()))?,
            None => devices.into_iter().next().ok_or(SwiftWinError::NoDevice)?,
        };
        let mut udid = dev.udid.clone();
        if let Ok(mut l) = opts.mux.connect(dev.device_id, lockdown::LOCKDOWN_PORT).map(lockdown::Lockdown::new) {
            if let Some(u) = l.get_string("UniqueDeviceID") {
                udid = u;
            }
        }
        Ok(RealDevice {
            mux: opts.mux,
            dev,
            udid,
            runner: Mutex::new(opts.runner),
            // ~8 s of DEBUG at a real phone's ~2,600 lines/s; other levels keep a minute or more
            // even while WDA queries the UI (each query adds hundreds of INFO/NOTICE lines).
            logs: Arc::new(LogBuffer::with_caps(20_000, 60_000)),
            log_capture: Arc::new(LogCapture::new(Duration::from_secs(20), Duration::from_secs(3))),
            ddi_mounted: AtomicBool::new(false),
            forwards: Mutex::new(HashMap::new()),
        })
    }

    pub fn udid(&self) -> &str {
        &self.udid
    }

    pub fn is_paired(&self) -> bool {
        // usbmuxd's serial and lockdown's UniqueDeviceID may be spelled differently; try both.
        has_pair_record(&self.mux, &[self.dev.udid.as_str(), self.udid.as_str()])
    }

    pub fn pmd(&self) -> Result<Pmd> {
        let mut g = self.runner.lock().unwrap_or_else(|p| p.into_inner());
        if g.is_none() {
            *g = Some(Arc::new(ProcessRunner::locate()?));
        }
        Ok(Pmd::new(g.clone().unwrap(), self.udid.clone()))
    }

    fn require_paired(&self) -> Result<()> {
        if self.is_paired() {
            Ok(())
        } else {
            Err(SwiftWinError::NotTrusted(format!("no pairing record for {}", self.udid)))
        }
    }

    /// Mount the developer disk image once per session (needed for screenshots, launch, XCTest).
    pub fn ensure_developer_services(&self) -> Result<()> {
        self.require_paired()?;
        if self.ddi_mounted.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.pmd()?.mount_developer_image()?;
        self.ddi_mounted.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Start background syslog capture into the ring buffer (idempotent). A supervisor thread
    /// restarts `syslog live` when it ends or goes silent: a phone sends thousands of lines a
    /// second, so silence means the stream stalled (seen on iOS 27.0.1: `logs` kept returning the
    /// same old lines and the app's own notices never arrived).
    pub fn start_log_capture(&self) -> Result<()> {
        if self.log_capture.started.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        // The first start reports its error to the caller; later restarts only to the log.
        let started = self.pmd().and_then(|p| p.syslog_stream().map(|s| (p, s)));
        let (pmd, first) = match started {
            Ok(x) => x,
            Err(e) => {
                self.log_capture.started.store(false, Ordering::SeqCst);
                return Err(e);
            }
        };
        let (cap, logs) = (self.log_capture.clone(), self.logs.clone());
        std::thread::Builder::new().name("syslog-supervisor".into()).spawn(move || cap.supervise(pmd, logs, first))?;
        Ok(())
    }
}

/// Background `syslog live` capture of one device, shared with its supervisor thread.
struct LogCapture {
    started: AtomicBool,
    /// Set when the device is dropped: the supervisor ends.
    stop: AtomicBool,
    killer: Mutex<Option<Killer>>,
    /// Unix milliseconds of the last line (0: none yet).
    last_line_ms: AtomicU64,
    restarts: AtomicU64,
    problem: Mutex<Option<String>>,
    /// No line for this long = stalled.
    stall_after: Duration,
    /// Wait this long before a restart.
    retry_after: Duration,
}

impl LogCapture {
    fn new(stall_after: Duration, retry_after: Duration) -> Self {
        LogCapture {
            started: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            killer: Mutex::new(None),
            last_line_ms: AtomicU64::new(0),
            restarts: AtomicU64::new(0),
            problem: Mutex::new(None),
            stall_after,
            retry_after,
        }
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
    }

    /// Record a problem and show it in the log itself, so the app's log panel shows it too.
    fn report(&self, logs: &LogBuffer, msg: String) {
        let now = swiftwin_core::time::now_rfc3339();
        *self.problem.lock().unwrap_or_else(|p| p.into_inner()) = Some(format!("{msg} (at {now})"));
        logs.push(LogEntry { timestamp: now, process: "swiftwin".into(), pid: 0, level: LogLevel::Error, message: msg, subsystem: None });
    }

    fn supervise(self: Arc<Self>, pmd: Pmd, logs: Arc<LogBuffer>, first: Box<dyn LineStream>) {
        let mut next = Some(first);
        while !self.stop.load(Ordering::SeqCst) {
            let mut s = match next.take().map(Ok).unwrap_or_else(|| pmd.syslog_stream()) {
                Ok(s) => s,
                Err(e) => {
                    self.report(&logs, format!("could not restart the device log stream: {e}"));
                    std::thread::sleep(self.retry_after.max(Duration::from_secs(5)));
                    continue;
                }
            };
            *self.killer.lock().unwrap_or_else(|p| p.into_inner()) = Some(s.killer());
            self.last_line_ms.store(Self::now_ms(), Ordering::SeqCst);
            let ended = Arc::new(AtomicBool::new(false));
            let (me, l2, e2) = (self.clone(), logs.clone(), ended.clone());
            let reader = std::thread::Builder::new().name("syslog".into()).spawn(move || {
                while let Some(line) = s.next_line() {
                    me.last_line_ms.store(Self::now_ms(), Ordering::SeqCst);
                    match pmd::parse_syslog_line(&line) {
                        Some(e) => l2.push(e),
                        // stderr is merged in: pymobiledevice3's own errors explain a dead stream.
                        None if !line.trim().is_empty() => me.report(&l2, format!("syslog: {}", line.trim())),
                        None => {}
                    }
                }
                e2.store(true, Ordering::SeqCst);
            });
            if reader.is_err() {
                return;
            }
            let why = loop {
                std::thread::sleep(Duration::from_millis(250));
                if self.stop.load(Ordering::SeqCst) {
                    return;
                }
                if ended.load(Ordering::SeqCst) {
                    break "ended".to_string();
                }
                let silent = Self::now_ms().saturating_sub(self.last_line_ms.load(Ordering::SeqCst));
                if silent >= self.stall_after.as_millis() as u64 {
                    break format!("stalled (no line for {} s)", silent / 1000);
                }
            };
            if let Some(k) = self.killer.lock().unwrap_or_else(|p| p.into_inner()).take() {
                k();
            }
            self.restarts.fetch_add(1, Ordering::SeqCst);
            self.report(&logs, format!("device log stream {why}: restarting it; lines in between are lost"));
            std::thread::sleep(self.retry_after);
        }
    }

    fn end(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(kill) = self.killer.lock().unwrap_or_else(|p| p.into_inner()).take() {
            kill();
        }
    }
}

impl Drop for RealDevice {
    /// Stop the `syslog live` process: nobody can read this device's buffer any more.
    fn drop(&mut self) {
        self.log_capture.end();
    }
}

impl Device for RealDevice {
    fn info(&self) -> Result<DeviceInfo> {
        let paired = self.is_paired();
        let mut name = String::from("iPhone");
        let mut model = String::new();
        let mut version = String::new();
        if let Ok(s) = self.mux.connect(self.dev.device_id, lockdown::LOCKDOWN_PORT) {
            let mut l = lockdown::Lockdown::new(s);
            model = l.get_string("ProductType").unwrap_or_default();
            version = l.get_string("ProductVersion").unwrap_or_default();
            if let Some(n) = l.get_string("DeviceName") {
                name = n;
            }
        }
        let mut developer_mode = None;
        if paired {
            if let Ok(p) = self.pmd() {
                if let Ok(v) = p.lockdown_info() {
                    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
                    name = s("DeviceName").unwrap_or(name);
                    model = s("ProductType").unwrap_or(model);
                    version = s("ProductVersion").unwrap_or(version);
                }
                developer_mode = p.developer_mode_status().ok();
            }
        }
        Ok(DeviceInfo {
            udid: self.udid.clone(),
            name,
            screen: models::screen_for(&model),
            model,
            ios_version: version,
            connection: connection_of(&self.dev),
            paired,
            developer_mode,
        })
    }

    fn list_apps(&self) -> Result<Vec<AppInfo>> {
        self.require_paired()?;
        self.pmd()?.list_apps()
    }

    fn install(&self, package: &Path) -> Result<AppInfo> {
        let info = crate::bundle::read_app_info(package)?;
        self.require_paired()?;
        self.pmd()?.install(package)?;
        Ok(info)
    }

    // UNTESTED-ON-DEVICE
    fn uninstall(&self, bundle_id: &str) -> Result<()> {
        self.require_paired()?;
        self.pmd()?.uninstall(bundle_id)
    }

    fn launch(&self, bundle_id: &str, opts: &LaunchOptions) -> Result<u32> {
        self.ensure_developer_services()?;
        let _ = self.start_log_capture();
        self.pmd()?.launch(bundle_id, &opts.args, &opts.env, opts.kill_existing)
    }

    // UNTESTED-ON-DEVICE
    fn terminate(&self, bundle_id: &str) -> Result<()> {
        self.ensure_developer_services()?;
        let p = self.pmd()?;
        match p.pid_for_bundle(bundle_id)? {
            Some(pid) => p.kill(pid),
            None => Err(SwiftWinError::NotFound(format!("{bundle_id} is not running"))),
        }
    }

    fn screenshot(&self) -> Result<Screenshot> {
        self.ensure_developer_services()?;
        let path = std::env::temp_dir().join(format!("swiftwin-shot-{}-{}.png", std::process::id(), unique()));
        self.pmd()?.screenshot(&path)?;
        let data = std::fs::read(&path);
        let _ = std::fs::remove_file(&path);
        Screenshot::from_png(data?)
    }

    fn syslog(&self, query: &LogQuery, sink: &mut dyn FnMut(LogEntry) -> bool) -> Result<()> {
        self.require_paired()?;
        self.start_log_capture()?;
        if self.logs.is_empty() && !query.follow {
            // Capture just started: give the stream a moment to deliver something.
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
        self.logs.read(query, sink)
    }

    fn keep_logs(&self) -> Result<()> {
        self.require_paired()?;
        self.start_log_capture()
    }

    fn log_status(&self) -> Option<crate::LogStatus> {
        let c = &self.log_capture;
        let last = c.last_line_ms.load(Ordering::SeqCst);
        Some(crate::LogStatus {
            next: self.logs.next_seq(),
            silent_secs: (last > 0).then(|| LogCapture::now_ms().saturating_sub(last) as f64 / 1000.0),
            restarts: c.restarts.load(Ordering::SeqCst),
            problem: c.problem.lock().unwrap_or_else(|p| p.into_inner()).clone(),
        })
    }

    // UNTESTED-ON-DEVICE
    fn crash_reports(&self, out_dir: &Path) -> Result<Vec<PathBuf>> {
        self.require_paired()?;
        std::fs::create_dir_all(out_dir)?;
        self.pmd()?.crash_pull(out_dir)?;
        let mut files = Vec::new();
        collect_files(out_dir, &mut files)?;
        files.sort();
        Ok(files)
    }

    fn start_xctest(&self, runner_bundle_id: &str, env: &[(String, String)]) -> Result<Box<dyn RunningProcess>> {
        self.ensure_developer_services()?;
        let mut stream = self.pmd()?.xcuitest(runner_bundle_id, env)?;
        let killer = stream.killer();
        let running = Arc::new(AtomicBool::new(true));
        let (logs, flag, name) = (self.logs.clone(), running.clone(), runner_bundle_id.to_string());
        std::thread::Builder::new().name("xctest".into()).spawn(move || {
            while let Some(line) = stream.next_line() {
                logs.push(LogEntry {
                    timestamp: swiftwin_core::time::now_rfc3339(),
                    process: format!("xctest:{name}"),
                    pid: 0,
                    level: LogLevel::Info,
                    message: line,
                    subsystem: None,
                });
            }
            flag.store(false, Ordering::SeqCst);
        })?;
        Ok(Box::new(XcTestProcess { running, killer }))
    }

    fn forward_port(&self, device_port: u16) -> Result<u16> {
        let mut f = self.forwards.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(p) = f.get(&device_port) {
            return Ok(*p);
        }
        let local = usbmux::spawn_forwarder(self.mux.clone(), self.dev.device_id, device_port)?;
        f.insert(device_port, local);
        Ok(local)
    }
}

struct XcTestProcess {
    running: Arc<AtomicBool>,
    killer: Killer,
}

impl RunningProcess for XcTestProcess {
    fn is_running(&mut self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
    fn stop(&mut self) -> Result<()> {
        (self.killer)();
        Ok(())
    }
}

fn unique() -> u64 {
    use std::sync::atomic::AtomicU64;
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::SeqCst)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            collect_files(&p, out)?;
        } else {
            out.push(p);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::pmd::fake::FakeRunner;
    use super::usbmux::{fake, MuxAddr};
    use super::*;

    const UDID: &str = "00008110-000A1B2C3D4E5F60";

    fn open(paired: bool) -> (RealDevice, Arc<FakeRunner>) {
        let f = fake::start(UDID, paired);
        let runner = Arc::new(FakeRunner::default());
        let d = RealDevice::open(
            None,
            RealOptions { mux: Usbmux::new(MuxAddr::Tcp(f.addr)), runner: Some(runner.clone()) },
        )
        .unwrap();
        (d, runner)
    }

    #[test]
    fn info_from_lockdown_when_unpaired() {
        let (d, runner) = open(false);
        assert_eq!(d.udid(), UDID, "UDID should come from lockdown (with dash)");
        let i = d.info().unwrap();
        assert!(!i.paired);
        assert_eq!(i.ios_version, "17.5.1");
        assert_eq!(i.model, "iPhone15,2");
        assert_eq!(i.screen.unwrap().width_pts, 393.0);
        assert!(runner.calls().is_empty(), "unpaired info must not need pymobiledevice3");
        assert!(matches!(d.screenshot(), Err(SwiftWinError::NotTrusted(_))));
    }

    #[test]
    fn paired_info_uses_pymobiledevice3() {
        let (d, runner) = open(true);
        runner.reply("lockdown info", true, r#"{"DeviceName":"Ada's iPhone","ProductType":"iPhone15,2","ProductVersion":"17.5.1"}"#, "");
        runner.reply("developer-mode-status", true, "true\n", "");
        let i = d.info().unwrap();
        assert!(i.paired);
        assert_eq!(i.name, "Ada's iPhone");
        assert_eq!(i.developer_mode, Some(true));
    }

    #[test]
    fn screenshot_mounts_ddi_once() {
        let (d, runner) = open(true);
        let s = d.screenshot().unwrap();
        assert_eq!((s.width, s.height), (30, 60));
        d.screenshot().unwrap();
        let mounts = runner.calls().iter().filter(|c| c.contains(&"auto-mount".to_string())).count();
        assert_eq!(mounts, 1);
    }

    #[test]
    fn syslog_and_crashes() {
        let (d, runner) = open(true);
        *runner.stream_lines.lock().unwrap() = vec![
            r#"{"pid": 5, "timestamp": "t", "level": "INFO", "filename": "/usr/libexec/backboardd", "message": "hello"}"#.into(),
            r#"{"pid": 6, "timestamp": "t", "level": "ERROR", "filename": "/x/Hello.app/Hello", "message": "boom"}"#.into(),
        ];
        let logs = crate::collect_logs(&d, &LogQuery { process: Some("Hello".into()), ..Default::default() }).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].message, "boom");
        let dir = std::env::temp_dir().join(format!("swiftwin-real-crash-{}", std::process::id()));
        let files = d.crash_reports(&dir).unwrap();
        assert_eq!(files.len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    /// `keep_logs` starts the syslog stream, so later reads include earlier lines.
    #[test]
    fn keep_logs_collects_before_the_first_read() {
        let (d, runner) = open(true);
        *runner.stream_lines.lock().unwrap() = vec![
            r#"{"pid": 6, "timestamp": "t", "level": "NOTICE", "filename": "/x/Hello.app/Hello", "message": "Button tapped"}"#.into(),
        ];
        d.keep_logs().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(runner.calls().iter().any(|c| c.contains(&"syslog".to_string())));
        let logs = crate::collect_logs(&d, &LogQuery { process: Some("Hello".into()), ..Default::default() }).unwrap();
        assert_eq!(logs[0].message, "Button tapped");
        let (u, _) = open(false);
        assert!(matches!(u.keep_logs(), Err(SwiftWinError::NotTrusted(_))));
    }

    /// A reconnect drops the old device: its `syslog live` process must end with it.
    #[test]
    fn dropping_the_device_ends_its_log_stream() {
        let (d, runner) = open(true);
        runner.stream_hold.store(true, Ordering::SeqCst);
        d.keep_logs().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(runner.killed.load(Ordering::SeqCst), 0);
        drop(d);
        assert_eq!(runner.killed.load(Ordering::SeqCst), 1);
    }

    /// A stream that goes silent is restarted, and the restart shows in the log and the status.
    #[test]
    fn a_stalled_log_stream_is_restarted() {
        let (mut d, runner) = open(true);
        d.log_capture = Arc::new(LogCapture::new(Duration::from_millis(300), Duration::from_millis(50)));
        runner.stream_hold.store(true, Ordering::SeqCst);
        *runner.stream_lines.lock().unwrap() = vec![
            r#"{"pid": 6, "timestamp": "t", "level": "NOTICE", "filename": "/x/Hello.app/Hello", "message": "hi"}"#.into(),
        ];
        d.keep_logs().unwrap();
        std::thread::sleep(Duration::from_millis(1000));
        let syslogs = runner.calls().iter().filter(|c| c.contains(&"syslog".to_string())).count();
        assert!(syslogs >= 2, "restarted: {syslogs}");
        let st = d.log_status().unwrap();
        assert!(st.restarts >= 1);
        assert!(st.problem.unwrap().contains("stalled"));
        let all = crate::collect_logs(&d, &LogQuery::default()).unwrap();
        assert!(all.iter().any(|e| e.process == "swiftwin" && e.message.contains("restarting")));
    }

    #[test]
    fn launch_and_terminate() {
        let (d, runner) = open(true);
        runner.reply("dvt launch", true, "Process launched with pid 77\n", "");
        runner.reply("process-id-for-bundle-id", true, "77\n", "");
        assert_eq!(d.launch("com.x", &LaunchOptions::default()).unwrap(), 77);
        d.terminate("com.x").unwrap();
        assert!(runner.calls().iter().any(|c| c[1..4] == ["developer", "dvt", "kill"]));
    }

    #[test]
    fn missing_device_errors() {
        let f = fake::start(UDID, true);
        let opts = RealOptions { mux: Usbmux::new(MuxAddr::Tcp(f.addr)), runner: None };
        assert!(matches!(RealDevice::open(Some("nope"), opts), Err(SwiftWinError::DeviceNotFound(_))));
    }

    #[test]
    fn listing() {
        let f = fake::start(UDID, false);
        let l = list_devices(&Usbmux::new(MuxAddr::Tcp(f.addr))).unwrap();
        assert_eq!(l.len(), 1);
        assert!(!l[0].paired);
        assert_eq!(l[0].connection, Connection::Usb);
    }

    #[test]
    fn listing_finds_the_dashed_pair_record_for_an_undashed_serial() {
        // The fake reports the serial without the dash and, like Windows, only answers
        // ReadPairRecord for the dashed form.
        let f = fake::start(UDID, true);
        let l = list_devices(&Usbmux::new(MuxAddr::Tcp(f.addr))).unwrap();
        assert!(!l[0].udid.contains('-'));
        assert!(l[0].paired);
    }
}
