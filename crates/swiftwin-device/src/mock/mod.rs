//! `MockDevice`: a simulated iPhone.
//!
//! It keeps a small amount of state (installed apps, which screen is showing,
//! a counter, a text field, a scrollable list, logs, crash reports) and derives
//! both the rendered PNG screenshot and the accessibility tree from one layout,
//! so taps on what you *see* change what the UI tree *says*, just like a phone.
//!
//! Screen: 393 x 852 points (iPhone 15 / 16 size), rendered at scale 1.

pub mod canvas;
pub mod font;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};


use canvas::{Canvas, Rgb};
use swiftwin_core::{
    time, AppInfo, Button, Connection, DeviceInfo, LogEntry, LogLevel, LogQuery, Point, Rect, Result, ScreenSpec,
    Screenshot, SwiftWinError, UiNode,
};

use crate::{Controller, Device, LaunchOptions, RunningProcess};

pub const MOCK_UDID: &str = "00008130-00MOCK0000000001";
pub const SCREEN_W: f64 = 393.0;
pub const SCREEN_H: f64 = 852.0;
/// Bundle id of the demo app that comes pre-installed on the mock phone.
pub const DEMO_APP: &str = "dev.swiftwin.hello";
/// A third-party-style app with a tab bar (dark theme), for Inspo captures.
pub const TUNES_APP: &str = "fm.tunes.player";
/// Tab titles of the Tunes app.
pub const TUNES_TABS: [&str; 3] = ["Home", "Search", "Library"];
const TAB_TOP: f64 = SCREEN_H - 83.0;

const LIST_TOP: f64 = 440.0;
const LIST_BOTTOM: f64 = SCREEN_H - 40.0;
const ROW_H: f64 = 44.0;
const ROWS: usize = 40;

#[derive(Debug, Clone, PartialEq)]
enum Screen {
    Home,
    App(String),
}

#[derive(Debug, Clone)]
struct MockApp {
    info: AppInfo,
    system: bool,
}

#[derive(Debug, Clone, Default)]
struct AppState {
    count: u32,
    name: String,
    scroll: f64,
    tab: usize,
    /// Tunes: the row whose detail screen is open.
    detail: Option<String>,
}

#[derive(Debug)]
struct State {
    apps: Vec<MockApp>,
    screen: Screen,
    app_state: HashMap<String, AppState>,
    focused: bool,
    pids: HashMap<String, u32>,
    next_pid: u32,
    logs: Arc<crate::logbuf::LogBuffer>,
    crashes: Vec<(String, String)>,
    xctest_running: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Action {
    None,
    Launch(String),
    Increment,
    Focus,
    Crash,
    Tab(usize),
    /// Tunes' "Buy Premium": logs a purchase (tests check Inspo never taps it).
    Purchase,
    /// Tunes: open a row's detail screen, go back, or delete (tests check Inspo never deletes).
    Open(String),
    Back,
    Delete,
}

#[derive(Debug, Clone)]
struct Element {
    kind: &'static str,
    label: String,
    value: Option<String>,
    rect: Rect,
    action: Action,
}

/// A simulated iPhone. Cheap to clone (shared state).
#[derive(Clone)]
pub struct MockDevice {
    inner: Arc<(Mutex<State>, Condvar)>,
}

impl Default for MockDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl MockDevice {
    pub fn new() -> Self {
        let sys = |id: &str, name: &str| MockApp {
            info: AppInfo { bundle_id: id.into(), name: name.into(), version: Some("26.1".into()) },
            system: true,
        };
        let mut st = State {
            apps: vec![
                sys("com.apple.Preferences", "Settings"),
                sys("com.apple.mobilesafari", "Safari"),
                sys("com.apple.camera", "Camera"),
                MockApp {
                    info: AppInfo { bundle_id: DEMO_APP.into(), name: "Hello".into(), version: Some("1.0".into()) },
                    system: false,
                },
                MockApp {
                    info: AppInfo { bundle_id: TUNES_APP.into(), name: "Tunes".into(), version: Some("4.2".into()) },
                    system: false,
                },
            ],
            screen: Screen::Home,
            app_state: HashMap::new(),
            focused: false,
            pids: HashMap::new(),
            next_pid: 400,
            logs: Arc::new(crate::logbuf::LogBuffer::new(5000)),
            crashes: Vec::new(),
            xctest_running: false,
        };
        log(&mut st, "kernel", 0, LogLevel::Notice, "Mock iPhone booted (SwiftWin MockDevice)");
        log(&mut st, "SpringBoard", 58, LogLevel::Info, "Home screen ready");
        MockDevice { inner: Arc::new((Mutex::new(st), Condvar::new())) }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn notify(&self) {
        self.inner.1.notify_all();
    }

    /// Bundle id of the app in the foreground (`None` on the home screen).
    pub fn foreground_app(&self) -> Option<String> {
        match &self.state().screen {
            Screen::Home => None,
            Screen::App(id) => Some(id.clone()),
        }
    }

    /// Append a log line as if an app printed it (useful in tests and demos).
    pub fn inject_log(&self, process: &str, level: LogLevel, message: &str) {
        {
            let mut st = self.state();
            log(&mut st, process, 1000, level, message);
        }
        self.notify();
    }

    pub fn screen_spec() -> ScreenSpec {
        ScreenSpec { width_pts: SCREEN_W, height_pts: SCREEN_H, scale: 1.0 }
    }
}

fn log(st: &mut State, process: &str, pid: u32, level: LogLevel, message: &str) {
    log_sub(st, process, pid, level, message, None)
}

fn log_sub(st: &mut State, process: &str, pid: u32, level: LogLevel, message: &str, subsystem: Option<String>) {
    st.logs.push(LogEntry {
        timestamp: time::now_rfc3339(),
        process: process.to_string(),
        pid,
        level,
        message: message.to_string(),
        subsystem,
    });
}

fn app_name(st: &State, id: &str) -> String {
    st.apps.iter().find(|a| a.info.bundle_id == id).map(|a| a.info.name.clone()).unwrap_or_else(|| id.to_string())
}

fn launch_locked(st: &mut State, id: &str) -> Result<u32> {
    if !st.apps.iter().any(|a| a.info.bundle_id == id) {
        return Err(SwiftWinError::NotFound(format!("app {id} is not installed")));
    }
    let pid = match st.pids.get(id) {
        Some(p) => *p,
        None => {
            st.next_pid += 1;
            let p = st.next_pid;
            st.pids.insert(id.to_string(), p);
            p
        }
    };
    st.screen = Screen::App(id.to_string());
    st.focused = false;
    log(st, "SpringBoard", 58, LogLevel::Info, &format!("Launching {id} (pid {pid})"));
    let name = app_name(st, id);
    log(st, &name, pid, LogLevel::Info, &format!("{name} did finish launching"));
    Ok(pid)
}

/// The single source of truth for what is on screen.
fn layout(st: &State) -> Vec<Element> {
    let mut els = Vec::new();
    match &st.screen {
        Screen::Home => {
            let (cols, icon, gap_x, top) = (4usize, 62.0, 30.0, 90.0);
            let left = (SCREEN_W - (cols as f64 * icon + (cols - 1) as f64 * gap_x)) / 2.0;
            for (i, a) in st.apps.iter().enumerate() {
                let (r, c) = (i / cols, i % cols);
                let x = left + c as f64 * (icon + gap_x);
                let y = top + r as f64 * (icon + 40.0);
                els.push(Element {
                    kind: "XCUIElementTypeIcon",
                    label: a.info.name.clone(),
                    value: None,
                    rect: Rect::new(x, y, icon, icon + 22.0),
                    action: Action::Launch(a.info.bundle_id.clone()),
                });
            }
        }
        Screen::App(id) if id == TUNES_APP => tunes_layout(st.app_state.get(id).cloned().unwrap_or_default(), &mut els),
        Screen::App(id) => {
            let name = app_name(st, id);
            let s = st.app_state.get(id).cloned().unwrap_or_default();
            let text = |label: String, rect: Rect| Element {
                kind: "XCUIElementTypeStaticText",
                label,
                value: None,
                rect,
                action: Action::None,
            };
            els.push(text(name, Rect::new(20.0, 60.0, 353.0, 40.0)));
            els.push(text(format!("Count: {}", s.count), Rect::new(20.0, 120.0, 353.0, 30.0)));
            els.push(Element {
                kind: "XCUIElementTypeButton",
                label: "Tap me".into(),
                value: None,
                rect: Rect::new(20.0, 165.0, 170.0, 50.0),
                action: Action::Increment,
            });
            els.push(Element {
                kind: "XCUIElementTypeButton",
                label: "Crash".into(),
                value: None,
                rect: Rect::new(203.0, 165.0, 170.0, 50.0),
                action: Action::Crash,
            });
            els.push(Element {
                kind: "XCUIElementTypeTextField",
                label: "Your name".into(),
                value: Some(if s.name.is_empty() { "Your name".into() } else { s.name.clone() }),
                rect: Rect::new(20.0, 235.0, 353.0, 44.0),
                action: Action::Focus,
            });
            let greeting = if s.name.is_empty() { "Hello, SwiftWin!".to_string() } else { format!("Hello, {}!", s.name) };
            els.push(text(greeting, Rect::new(20.0, 295.0, 353.0, 30.0)));
            els.push(text("Items".into(), Rect::new(20.0, 400.0, 353.0, 30.0)));
            for i in 0..ROWS {
                let y = LIST_TOP + i as f64 * ROW_H - s.scroll;
                if y + ROW_H <= LIST_TOP || y >= LIST_BOTTOM {
                    continue;
                }
                els.push(Element {
                    kind: "XCUIElementTypeCell",
                    label: format!("Item {}", i + 1),
                    value: None,
                    rect: Rect::new(0.0, y, SCREEN_W, ROW_H),
                    action: Action::None,
                });
            }
        }
    }
    els
}

fn tunes_layout(s: AppState, els: &mut Vec<Element>) {
    let el = |kind, label: String, rect, action| Element { kind, label, value: None, rect, action };
    if let Some(d) = s.detail {
        els.push(el("XCUIElementTypeButton", "Back".into(), Rect::new(8.0, 54.0, 70.0, 32.0), Action::Back));
        els.push(el("XCUIElementTypeStaticText", d, Rect::new(20.0, 100.0, 353.0, 41.0), Action::None));
        els.push(el("XCUIElementTypeButton", "Delete Playlist".into(), Rect::new(20.0, 160.0, 353.0, 50.0), Action::Delete));
        return;
    }
    els.push(el("XCUIElementTypeStaticText", TUNES_TABS[s.tab].into(), Rect::new(20.0, 60.0, 353.0, 41.0), Action::None));
    els.push(el("XCUIElementTypeButton", "Buy Premium".into(), Rect::new(20.0, 120.0, 353.0, 50.0), Action::Purchase));
    let noun = ["Song", "Genre", "Playlist"][s.tab];
    for i in 0..ROWS {
        let y = 190.0 + i as f64 * 64.0 - s.scroll;
        if y + 56.0 <= 190.0 || y >= TAB_TOP {
            continue;
        }
        let label = format!("{noun} {}", i + 1);
        els.push(el("XCUIElementTypeCell", label.clone(), Rect::new(20.0, y, 353.0, 56.0), Action::Open(label)));
    }
    els.push(el("XCUIElementTypeTabBar", "Tab Bar".into(), Rect::new(0.0, TAB_TOP, SCREEN_W, 83.0), Action::None));
    for (i, t) in TUNES_TABS.iter().enumerate() {
        let w = SCREEN_W / 3.0;
        els.push(el("XCUIElementTypeButton", t.to_string(), Rect::new(i as f64 * w, TAB_TOP, w, 49.0), Action::Tab(i)));
    }
}

fn render_tunes(st: &State) -> Canvas {
    let (bg, surface, accent, fg) = (Rgb(18, 18, 24), Rgb(40, 40, 52), Rgb(255, 90, 95), Rgb(255, 255, 255));
    let mut c = Canvas::new(SCREEN_W as u32, SCREEN_H as u32, bg);
    c.draw_text(30.0, 18.0, "9:41", 2, fg);
    let tab = st.app_state.get(TUNES_APP).map(|s| s.tab).unwrap_or(0);
    for e in layout(st) {
        let r = e.rect;
        match (e.kind, &e.action) {
            ("XCUIElementTypeStaticText", _) => c.draw_text(r.x, r.y + 8.0, &e.label, 3, fg),
            (_, Action::Purchase | Action::Delete) => {
                c.fill_round_rect(r.x, r.y, r.width, r.height, 12.0, accent);
                c.draw_text_centered(r.x, r.y, r.width, r.height, &e.label, 2, fg);
            }
            ("XCUIElementTypeCell", _) => {
                c.fill_round_rect(r.x, r.y.max(190.0), r.width, (r.y + r.height).min(TAB_TOP) - r.y.max(190.0), 12.0, surface);
                if r.y + 20.0 >= 190.0 && r.y + 36.0 <= TAB_TOP {
                    c.draw_text(r.x + 16.0, r.y + 20.0, &e.label, 2, fg);
                }
            }
            (_, Action::Back) => c.draw_text(r.x + 4.0, r.y + 8.0, "< Back", 2, accent),
            ("XCUIElementTypeTabBar", _) => c.fill_rect(r.x, r.y, r.width, r.height, Rgb(28, 28, 36)),
            (_, Action::Tab(i)) => {
                let col = if *i == tab { accent } else { Rgb(142, 142, 147) };
                c.draw_text_centered(r.x, r.y, r.width, r.height, &e.label, 2, col);
            }
            _ => {}
        }
    }
    c.fill_round_rect(SCREEN_W / 2.0 - 67.0, SCREEN_H - 13.0, 134.0, 5.0, 2.5, fg);
    c
}

fn render(st: &State) -> Canvas {
    if st.screen == Screen::App(TUNES_APP.into()) {
        return render_tunes(st);
    }
    let bg = match st.screen {
        Screen::Home => Rgb(36, 52, 92),
        Screen::App(_) => Rgb(255, 255, 255),
    };
    let fg = match st.screen {
        Screen::Home => Rgb(255, 255, 255),
        Screen::App(_) => Rgb(0, 0, 0),
    };
    let mut c = Canvas::new(SCREEN_W as u32, SCREEN_H as u32, bg);
    c.draw_text(30.0, 18.0, "9:41", 2, fg);
    c.draw_text(SCREEN_W - 30.0 - Canvas::text_width("MOCK", 2), 18.0, "MOCK", 2, fg);
    let palette = [Rgb(142, 142, 147), Rgb(0, 122, 255), Rgb(60, 60, 67), Rgb(255, 149, 0), Rgb(52, 199, 89)];
    for (i, e) in layout(st).iter().enumerate() {
        let r = e.rect;
        match e.kind {
            "XCUIElementTypeIcon" => {
                c.fill_round_rect(r.x, r.y, r.width, r.width, 14.0, palette[i % palette.len()]);
                let initial: String = e.label.chars().take(1).collect();
                c.draw_text_centered(r.x, r.y, r.width, r.width, &initial, 4, Rgb(255, 255, 255));
                let label: String = e.label.chars().take(8).collect();
                c.draw_text_centered(r.x - 15.0, r.y + r.width + 4.0, r.width + 30.0, 16.0, &label, 1, fg);
            }
            "XCUIElementTypeButton" => {
                let col = if e.action == Action::Crash { Rgb(255, 59, 48) } else { Rgb(0, 122, 255) };
                c.fill_round_rect(r.x, r.y, r.width, r.height, 12.0, col);
                c.draw_text_centered(r.x, r.y, r.width, r.height, &e.label, 2, Rgb(255, 255, 255));
            }
            "XCUIElementTypeTextField" => {
                c.fill_round_rect(r.x, r.y, r.width, r.height, 8.0, Rgb(242, 242, 247));
                if st.focused {
                    c.stroke_rect(r.x, r.y, r.width, r.height, Rgb(0, 122, 255));
                }
                let v = e.value.clone().unwrap_or_default();
                let col = if v == e.label { Rgb(142, 142, 147) } else { Rgb(0, 0, 0) };
                c.draw_text(r.x + 12.0, r.y + (r.height - 14.0) / 2.0, &v, 2, col);
            }
            "XCUIElementTypeCell" => {
                if r.y >= LIST_TOP && r.y + r.height <= LIST_BOTTOM {
                    c.fill_rect(r.x + 20.0, r.y + r.height - 1.0, r.width - 20.0, 1.0, Rgb(220, 220, 225));
                }
                if r.y + 10.0 >= LIST_TOP && r.y + 24.0 <= LIST_BOTTOM {
                    c.draw_text(r.x + 20.0, r.y + 15.0, &e.label, 2, fg);
                }
            }
            _ => {
                let scale = if i == 0 { 3 } else { 2 };
                c.draw_text(r.x, r.y + (r.height - Canvas::text_height(scale)) / 2.0, &e.label, scale, fg);
            }
        }
    }
    // Home indicator.
    c.fill_round_rect(SCREEN_W / 2.0 - 67.0, SCREEN_H - 13.0, 134.0, 5.0, 2.5, fg);
    c
}

impl Device for MockDevice {
    fn info(&self) -> Result<DeviceInfo> {
        Ok(DeviceInfo {
            udid: MOCK_UDID.into(),
            name: "Mock iPhone".into(),
            model: "iPhone16,1".into(),
            ios_version: "26.1".into(),
            connection: Connection::Mock,
            paired: true,
            developer_mode: Some(true),
            screen: Some(Self::screen_spec()),
        })
    }

    fn list_apps(&self) -> Result<Vec<AppInfo>> {
        Ok(self.state().apps.iter().map(|a| a.info.clone()).collect())
    }

    fn install(&self, package: &Path) -> Result<AppInfo> {
        let info = crate::bundle::read_app_info(package)?;
        {
            let mut st = self.state();
            st.apps.retain(|a| a.info.bundle_id != info.bundle_id);
            st.apps.push(MockApp { info: info.clone(), system: false });
            st.pids.remove(&info.bundle_id);
            st.app_state.remove(&info.bundle_id);
            if st.screen == Screen::App(info.bundle_id.clone()) {
                st.screen = Screen::Home;
            }
            log(&mut st, "installd", 120, LogLevel::Info, &format!("Installed {} ({})", info.bundle_id, info.name));
        }
        self.notify();
        Ok(info)
    }

    fn uninstall(&self, bundle_id: &str) -> Result<()> {
        {
            let mut st = self.state();
            let before = st.apps.len();
            if st.apps.iter().any(|a| a.info.bundle_id == bundle_id && a.system) {
                return Err(SwiftWinError::invalid(format!("{bundle_id} is a system app")));
            }
            st.apps.retain(|a| a.info.bundle_id != bundle_id);
            if st.apps.len() == before {
                return Err(SwiftWinError::NotFound(format!("app {bundle_id} is not installed")));
            }
            st.pids.remove(bundle_id);
            if st.screen == Screen::App(bundle_id.into()) {
                st.screen = Screen::Home;
            }
            log(&mut st, "installd", 120, LogLevel::Info, &format!("Uninstalled {bundle_id}"));
        }
        self.notify();
        Ok(())
    }

    fn launch(&self, bundle_id: &str, opts: &LaunchOptions) -> Result<u32> {
        let pid = {
            let mut st = self.state();
            if opts.kill_existing {
                st.pids.remove(bundle_id);
            }
            launch_locked(&mut st, bundle_id)?
        };
        self.notify();
        Ok(pid)
    }

    fn terminate(&self, bundle_id: &str) -> Result<()> {
        {
            let mut st = self.state();
            if st.pids.remove(bundle_id).is_none() {
                return Err(SwiftWinError::NotFound(format!("{bundle_id} is not running")));
            }
            if st.screen == Screen::App(bundle_id.into()) {
                st.screen = Screen::Home;
            }
            log(&mut st, "SpringBoard", 58, LogLevel::Info, &format!("Terminated {bundle_id}"));
        }
        self.notify();
        Ok(())
    }

    fn screenshot(&self) -> Result<Screenshot> {
        let png = render(&self.state()).to_png();
        Screenshot::from_png(png)
    }

    fn syslog(&self, query: &LogQuery, sink: &mut dyn FnMut(LogEntry) -> bool) -> Result<()> {
        let logs = self.state().logs.clone();
        logs.read(query, sink)
    }

    fn log_status(&self) -> Option<crate::LogStatus> {
        Some(crate::LogStatus { next: self.state().logs.next_seq(), ..Default::default() })
    }

    fn crash_reports(&self, out_dir: &Path) -> Result<Vec<PathBuf>> {
        std::fs::create_dir_all(out_dir)?;
        let crashes = self.state().crashes.clone();
        let mut out = Vec::new();
        for (name, body) in crashes {
            let p = out_dir.join(name);
            std::fs::write(&p, body)?;
            out.push(p);
        }
        Ok(out)
    }

    fn start_xctest(&self, runner_bundle_id: &str, _env: &[(String, String)]) -> Result<Box<dyn RunningProcess>> {
        {
            let mut st = self.state();
            st.xctest_running = true;
            log(&mut st, "testmanagerd", 77, LogLevel::Info, &format!("Started XCTest runner {runner_bundle_id}"));
        }
        Ok(Box::new(MockProcess { dev: self.clone() }))
    }

    fn forward_port(&self, _device_port: u16) -> Result<u16> {
        Err(SwiftWinError::Unsupported("the mock device has no TCP ports; use its Controller directly".into()))
    }
}

struct MockProcess {
    dev: MockDevice,
}

impl RunningProcess for MockProcess {
    fn is_running(&mut self) -> bool {
        self.dev.state().xctest_running
    }
    fn stop(&mut self) -> Result<()> {
        self.dev.state().xctest_running = false;
        Ok(())
    }
}

impl Controller for MockDevice {
    fn window_size(&self) -> Result<(f64, f64)> {
        Ok((SCREEN_W, SCREEN_H))
    }

    fn tap(&self, at: Point) -> Result<()> {
        if !(0.0..=SCREEN_W).contains(&at.x) || !(0.0..=SCREEN_H).contains(&at.y) {
            return Err(SwiftWinError::invalid(format!("tap ({}, {}) is outside the screen", at.x, at.y)));
        }
        {
            let mut st = self.state();
            let hit = layout(&st).into_iter().rev().find(|e| e.rect.contains(at) && e.action != Action::None);
            let app = match &st.screen {
                Screen::App(id) => Some(id.clone()),
                Screen::Home => None,
            };
            match hit.map(|e| e.action) {
                Some(Action::Launch(id)) => {
                    launch_locked(&mut st, &id)?;
                }
                Some(Action::Increment) => {
                    let id = app.unwrap_or_default();
                    let name = app_name(&st, &id);
                    let pid = st.pids.get(&id).copied().unwrap_or(0);
                    let s = st.app_state.entry(id.clone()).or_default();
                    s.count += 1;
                    let msg = format!("Button tapped, count = {}", s.count);
                    st.focused = false;
                    // Like the app's own `Logger(subsystem: <bundle id>, ...)`.
                    log_sub(&mut st, &name, pid, LogLevel::Info, &msg, Some(id.clone()));
                }
                Some(Action::Focus) => st.focused = true,
                Some(Action::Tab(i)) => {
                    let s = st.app_state.entry(app.unwrap_or_default()).or_default();
                    (s.tab, s.scroll, s.detail) = (i, 0.0, None);
                }
                Some(Action::Open(label)) => st.app_state.entry(app.unwrap_or_default()).or_default().detail = Some(label),
                Some(Action::Back) => st.app_state.entry(app.unwrap_or_default()).or_default().detail = None,
                Some(Action::Delete) => log(&mut st, "Tunes", 0, LogLevel::Notice, "Deleted a playlist"),
                Some(Action::Purchase) => log(&mut st, "Tunes", 0, LogLevel::Notice, "Purchase started: Premium"),
                Some(Action::Crash) => {
                    let id = app.unwrap_or_default();
                    let name = app_name(&st, &id);
                    let pid = st.pids.remove(&id).unwrap_or(0);
                    log(&mut st, &name, pid, LogLevel::Fault, "Fatal error: user tapped Crash");
                    log(&mut st, "ReportCrash", 90, LogLevel::Error, &format!("Saved crash report for {name}[{pid}]"));
                    let n = st.crashes.len() + 1;
                    let body = format!(
                        "{{\"app_name\":\"{name}\",\"bundleID\":\"{id}\",\"exception\":{{\"type\":\"EXC_BREAKPOINT\",\"signal\":\"SIGTRAP\"}},\"timestamp\":\"{}\"}}\nThread 0 Crashed:\n0   {name}  0x0000000100004000 ContentView.crash() + 40\n",
                        time::now_rfc3339()
                    );
                    st.crashes.push((format!("{name}-{n}.ips"), body));
                    st.screen = Screen::Home;
                    st.focused = false;
                }
                _ => st.focused = false,
            }
        }
        self.notify();
        Ok(())
    }

    fn swipe(&self, from: Point, to: Point, _duration_s: f64) -> Result<()> {
        {
            let mut st = self.state();
            let dy = to.y - from.y;
            match st.screen.clone() {
                Screen::App(_) if from.y > SCREEN_H - 60.0 && dy < -100.0 => {
                    st.screen = Screen::Home; // swipe up from the bottom edge = go home
                    log(&mut st, "SpringBoard", 58, LogLevel::Info, "Home gesture");
                }
                Screen::App(id) => {
                    let max = if id == TUNES_APP {
                        ROWS as f64 * 64.0 - (TAB_TOP - 190.0)
                    } else {
                        (ROWS as f64 * ROW_H - (LIST_BOTTOM - LIST_TOP)).max(0.0)
                    };
                    let s = st.app_state.entry(id).or_default();
                    s.scroll = (s.scroll - dy).clamp(0.0, max);
                }
                Screen::Home => {}
            }
        }
        self.notify();
        Ok(())
    }

    fn type_text(&self, text: &str) -> Result<()> {
        {
            let mut st = self.state();
            let id = match (&st.screen, st.focused) {
                (Screen::App(id), true) => id.clone(),
                _ => return Err(SwiftWinError::invalid("no text field has keyboard focus; tap one first")),
            };
            let s = st.app_state.entry(id).or_default();
            for ch in text.chars() {
                match ch {
                    '\u{8}' => {
                        s.name.pop();
                    }
                    '\n' => {}
                    c => s.name.push(c),
                }
            }
            if text.contains('\n') {
                st.focused = false;
            }
        }
        self.notify();
        Ok(())
    }

    fn press_button(&self, button: Button) -> Result<()> {
        {
            let mut st = self.state();
            if button == Button::Home {
                st.screen = Screen::Home;
                st.focused = false;
            }
            log(&mut st, "SpringBoard", 58, LogLevel::Info, &format!("Button pressed: {}", button.wda_name()));
        }
        self.notify();
        Ok(())
    }

    fn focused(&self) -> Result<Option<UiNode>> {
        let st = self.state();
        if !st.focused {
            return Ok(None);
        }
        Ok(layout(&st).into_iter().find(|e| e.kind == "XCUIElementTypeTextField").map(|e| UiNode::new(e.kind, e.rect).with_label(e.label)))
    }

    fn set_appearance(&self, dark: bool) -> Result<()> {
        log(&mut self.state(), "SpringBoard", 58, LogLevel::Info, if dark { "Appearance: dark" } else { "Appearance: light" });
        Ok(())
    }

    fn ui_tree(&self) -> Result<UiNode> {
        let st = self.state();
        let title = match &st.screen {
            Screen::Home => "SpringBoard".to_string(),
            Screen::App(id) => app_name(&st, id),
        };
        let children = layout(&st)
            .into_iter()
            .map(|e| {
                let mut n = UiNode::new(e.kind, e.rect).with_label(e.label);
                n.value = e.value;
                n
            })
            .collect();
        Ok(UiNode::new("XCUIElementTypeApplication", Rect::new(0.0, 0.0, SCREEN_W, SCREEN_H))
            .with_label(title)
            .with_children(children))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect_logs;

    fn tap_label(d: &MockDevice, label: &str) {
        let tree = d.ui_tree().unwrap();
        let n = tree.find(label).unwrap_or_else(|| panic!("no {label} in\n{}", tree.to_text()));
        d.tap(n.rect.center()).unwrap();
    }

    #[test]
    fn home_screen_lists_apps() {
        let d = MockDevice::new();
        let t = d.ui_tree().unwrap();
        assert_eq!(t.label.as_deref(), Some("SpringBoard"));
        assert!(t.find("Hello").is_some());
        assert!(t.find("Settings").is_some());
    }

    #[test]
    fn tap_icon_then_button_increments() {
        let d = MockDevice::new();
        tap_label(&d, "Hello");
        assert_eq!(d.foreground_app().as_deref(), Some(DEMO_APP));
        tap_label(&d, "Tap me");
        tap_label(&d, "Tap me");
        assert!(d.ui_tree().unwrap().find("Count: 2").is_some());
        let logs = collect_logs(&d, &LogQuery { process: Some("Hello".into()), ..Default::default() }).unwrap();
        assert!(logs.iter().any(|l| l.message == "Button tapped, count = 2"));
    }

    #[test]
    fn typing_requires_focus() {
        let d = MockDevice::new();
        d.launch(DEMO_APP, &LaunchOptions::default()).unwrap();
        assert!(d.type_text("x").is_err());
        tap_label(&d, "Your name");
        d.type_text("Ada").unwrap();
        let t = d.ui_tree().unwrap();
        assert!(t.find("Hello, Ada!").is_some());
        assert_eq!(t.find("Your name").unwrap().value.as_deref(), Some("Ada"));
    }

    #[test]
    fn swipe_scrolls_list_and_home_gesture() {
        let d = MockDevice::new();
        d.launch(DEMO_APP, &LaunchOptions::default()).unwrap();
        assert!(d.ui_tree().unwrap().find("Item 1").is_some());
        d.swipe(Point { x: 200.0, y: 700.0 }, Point { x: 200.0, y: 450.0 }, 0.2).unwrap();
        let t = d.ui_tree().unwrap();
        assert!(t.find("Item 1").is_none(), "{}", t.to_text());
        assert!(t.find("Item 12").is_some());
        d.swipe(Point { x: 200.0, y: 840.0 }, Point { x: 200.0, y: 500.0 }, 0.2).unwrap();
        assert_eq!(d.foreground_app(), None);
    }

    #[test]
    fn screenshot_changes_with_state() {
        let d = MockDevice::new();
        let a = d.screenshot().unwrap();
        assert_eq!((a.width, a.height), (393, 852));
        d.launch(DEMO_APP, &LaunchOptions::default()).unwrap();
        let b = d.screenshot().unwrap();
        assert_ne!(a.png, b.png);
    }

    #[test]
    fn crash_creates_report_and_returns_home() {
        let d = MockDevice::new();
        d.launch(DEMO_APP, &LaunchOptions::default()).unwrap();
        tap_label(&d, "Crash");
        assert_eq!(d.foreground_app(), None);
        let dir = std::env::temp_dir().join(format!("swiftwin-crash-{}", std::process::id()));
        let files = d.crash_reports(&dir).unwrap();
        assert_eq!(files.len(), 1);
        assert!(std::fs::read_to_string(&files[0]).unwrap().contains("EXC_BREAKPOINT"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn install_and_uninstall() {
        let d = MockDevice::new();
        let dir = std::env::temp_dir().join(format!("swiftwin-inst-{}", std::process::id()));
        let app = dir.join("Foo.app");
        std::fs::create_dir_all(&app).unwrap();
        let mut dict = plist::Dictionary::new();
        dict.insert("CFBundleIdentifier".into(), "com.example.foo".into());
        dict.insert("CFBundleName".into(), "Foo".into());
        plist::Value::Dictionary(dict).to_file_xml(app.join("Info.plist")).unwrap();
        let info = d.install(&app).unwrap();
        assert_eq!(info.bundle_id, "com.example.foo");
        assert!(d.ui_tree().unwrap().find("Foo").is_some());
        d.launch("com.example.foo", &LaunchOptions::default()).unwrap();
        d.uninstall("com.example.foo").unwrap();
        assert!(d.launch("com.example.foo", &LaunchOptions::default()).is_err());
        assert!(d.uninstall("com.apple.Preferences").is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn follow_logs_receives_new_entries() {
        let d = MockDevice::new();
        let d2 = d.clone();
        let h = std::thread::spawn(move || {
            let mut got = Vec::new();
            d2.syslog(&LogQuery { follow: true, contains: Some("ping".into()), ..Default::default() }, &mut |e| {
                got.push(e.message);
                got.len() < 2
            })
            .unwrap();
            got
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        d.inject_log("Test", LogLevel::Info, "ping 1");
        d.inject_log("Test", LogLevel::Info, "other");
        d.inject_log("Test", LogLevel::Info, "ping 2");
        assert_eq!(h.join().unwrap(), vec!["ping 1", "ping 2"]);
    }

    #[test]
    fn home_button_and_bounds() {
        let d = MockDevice::new();
        d.launch(DEMO_APP, &LaunchOptions::default()).unwrap();
        d.press_button(Button::Home).unwrap();
        assert_eq!(d.foreground_app(), None);
        assert!(d.tap(Point { x: -1.0, y: 5.0 }).is_err());
    }
}
