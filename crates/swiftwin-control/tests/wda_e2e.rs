//! WebDriverAgent client + controller against the fake WDA server backed by MockDevice.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use swiftwin_control::fakewda::FakeWda;
use swiftwin_control::wda::WdaClient;
use swiftwin_control::{Phone, WdaController};
use swiftwin_core::{AppInfo, Button, DeviceInfo, LogEntry, LogQuery, Point, Result, Screenshot, SwiftWinError};
use swiftwin_device::{Controller, Device, LaunchOptions, MockDevice, RunningProcess};

fn fake() -> (MockDevice, FakeWda) {
    let m = MockDevice::new();
    let f = FakeWda::start(Arc::new(m.clone()), Arc::new(m.clone())).unwrap();
    (m, f)
}

#[test]
fn client_drives_mock_over_http() {
    let (m, f) = fake();
    let c = WdaClient::new(f.url());
    assert!(c.is_ready());
    assert_eq!(c.window_size().unwrap(), (393.0, 852.0));
    let tree = c.ui_tree().unwrap();
    c.tap(tree.find("Hello").unwrap().rect.center()).unwrap();
    assert_eq!(m.foreground_app().as_deref(), Some("dev.swiftwin.hello"));
    let tree = c.ui_tree().unwrap();
    c.tap(tree.find("Tap me").unwrap().rect.center()).unwrap();
    c.tap(tree.find("Your name").unwrap().rect.center()).unwrap();
    c.type_text("Zoë").unwrap();
    let tree = c.ui_tree().unwrap();
    assert!(tree.find("Count: 1").is_some());
    assert!(tree.find("Hello, Zoë!").is_some(), "{}", tree.to_text());
    c.swipe(Point { x: 200.0, y: 700.0 }, Point { x: 200.0, y: 450.0 }, 0.3).unwrap();
    assert!(c.ui_tree().unwrap().find("Item 1").is_none());
    let shot = c.screenshot().unwrap();
    assert_eq!((shot.width, shot.height), (393, 852));
    c.press_button(Button::Home).unwrap();
    assert_eq!(m.foreground_app(), None);
    let reqs = f.requests();
    assert!(reqs.iter().any(|r| r.starts_with("POST /session/FAKE-1/wda/tap")));
    assert!(reqs.iter().any(|r| r == "POST /wda/homescreen"));
    assert!(reqs.iter().any(|r| r.starts_with("GET /source?format=json&excluded_attributes=visible,")));
}

#[test]
fn session_is_recreated_after_expiry() {
    let (_m, f) = fake();
    let c = WdaClient::new(f.url());
    c.window_size().unwrap();
    f.expire_session();
    c.window_size().unwrap();
    assert!(f.requests().iter().any(|r| r.starts_with("GET /session/FAKE-2/")));
}

#[test]
fn errors_surface_wda_message() {
    let (_m, f) = fake();
    let c = WdaClient::new(f.url());
    let e = c.type_text("x").unwrap_err().to_string();
    assert!(e.contains("focus"), "{e}");
    let down = WdaClient::new("http://127.0.0.1:1");
    assert!(!down.is_ready());
    assert!(matches!(down.ui_tree(), Err(SwiftWinError::Http(_))));
}

/// A MockDevice whose "phone port 8100" is a fake WDA started by `start_xctest`.
struct WdaPhone {
    mock: MockDevice,
    port: u16,
    installed: bool,
    server: Mutex<Option<FakeWda>>,
    starts: Mutex<u32>,
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

struct Proc;
impl RunningProcess for Proc {
    fn is_running(&mut self) -> bool {
        true
    }
    fn stop(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Device for WdaPhone {
    fn info(&self) -> Result<DeviceInfo> {
        self.mock.info()
    }
    fn list_apps(&self) -> Result<Vec<AppInfo>> {
        let mut a = self.mock.list_apps()?;
        if self.installed {
            a.push(AppInfo { bundle_id: "com.test.WebDriverAgentRunner.xctrunner".into(), name: "WDA".into(), version: None });
        }
        Ok(a)
    }
    fn install(&self, p: &Path) -> Result<AppInfo> {
        self.mock.install(p)
    }
    fn uninstall(&self, b: &str) -> Result<()> {
        self.mock.uninstall(b)
    }
    fn launch(&self, b: &str, o: &LaunchOptions) -> Result<u32> {
        self.mock.launch(b, o)
    }
    fn terminate(&self, b: &str) -> Result<()> {
        self.mock.terminate(b)
    }
    fn screenshot(&self) -> Result<Screenshot> {
        self.mock.screenshot()
    }
    fn syslog(&self, q: &LogQuery, s: &mut dyn FnMut(LogEntry) -> bool) -> Result<()> {
        self.mock.syslog(q, s)
    }
    fn crash_reports(&self, d: &Path) -> Result<Vec<PathBuf>> {
        self.mock.crash_reports(d)
    }
    fn start_xctest(&self, id: &str, env: &[(String, String)]) -> Result<Box<dyn RunningProcess>> {
        assert_eq!(id, "com.test.WebDriverAgentRunner.xctrunner");
        assert!(env.iter().any(|(k, v)| k == "USE_PORT" && v == "8100"));
        *self.starts.lock().unwrap() += 1;
        let m = self.mock.clone();
        let port = self.port;
        // Simulate WDA taking a moment to boot.
        let srv = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            FakeWda::start_on(port, Arc::new(m.clone()), Arc::new(m)).unwrap()
        });
        *self.server.lock().unwrap() = Some(srv.join().unwrap());
        Ok(Box::new(Proc))
    }
    fn forward_port(&self, device_port: u16) -> Result<u16> {
        match device_port {
            8100 => Ok(self.port),
            9100 => Ok(4242), // the MJPEG stream (not served by the fake)
            p => panic!("unexpected device port {p}"),
        }
    }
}

#[test]
fn controller_starts_wda_and_drives_phone() {
    let dev = Arc::new(WdaPhone {
        mock: MockDevice::new(),
        port: free_port(),
        installed: true,
        server: Mutex::new(None),
        starts: Mutex::new(0),
    });
    let phone = Phone::from_device(dev.clone(), "com.test.WebDriverAgentRunner.xctrunner");
    let tree = phone.control.ui_tree().unwrap();
    phone.control.tap(tree.find("Hello").unwrap().rect.center()).unwrap();
    assert_eq!(dev.mock.foreground_app().as_deref(), Some("dev.swiftwin.hello"));
    phone.control.press_button(Button::Home).unwrap();
    let s = phone.screenshot().unwrap();
    assert_eq!(s.width, 393);
    assert_eq!(*dev.starts.lock().unwrap(), 1, "WDA must be started once and reused");
}

#[test]
fn video_stream_sets_the_frame_rate_and_forwards_the_stream_port() {
    let dev = Arc::new(WdaPhone {
        mock: MockDevice::new(),
        port: free_port(),
        installed: true,
        server: Mutex::new(None),
        starts: Mutex::new(0),
    });
    let phone = Phone::from_device(dev.clone(), "com.test.WebDriverAgentRunner.xctrunner");
    assert_eq!(phone.video_stream(30).unwrap(), "127.0.0.1:4242");
    let settings = dev.server.lock().unwrap().as_ref().unwrap().settings();
    assert_eq!(settings["mjpegServerFramerate"], 30);
    assert_eq!(settings["mjpegScalingFactor"], 100);
    assert_eq!(settings["mjpegServerScreenshotQuality"], 25);
    assert!(matches!(Phone::mock().video_stream(30), Err(SwiftWinError::Unsupported(_))));
}

#[test]
fn alerts_are_seen_and_their_buttons_tapped() {
    let m = MockDevice::new();
    let f = FakeWda::start(Arc::new(m.clone()), Arc::new(m)).unwrap();
    let c = WdaClient::new(f.url());
    assert_eq!(c.alert().unwrap(), None, "no alert open");
    f.show_alert("Reset all data?", &["Cancel", "Reset"]);
    let a = c.alert().unwrap().unwrap();
    assert_eq!((a.text.as_str(), a.buttons.len()), ("Reset all data?", 2));
    assert!(c.alert_tap("Nope").is_err());
    c.alert_tap("Reset").unwrap();
    assert_eq!(f.alert_taps(), vec!["Reset"]);
    assert_eq!(c.alert().unwrap(), None, "the alert closed");
}

#[test]
fn controller_reports_missing_wda() {
    let dev = Arc::new(WdaPhone {
        mock: MockDevice::new(),
        port: free_port(),
        installed: false,
        server: Mutex::new(None),
        starts: Mutex::new(0),
    });
    let c = WdaController::new(dev, "com.test.WebDriverAgentRunner.xctrunner");
    let e = c.ui_tree().unwrap_err().to_string();
    assert!(e.contains("swiftwin wda prepare") && e.contains("swiftwin wda use"), "{e}");
}

#[test]
fn controller_uses_a_runner_installed_under_another_bundle_id() {
    // E.g. signed by Sideloadly with the user's own bundle id, while the config still has the default.
    let dev = Arc::new(WdaPhone {
        mock: MockDevice::new(),
        port: free_port(),
        installed: true,
        server: Mutex::new(None),
        starts: Mutex::new(0),
    });
    let c = WdaController::new(dev.clone(), swiftwin_core::config::DEFAULT_WDA_BUNDLE_ID);
    assert!(c.is_installed().unwrap());
    assert_eq!(c.bundle_id(), "com.test.WebDriverAgentRunner.xctrunner");
    c.ui_tree().unwrap();
    assert_eq!(*dev.starts.lock().unwrap(), 1);
}
