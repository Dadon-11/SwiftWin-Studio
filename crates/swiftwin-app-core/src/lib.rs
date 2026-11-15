//! The desktop app's backend. Every UI action is `AppCore::dispatch(cmd, args)`
//! returning JSON, so the Tauri shell needs a single command and the same
//! code can be served over local HTTP for browser testing (`swiftwin ui`).

pub mod account;
pub mod bridge;
pub mod http;
pub mod setup;
pub mod signing;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use swiftwin_control::{connect, ConnectOptions, Phone};
use swiftwin_core::{Button, DeviceInfo, LogEntry, LogQuery, Point, Result, SwiftWinError};
use swiftwin_mirror::{Feed, Mirror, Mode, Policy, SourceKind, Video};
use swiftwin_workflow::Workspace;

/// Frame rate asked of the phone's video stream (WDA's MJPEG server); ~23 fps arrive.
const VIDEO_FPS: u32 = 30;

struct Session {
    ws: Arc<Workspace>,
    mirror: Mirror,
    info: DeviceInfo,
    /// Ends this session's log follower.
    logs_stop: Arc<AtomicBool>,
}

impl Session {
    /// Stop mirroring and following this phone's logs (a reconnect starts new ones).
    fn end(&self) {
        self.mirror.stop();
        self.logs_stop.store(true, Ordering::SeqCst);
    }
}

pub struct AppCore {
    session: Mutex<Option<Session>>,
    logs: Arc<Mutex<Vec<LogEntry>>>,
    project: Mutex<Option<PathBuf>>,
    /// What the AI agent (Claude Code, Codex…) did through the MCP bridge (shown in AI mode).
    activity: Mutex<Vec<Value>>,
    /// The tool the agent is running right now (`{"tool", "since"}`), so AI mode can show that
    /// it is busy during a long build instead of only after it.
    busy: Mutex<Option<Value>>,
    /// Sign-in, GitHub connection and Upload (UI only; the bridge refuses these commands).
    account: account::AccountUi,
    /// Signing setup (free Apple ID / App Store Connect key; UI only, like `account`).
    signing: signing::SigningUi,
    /// First-run setup: installs prerequisites, gets the iOS SDK (UI only, like `account`).
    pub setup: setup::SetupUi,
    /// Secret for the local HTTP endpoint (bridge + browser UI).
    token: String,
    /// Inspo capture progress (`{running, message, screens, error, result}`) and its cancel flag.
    inspo: Arc<Mutex<Value>>,
    inspo_cancel: Arc<AtomicBool>,
    /// WebDriverAgent install progress (`{running, step, error, result}`).
    wda_job: Arc<Mutex<Value>>,
    /// Force the mock phone (SWIFTWIN_MOCK=1 or --mock).
    pub force_mock: bool,
    /// Brings the SwiftWin Studio window forward on a page ("", "account", "signing", "inspo"); set by
    /// the desktop app. Only focuses a window: the phone page on the bridge uses it for what the bridge
    /// refuses itself (account, signing), so nothing private is served there.
    pub studio_opener: OnceLock<Box<dyn Fn(&str) + Send + Sync>>,
}

impl AppCore {
    pub fn new(force_mock: bool) -> Self {
        use rand::Rng;
        let token: String = (0..32).map(|_| format!("{:x}", rand::thread_rng().gen_range(0..16u8))).collect();
        AppCore {
            session: Mutex::new(None),
            logs: Arc::new(Mutex::new(Vec::new())),
            project: Mutex::new(None),
            activity: Mutex::new(Vec::new()),
            busy: Mutex::new(None),
            account: account::AccountUi::new(swiftwin_core::mock_requested(force_mock)),
            signing: signing::SigningUi::new(swiftwin_core::mock_requested(force_mock)),
            setup: setup::SetupUi::new(swiftwin_core::mock_requested(force_mock)),
            token,
            inspo: Arc::new(Mutex::new(json!({"running": false}))),
            inspo_cancel: Arc::new(AtomicBool::new(false)),
            wda_job: Arc::new(Mutex::new(json!({"running": false}))),
            force_mock: swiftwin_core::mock_requested(force_mock),
            studio_opener: OnceLock::new(),
        }
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn dispatch(&self, cmd: &str, args: &Value) -> Result<Value> {
        let s = |k: &str| args[k].as_str().map(str::to_string);
        let f = |k: &str| args[k].as_f64().ok_or_else(|| SwiftWinError::invalid(format!("missing number `{k}`")));
        match cmd {
            "scan" => self.scan(),
            "connect" => self.connect(args["mock"].as_bool().unwrap_or(false), s("udid")),
            "disconnect" => {
                if let Some(sess) = self.session.lock().unwrap().take() {
                    sess.end();
                }
                Ok(json!({}))
            }
            "status" => self.status(),
            "frame" => self.frame(args["after"].as_u64().unwrap_or(0), args["wait_ms"].as_u64().unwrap_or(800)),
            "tap" => {
                let phone = self.phone()?;
                let p = self.to_points(&phone, f("fx")?, f("fy")?)?;
                phone.control.tap(p)?;
                Ok(json!({"x": p.x, "y": p.y}))
            }
            "swipe" => {
                let phone = self.phone()?;
                let a = self.to_points(&phone, f("fx1")?, f("fy1")?)?;
                let b = self.to_points(&phone, f("fx2")?, f("fy2")?)?;
                phone.control.swipe(a, b, args["duration"].as_f64().unwrap_or(0.3))?;
                Ok(json!({}))
            }
            "type" => {
                self.phone()?.control.type_text(&s("text").unwrap_or_default())?;
                Ok(json!({}))
            }
            "press" => {
                let b: Button = s("button").unwrap_or_else(|| "home".into()).parse()?;
                self.phone()?.control.press_button(b)?;
                Ok(json!({}))
            }
            // WebDriverAgent, the helper app that lets SwiftWin control the phone.
            "wda_status" => {
                let phone = self.phone()?;
                let installed = match &phone.wda {
                    Some(w) => w.is_installed()?,
                    None => true, // the simulated iPhone needs none
                };
                Ok(json!({"installed": installed, "mock": phone.is_mock}))
            }
            "wda_install" => self.wda_install(),
            "wda_job" => Ok(self.wda_job.lock().unwrap().clone()),
            // Start it (window size needs a running WebDriverAgent).
            "wda_check" => Ok(match self.phone()?.control.window_size() {
                Ok(_) => json!({"ok": true}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            }),
            "ui_tree" => Ok(json!({"text": self.phone()?.control.ui_tree()?.to_text()})),
            "logs_since" => {
                let from = args["index"].as_u64().unwrap_or(0) as usize;
                let logs = self.logs.lock().unwrap();
                let from = from.min(logs.len());
                Ok(json!({"next": logs.len(), "entries": logs[from..].iter().rev().take(500).rev().collect::<Vec<_>>()}))
            }
            "set_mirror_mode" => {
                let mode = match s("mode").as_deref() {
                    Some("usb") => Mode::UsbOnly,
                    Some("video" | "airplay") => Mode::VideoOnly,
                    _ => Mode::Auto,
                };
                self.with_session(|sess| sess.mirror.set_mode(mode))?;
                Ok(json!({}))
            }
            "open_project" => self.open_project(Path::new(&s("path").unwrap_or_default())),
            "new_project" => {
                let parent = PathBuf::from(s("parent").unwrap_or_else(|| ".".into()));
                let dir = swiftwin_builder::scaffold::new_project(&parent, &s("name").unwrap_or_default(), s("bundle_id").as_deref())?;
                self.open_project(&dir)
            }
            "read_file" => {
                let p = self.project_path(&s("path").unwrap_or_default())?;
                Ok(json!({"path": s("path"), "text": std::fs::read_to_string(p)?}))
            }
            "write_file" => {
                let p = self.project_path(&s("path").unwrap_or_default())?;
                swiftwin_core::paths::write_atomic(&p, s("text").unwrap_or_default().as_bytes())?;
                Ok(json!({}))
            }
            "build" => {
                let (ws, dir) = (self.workspace()?, self.project_dir()?);
                let out = ws.build(&dir, false)?;
                Ok(json!({"message": format!("Built {}", out.app.display()), "warnings": out.warnings}))
            }
            "run" => {
                let (ws, dir) = (self.workspace()?, self.project_dir()?);
                let r = ws.install_run(&dir, false)?;
                Ok(json!({"message": format!("Launched {} (pid {})", r.bundle_id, r.pid), "warnings": r.warnings}))
            }
            "doctor" => {
                let mock = self.force_mock || self.with_session(|s| s.ws.is_mock()).unwrap_or(false);
                let r = if mock {
                    swiftwin_doctor::run(&swiftwin_doctor::env::MockEnv::new())
                } else {
                    swiftwin_doctor::run(&swiftwin_doctor::env::RealEnv::default())
                };
                Ok(json!({"ok": r.ok(), "text": r.to_text(), "checks": r.checks}))
            }
            "mcp_tool" => self.mcp_tool(&s("tool").unwrap_or_default(), &args["args"], args["mock"].as_bool().unwrap_or(false), s("project")),
            "activity_since" => {
                let from = args["index"].as_u64().unwrap_or(0) as usize;
                let a = self.activity.lock().unwrap();
                let mut busy = self.busy.lock().unwrap().clone();
                // A long build / install_run says which step it is at ("Compiling Tempo…").
                if let (Some(b), Ok(Some(step))) = (busy.as_mut(), self.workspace().map(|w| w.step())) {
                    b["step"] = json!(step);
                }
                Ok(json!({"next": a.len(), "entries": a[from.min(a.len())..].to_vec(), "busy": busy}))
            }
            "show_studio" => {
                let page = s("page").unwrap_or_default();
                if !["", "account", "signing", "inspo"].contains(&page.as_str()) {
                    return Err(SwiftWinError::invalid(format!("unknown page `{page}`")));
                }
                let opened = self.studio_opener.get().map(|open| open(&page)).is_some();
                Ok(json!({"opened": opened}))
            }
            "bridge_info" => Ok(json!({
                "mcp_command": "claude mcp add swiftwin -- swiftwin mcp",
                "bridge_file": bridge::path(),
            })),
            "inspo_apps" => Ok(json!({"apps": swiftwin_inspo::user_apps(&self.phone()?)?})),
            "inspo_start" => self.inspo_start(&s("bundle_id").unwrap_or_default(), args["deep"].as_bool().unwrap_or(false)),
            "inspo_status" => Ok(self.inspo.lock().unwrap().clone()),
            "inspo_cancel" => {
                self.inspo_cancel.store(true, Ordering::SeqCst);
                Ok(json!({}))
            }
            "inspo_image" => {
                // A small copy of a captured screen for the dialog's grid.
                let png = std::fs::read(self.project_path(&s("path").unwrap_or_default())?)?;
                let img = swiftwin_builder::assets::Image::decode(&png)?;
                let w = img.w.min(240);
                let small = img.resize(w, (img.h as u64 * w as u64 / img.w.max(1) as u64).max(1) as u32).encode_rgb();
                Ok(json!({"data": swiftwin_core::base64_bytes::encode(&small)}))
            }
            "inspo_add_theme" => {
                let from = self.project_path(&format!("{}/InspoTheme.swift", s("dir").unwrap_or_default()))?;
                let project = swiftwin_builder::Project::load(&self.project_dir()?)?;
                let to = project.sources_dir().join("InspoTheme.swift");
                std::fs::copy(from, &to)?;
                let rel = to.strip_prefix(&project.dir).unwrap_or(&to).to_string_lossy().replace('\\', "/");
                Ok(json!({"path": rel}))
            }
            c if c.starts_with("signing_") => self.signing.dispatch(c, args),
            c if c.starts_with("setup_") => self.setup.dispatch(c, args),
            c if account::is_private(c) => self.account.dispatch(c, args, self.project.lock().unwrap().clone()),
            other => Err(SwiftWinError::invalid(format!("unknown command {other}"))),
        }
    }

    /// Run an MCP tool for `swiftwin mcp` (Claude) on the app's phone and record it.
    fn mcp_tool(&self, tool: &str, args: &Value, caller_mock: bool, caller_project: Option<String>) -> Result<Value> {
        let Some(ws) = self.session.lock().unwrap().as_ref().map(|s| s.ws.clone()) else {
            return Ok(json!({"forwarded": false, "reason": "no iPhone connected in the SwiftWin app"}));
        };
        if ws.is_mock() != caller_mock {
            return Ok(json!({"forwarded": false, "reason": "the app and swiftwin mcp use different phones (mock vs real)"}));
        }
        let project = args["project"]
            .as_str()
            .map(PathBuf::from)
            .or_else(|| self.project.lock().unwrap().clone())
            .or_else(|| caller_project.map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        let mut server = swiftwin_mcp::Server::with_workspace(ws.clone(), project.clone());
        *self.busy.lock().unwrap() = Some(json!({"tool": tool, "since": swiftwin_core::time::now_rfc3339()}));
        let result = swiftwin_mcp::tools::call(&mut server, tool, args);
        *self.busy.lock().unwrap() = None;
        let ok = result["isError"] != json!(true);
        if ok && matches!(tool, "build" | "install_run") && self.project.lock().unwrap().is_none() {
            let _ = self.open_project(&project);
        }
        self.record(tool, args, &result, ok, &ws);
        Ok(json!({"forwarded": true, "result": result}))
    }

    fn record(&self, tool: &str, args: &Value, result: &Value, ok: bool, ws: &Workspace) {
        let text: String = result["content"]
            .as_array()
            .map(|c| c.iter().filter_map(|x| x["text"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        let first = text.lines().next().unwrap_or("").trim_start_matches("Error: ").to_string();
        let short = |s: &str, n: usize| if s.chars().count() > n { format!("{}…", s.chars().take(n).collect::<String>()) } else { s.to_string() };
        let summary = if !ok {
            format!("{tool} failed: {}", short(&first, 120))
        } else {
            match tool {
                "tap" => match args["label"].as_str() {
                    Some(l) => format!("Tapped “{}”", short(l, 40)),
                    None => first.clone(),
                },
                "swipe" => match args["direction"].as_str() {
                    Some(d) => format!("Swiped {d}"),
                    None => "Swiped".into(),
                },
                "type" => format!("Typed “{}”", short(args["text"].as_str().unwrap_or(""), 40)),
                "screenshot" => "Looked at the screen".into(),
                "ui_tree" => "Read the screen's elements".into(),
                "logs" => "Read the device logs".into(),
                "build" => "Built the app".into(),
                "install_run" => short(first.trim_end_matches(" Take a screenshot to see it."), 80),
                "press_button" => format!("Pressed {}", args["button"].as_str().unwrap_or("a button")),
                "device_info" => "Checked the device".into(),
                "launch_app" => "Opened the app".into(),
                "terminate_app" => "Closed the app".into(),
                "release" => "Started a TestFlight release".into(),
                "inspo_capture" => "Captured design inspiration from another app".into(),
                _ => tool.to_string(),
            }
        };
        let nums = numbers(&first);
        let (w, h) = ws.phone.control.window_size().unwrap_or((1.0, 1.0));
        let frac = |i: usize, d: f64| nums.get(i).map(|v| v / d);
        let (fx, fy, fx2, fy2) = match (ok, tool) {
            (true, "tap") => (frac(0, w), frac(1, h), None, None),
            (true, "swipe") => (frac(0, w), frac(1, h), frac(2, w), frac(3, h)),
            _ => (None, None, None, None),
        };
        let mut a = self.activity.lock().unwrap();
        let seq = a.len() + 1;
        a.push(json!({
            "seq": seq, "time": swiftwin_core::time::now_rfc3339(), "source": "claude", "tool": tool,
            "summary": summary, "ok": ok, "fx": fx, "fy": fy, "fx2": fx2, "fy2": fy2,
        }));
    }

    /// Capture another app in the background; the dialog polls `inspo_status`.
    fn inspo_start(&self, bundle_id: &str, deep: bool) -> Result<Value> {
        let (phone, project) = (self.phone()?, self.project_dir()?);
        if bundle_id.is_empty() {
            return Err(SwiftWinError::invalid("choose an app"));
        }
        let mut st = self.inspo.lock().unwrap();
        if st["running"] == true {
            return Err(SwiftWinError::invalid("Inspo is already capturing an app"));
        }
        *st = json!({"running": true, "message": "Starting", "screens": 0, "max": if deep { swiftwin_inspo::MAX_SCREENS_DEEP } else { swiftwin_inspo::MAX_SCREENS }});
        self.inspo_cancel.store(false, Ordering::SeqCst);
        let (state, cancel, id) = (self.inspo.clone(), self.inspo_cancel.clone(), bundle_id.to_string());
        let back = swiftwin_builder::Project::load(&project).ok().map(|p| p.app().bundle_id.clone());
        std::thread::spawn(move || {
            let mut progress = |m: &str, n: usize| {
                let mut st = state.lock().unwrap();
                (st["message"], st["screens"]) = (json!(m), json!(n));
            };
            let r = swiftwin_inspo::capture(&phone, &id, &project, back.as_deref(), deep, &cancel, &mut progress);
            let mut st = state.lock().unwrap();
            *st = match r {
                Ok((dir, guide)) => {
                    let rel = dir.strip_prefix(&project).unwrap_or(&dir).to_string_lossy().replace('\\', "/");
                    json!({"running": false, "result": {
                        "dir": rel, "summary": guide.summary(), "prompt": guide.claude_prompt(&rel), "guide": guide,
                        "cancelled": cancel.load(Ordering::SeqCst),
                    }})
                }
                Err(e) => json!({"running": false, "error": e.to_string()}),
            };
        });
        Ok(json!({}))
    }

    fn scan(&self) -> Result<Value> {
        if self.force_mock {
            return Ok(json!({"devices": [], "mock_only": true, "error": null}));
        }
        let mux = swiftwin_device::real::usbmux::Usbmux::platform();
        match swiftwin_device::real::list_devices(&mux) {
            Ok(d) => Ok(json!({"devices": d, "mock_only": false, "error": null})),
            Err(e) => Ok(json!({"devices": [], "mock_only": false, "error": e.to_string()})),
        }
    }

    fn connect(&self, mock: bool, udid: Option<String>) -> Result<Value> {
        let phone = connect(&ConnectOptions { mock: mock || self.force_mock, udid })?;
        let info = phone.device.info()?;
        if !info.paired {
            return Err(SwiftWinError::NotTrusted("tap \"Trust\" on the iPhone, then try again".into()));
        }
        let p2 = phone.clone();
        // Real phones: WebDriverAgent's video stream (~23 fps), with USB screenshots
        // (2-3 fps) while it starts or if it stops.
        let video = (!phone.is_mock).then(|| {
            let p3 = phone.clone();
            Video { kind: SourceKind::Stream, feed: Feed::Mjpeg(Arc::new(move || p3.video_stream(VIDEO_FPS))) }
        });
        let mirror = Mirror::start(Arc::new(move || p2.screenshot()), video, Policy::new(Mode::Auto), if phone.is_mock { 8.0 } else { 3.0 });
        // Follow device logs into the shared buffer, until this session ends.
        let (dev, logs) = (phone.device.clone(), self.logs.clone());
        let logs_stop = Arc::new(AtomicBool::new(false));
        let stop = logs_stop.clone();
        logs.lock().unwrap().clear();
        std::thread::spawn(move || {
            let _ = dev.syslog(&LogQuery { follow: true, limit: Some(200), ..Default::default() }, &mut |e| {
                if stop.load(Ordering::SeqCst) {
                    return false;
                }
                let mut l = logs.lock().unwrap();
                l.push(e);
                if l.len() > 20_000 {
                    l.drain(..5_000);
                }
                true
            });
        });
        let session = Session { ws: Arc::new(Workspace::new(phone)), mirror, info: info.clone(), logs_stop };
        if let Some(old) = self.session.lock().unwrap().replace(session) {
            old.end();
        }
        Ok(serde_json::to_value(info)?)
    }

    /// `swiftwin wda prepare` in the background: download, sign (free Apple ID or paid key) and
    /// install the runner; `wda_job` reports the steps.
    fn wda_install(&self) -> Result<Value> {
        let ws = self.workspace()?;
        {
            let mut j = self.wda_job.lock().unwrap();
            if j["running"] == true {
                return Err(SwiftWinError::invalid("WebDriverAgent is already being installed"));
            }
            *j = json!({"running": true, "step": "Starting"});
        }
        let job = self.wda_job.clone();
        std::thread::spawn(move || {
            let r = ws.prepare_wda(None, &mut |step: &str| *job.lock().unwrap() = json!({"running": true, "step": step}));
            *job.lock().unwrap() = match r {
                Ok(app) => json!({"running": false, "result": app}),
                Err(e) => json!({"running": false, "error": e.to_string()}),
            };
        });
        Ok(json!({"started": true}))
    }

    fn with_session<T>(&self, f: impl FnOnce(&Session) -> T) -> Result<T> {
        let g = self.session.lock().unwrap();
        let s = g.as_ref().ok_or(SwiftWinError::NoDevice)?;
        Ok(f(s))
    }

    fn phone(&self) -> Result<Phone> {
        self.with_session(|s| s.ws.phone.clone())
    }

    fn workspace(&self) -> Result<Arc<Workspace>> {
        self.with_session(|s| s.ws.clone())
    }

    /// Fractions of the displayed image (0..1) → device points.
    fn to_points(&self, phone: &Phone, fx: f64, fy: f64) -> Result<Point> {
        let (w, h) = phone.control.window_size()?;
        Ok(Point { x: (fx.clamp(0.0, 1.0) * w).round(), y: (fy.clamp(0.0, 1.0) * h).round() })
    }

    fn status(&self) -> Result<Value> {
        let project = self.project.lock().unwrap().clone();
        let g = self.session.lock().unwrap();
        Ok(match g.as_ref() {
            None => json!({"connected": false, "project": project}),
            Some(s) => json!({
                "connected": true,
                "device": s.info,
                "mock": s.ws.is_mock(),
                "mirror": s.mirror.status(),
                "project": project,
            }),
        })
    }

    fn frame(&self, after: u64, wait_ms: u64) -> Result<Value> {
        let mirror = self.with_session(|s| s.mirror.clone())?;
        Ok(match mirror.wait_next(after, Duration::from_millis(wait_ms.min(5000))) {
            None => json!(null),
            Some(f) => json!({
                "seq": f.seq,
                "mime": f.mime(),
                "data": swiftwin_core::base64_bytes::encode(&f.data),
                "width": f.width,
                "height": f.height,
                "source": f.source,
            }),
        })
    }

    fn project_dir(&self) -> Result<PathBuf> {
        self.project.lock().unwrap().clone().ok_or_else(|| SwiftWinError::invalid("open or create a project first"))
    }

    /// Resolve a project-relative path, refusing anything that escapes the project: `..`,
    /// absolute paths, Windows device names and alternate data streams, and links (symlinks,
    /// junctions) that point out of it.
    fn project_path(&self, rel: &str) -> Result<PathBuf> {
        let root = self.project_dir()?;
        swiftwin_core::paths::join_inside(&root, Path::new(rel)).ok_or_else(|| SwiftWinError::invalid("path must be inside the project"))
    }

    fn open_project(&self, path: &Path) -> Result<Value> {
        let project = swiftwin_builder::Project::load(path)?;
        let root = project.dir.clone();
        let mut files = Vec::new();
        swiftwin_builder::project::walk(&root, &mut |p| {
            let rel = p.strip_prefix(&root).unwrap_or(p).to_string_lossy().replace('\\', "/");
            if !rel.starts_with("build/") && !rel.starts_with(".") {
                files.push(rel);
            }
        })?;
        *self.project.lock().unwrap() = Some(root.clone());
        Ok(json!({"root": root, "name": project.app().name, "bundle_id": project.app().bundle_id, "files": files}))
    }
}

/// All numbers in a string, e.g. "Swiped (1,2) -> (3,4)" → [1,2,3,4].
fn numbers(s: &str) -> Vec<f64> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() || (c == '.' && !cur.is_empty()) || (c == '-' && cur.is_empty()) {
            cur.push(c);
        } else {
            if let Ok(v) = cur.parse() {
                out.push(v);
            }
            cur.clear();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_app_flow() {
        let core = AppCore::new(true);
        assert_eq!(core.dispatch("scan", &json!({})).unwrap()["mock_only"], true);
        assert!(core.dispatch("tap", &json!({"fx": 0.5, "fy": 0.5})).is_err(), "no device yet");
        let info = core.dispatch("connect", &json!({"mock": true})).unwrap();
        assert_eq!(info["name"], "Mock iPhone");
        let f = core.dispatch("frame", &json!({"after": 0, "wait_ms": 3000})).unwrap();
        assert_eq!(f["mime"], "image/png");
        let seq = f["seq"].as_u64().unwrap();
        core.dispatch("tap", &json!({"fx": 334.5 / 393.0, "fy": 121.0 / 852.0})).unwrap(); // the "Hello" icon
        let f2 = core.dispatch("frame", &json!({"after": seq, "wait_ms": 3000})).unwrap();
        assert!(f2["seq"].as_u64().unwrap() > seq);
        assert!(core.dispatch("ui_tree", &json!({})).unwrap()["text"].as_str().unwrap().contains("Tap me"));
        std::thread::sleep(Duration::from_millis(200));
        let logs = core.dispatch("logs_since", &json!({"index": 0})).unwrap();
        assert!(logs["entries"].as_array().unwrap().iter().any(|e| e["message"].as_str().unwrap().contains("Launching")));
        let st = core.dispatch("status", &json!({})).unwrap();
        assert_eq!(st["connected"], true);
        assert_eq!(st["mirror"]["active"], "usb");

        let dir = std::env::temp_dir().join(format!("swiftwin-appcore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = core.dispatch("new_project", &json!({"parent": dir, "name": "Demo"})).unwrap();
        assert!(p["files"].as_array().unwrap().iter().any(|f| f == "Sources/ContentView.swift"));
        let t = core.dispatch("read_file", &json!({"path": "Sources/ContentView.swift"})).unwrap();
        assert!(t["text"].as_str().unwrap().contains("Tap me"));
        core.dispatch("write_file", &json!({"path": "Sources/Extra.swift", "text": "// hi"})).unwrap();
        for bad in ["../escape", "C:/Windows/win.ini", "/etc/passwd", "Sources/ContentView.swift::$DATA", "CON", "Sources/nul.swift"] {
            assert!(core.dispatch("read_file", &json!({"path": bad})).is_err(), "read {bad}");
            assert!(core.dispatch("write_file", &json!({"path": bad, "text": "x"})).is_err(), "write {bad}");
        }
        let r = core.dispatch("run", &json!({})).unwrap();
        assert!(r["message"].as_str().unwrap().contains("Launched com.example.demo"));
        core.dispatch("press", &json!({"button": "home"})).unwrap();
        // Inspo: capture the Tunes app into the project, then add its theme.
        let apps = core.dispatch("inspo_apps", &json!({})).unwrap();
        assert!(apps["apps"].as_array().unwrap().iter().any(|a| a["bundle_id"] == "fm.tunes.player"));
        core.dispatch("inspo_start", &json!({"bundle_id": "fm.tunes.player"})).unwrap();
        let st = loop {
            let st = core.dispatch("inspo_status", &json!({})).unwrap();
            if st["running"] == false {
                break st;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let res = &st["result"];
        assert_eq!(res["dir"], ".inspo/Tunes", "{st}");
        assert!(res["prompt"].as_str().unwrap().contains("Do not copy"));
        let img = core.dispatch("inspo_image", &json!({"path": ".inspo/Tunes/01.png"})).unwrap();
        assert!(img["data"].as_str().unwrap().len() > 100);
        assert!(core.dispatch("inspo_image", &json!({"path": "../x.png"})).is_err());
        let t = core.dispatch("inspo_add_theme", &json!({"dir": ".inspo/Tunes"})).unwrap();
        assert_eq!(t["path"], "Sources/InspoTheme.swift");
        assert!(core.dispatch("read_file", &json!({"path": "Sources/InspoTheme.swift"})).unwrap()["text"].as_str().unwrap().contains("enum InspoTheme"));
        let d = core.dispatch("doctor", &json!({})).unwrap();
        assert_eq!(d["ok"], true);
        assert!(d["text"].as_str().unwrap().contains("PASS"));
        // Claude's tool calls through the bridge are executed on the app's phone and recorded.
        core.dispatch("mcp_tool", &json!({"tool": "press_button", "args": {"button": "home"}, "mock": true})).unwrap();
        core.dispatch("mcp_tool", &json!({"tool": "tap", "args": {"label": "Demo"}, "mock": true})).unwrap();
        let r = core.dispatch("mcp_tool", &json!({"tool": "tap", "args": {"label": "Tap me"}, "mock": true})).unwrap();
        assert_eq!(r["forwarded"], true);
        let r = core.dispatch("mcp_tool", &json!({"tool": "swipe", "args": {"direction": "up"}, "mock": true})).unwrap();
        assert_eq!(r["result"]["isError"], false);
        core.dispatch("mcp_tool", &json!({"tool": "tap", "args": {"label": "Nope"}, "mock": true})).unwrap();
        let a = core.dispatch("activity_since", &json!({"index": 0})).unwrap();
        assert!(a["busy"].is_null(), "no tool is running between calls");
        let e = &a["entries"].as_array().unwrap()[2..];
        assert_eq!(e.len(), 3);
        assert_eq!(e[0]["summary"], "Tapped “Tap me”");
        assert!(e[0]["fx"].as_f64().unwrap() > 0.0 && e[0]["fy"].as_f64().unwrap() < 1.0);
        assert_eq!(e[1]["summary"], "Swiped up");
        assert!(e[1]["fy2"].as_f64().unwrap() < e[1]["fy"].as_f64().unwrap(), "finger moves up");
        assert_eq!(e[2]["ok"], false);
        assert_eq!(core.dispatch("mcp_tool", &json!({"tool": "ui_tree", "args": {}, "mock": false})).unwrap()["forwarded"], false);
        core.dispatch("disconnect", &json!({})).unwrap();
        assert_eq!(core.dispatch("mcp_tool", &json!({"tool": "ui_tree", "args": {}, "mock": true})).unwrap()["forwarded"], false);
        assert_eq!(core.dispatch("status", &json!({})).unwrap()["connected"], false);
        std::fs::remove_dir_all(dir).ok();
    }
}

#[cfg(test)]
mod num_tests {
    #[test]
    fn wda_setup_on_the_mock_phone() {
        use serde_json::json;
        use std::time::Duration;
        let core = crate::AppCore::new(true);
        assert!(core.dispatch("wda_status", &json!({})).is_err(), "no phone yet");
        core.dispatch("connect", &json!({"mock": true})).unwrap();
        assert_eq!(core.dispatch("wda_status", &json!({})).unwrap()["installed"], true);
        core.dispatch("wda_install", &json!({})).unwrap();
        let job = loop {
            let j = core.dispatch("wda_job", &json!({})).unwrap();
            if j["running"] == false {
                break j;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(job["result"]["bundle_id"].is_string(), "{job}");
        assert_eq!(core.dispatch("wda_check", &json!({})).unwrap()["ok"], true);
    }

    #[test]
    fn show_studio_only_focuses_known_pages() {
        use serde_json::json;
        use std::sync::{Arc, Mutex};
        let core = crate::AppCore::new(true);
        assert_eq!(core.dispatch("show_studio", &json!({"page": "account"})).unwrap()["opened"], false, "no desktop app");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let _ = core.studio_opener.set(Box::new(move |p: &str| s2.lock().unwrap().push(p.to_string())));
        assert_eq!(core.dispatch("show_studio", &json!({"page": "signing"})).unwrap()["opened"], true);
        assert!(core.dispatch("show_studio", &json!({"page": "javascript:alert(1)"})).is_err());
        assert_eq!(*seen.lock().unwrap(), vec!["signing".to_string()]);
    }

    #[test]
    fn numbers() {
        assert_eq!(super::numbers("Swiped (196,639) -> (196,213)"), vec![196.0, 639.0, 196.0, 213.0]);
        assert_eq!(super::numbers("Tapped (105, 190)"), vec![105.0, 190.0]);
    }
}
