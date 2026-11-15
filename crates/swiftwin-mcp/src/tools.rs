//! MCP tool definitions and their implementations.

use std::path::PathBuf;

use serde_json::{json, Value};
use swiftwin_core::{Button, LogLevel, LogQuery, Point, Result, SwiftWinError};

use crate::Server;

pub fn definitions() -> Vec<Value> {
    let project = json!({"type": "string", "description": "Project folder containing swiftwin.toml (default: the server's project)"});
    vec![
        json!({"name": "show_phone", "title": "Show the iPhone window",
            "description": "Show the live iPhone (a mirror that shows your taps and swipes and that the user can tap too). In the Claude desktop app it returns a one-time link: open it in the built-in browser (the Browser pane beside the chat). Elsewhere it opens a window on the PC, so the user can watch (and use) the phone while you work. Call it once at the start of a session.",
            "inputSchema": {"type": "object", "properties": {}}}),
        json!({"name": "build", "title": "Build app",
            "description": "Compile the SwiftUI project for iPhone (arm64-apple-ios). Returns all compiler errors and warnings and how long it took. \
                The first build with a new iOS SDK also compiles the SDK's modules (a few minutes, once); later builds take under a minute.",
            "inputSchema": {"type": "object", "properties": {"project": project, "release": {"type": "boolean", "description": "optimized release build (default false: debug)"}}}}),
        json!({"name": "install_run", "title": "Build, install and launch",
            "description": "Build, sign, install and launch the app on the connected iPhone. Use after every code change. \
                Returns compiler warnings (errors fail the call with the full compiler output) and a timing breakdown.",
            "inputSchema": {"type": "object", "properties": {"project": project, "release": {"type": "boolean", "description": "optimized release build (default false: debug)"}}}}),
        json!({"name": "screenshot", "title": "Screenshot",
            "description": "Exact screenshot of the iPhone screen (PNG). Tap coordinates are in points, not image pixels; the reply says how to convert.",
            "inputSchema": {"type": "object", "properties": {
                "max_width": {"type": "integer", "description": "downscale to at most this width in pixels (default 600; the phone's full width is ~1200)"},
                "scale": {"type": "number", "minimum": 0.1, "maximum": 1, "description": "instead of max_width: fraction of the full resolution, 0.1-1 (1 = every pixel)"},
                "settle": {"type": "boolean", "description": "wait until the screen stops moving (an animation ends), up to 2 s (default true)"}}}}),
        json!({"name": "ui_tree", "title": "UI element tree",
            "description": "Accessibility tree of what is on screen: element types, labels, values and frames in points. \
                The default `compact` text leaves out off-screen elements and empty containers; `text` and `json` give everything. \
                An open alert is listed first.",
            "inputSchema": {"type": "object", "properties": {"format": {"type": "string", "enum": ["compact", "text", "json"]}}}}),
        json!({"name": "tap", "title": "Tap",
            "description": "Tap an element by its label/name/value (preferred) or at x,y in points. Alert buttons work by label too. \
                Fails if no element with that label is on screen (scroll to it first), or if it is behind the keyboard. \
                Warns when x,y is on the keyboard or in the status bar / home indicator area.",
            "inputSchema": {"type": "object", "properties": {"label": {"type": "string", "description": "label, name or value of the element (from ui_tree)"}, "x": {"type": "number", "description": "points"}, "y": {"type": "number", "description": "points"}}}}),
        json!({"name": "swipe", "title": "Swipe",
            "description": "Swipe from (x1,y1) to (x2,y2) in points, or give direction (up/down/left/right) to scroll from the screen centre.",
            "inputSchema": {"type": "object", "properties": {
                "x1": {"type": "number"}, "y1": {"type": "number"}, "x2": {"type": "number"}, "y2": {"type": "number"},
                "direction": {"type": "string", "enum": ["up", "down", "left", "right"]},
                "duration": {"type": "number", "description": "seconds (default 0.3)"}}}}),
        json!({"name": "type", "title": "Type text",
            "description": "Type text into the focused text field (tap the field first). Use \\n to press return. \
                Says which element has keyboard focus, and fails without typing if none has.",
            "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}}),
        json!({"name": "wait_for", "title": "Wait for an element",
            "description": "Wait until an element is on screen (or, with gone=true, no longer is): `label` matches a label/name/value exactly, \
                `text` matches any part of one (alerts too). Polls ui_tree about twice a second. \
                timeout=0 checks once: use it as an assertion (\"is this text on screen now?\"). Fails with what is on screen when the time runs out.",
            "inputSchema": {"type": "object", "properties": {
                "label": {"type": "string"}, "text": {"type": "string"},
                "gone": {"type": "boolean", "description": "wait for it to disappear instead (default false)"},
                "timeout": {"type": "number", "description": "seconds, 0-60 (default 10)"}}}}),
        json!({"name": "crash_reports", "title": "Crash reports",
            "description": "The app's crash reports on the phone, newest first (copied into <project>/build/crashes). \
                Give `file` (a name from the list) to read one. `all` lists every app's reports.",
            "inputSchema": {"type": "object", "properties": {"project": project,
                "file": {"type": "string", "description": "a report name from the list, to read it"}, "all": {"type": "boolean", "description": "list every app's reports, not only the project's app (default false)"}}}}),
        json!({"name": "appearance", "title": "Dark / light mode",
            "description": "Switch the whole iPhone to dark or light appearance, to test the app in both. \
                It changes the phone's setting, not the app: switch it back when you are done.",
            "inputSchema": {"type": "object", "properties": {"mode": {"type": "string", "enum": ["dark", "light"]}}, "required": ["mode"]}}),
        json!({"name": "press_button", "title": "Press button",
            "description": "Press a hardware button: home, volumeUp or volumeDown.",
            "inputSchema": {"type": "object", "properties": {"button": {"type": "string", "enum": ["home", "volumeUp", "volumeDown"]}}, "required": ["button"]}}),
        json!({"name": "logs", "title": "Device logs",
            "description": "Recent device log lines, optionally filtered by process name, text or minimum level. \
                `app_only` shows only what the project's app wrote with os.Logger(subsystem: <its bundle id>, ...), without system noise. \
                Interpolated values show as <private> unless logged with `privacy: .public`. \
                Every reply starts with `since=N`: pass it back as `since` to get only the lines logged after that call. \
                The reply also warns when the phone's log stream stalled or was restarted (lines in between are lost).",
            "inputSchema": {"type": "object", "properties": {
                "project": project, "app_only": {"type": "boolean", "description": "only the project's app's own os.Logger lines (default false)"},
                "since": {"type": "integer", "description": "only lines after this cursor (the `since=N` of an earlier logs reply)"},
                "process": {"type": "string"}, "contains": {"type": "string"},
                "level": {"type": "string", "enum": ["debug", "info", "notice", "error", "fault"]},
                "limit": {"type": "integer", "description": "default 100"}}}}),
        json!({"name": "device_info", "title": "Device info",
            "description": "Connected iPhone: name, model, iOS version, trust/Developer Mode state, screen size in points.",
            "inputSchema": {"type": "object", "properties": {}}}),
        json!({"name": "launch_app", "title": "Open app",
            "description": "Open (restart) an installed app without going through the home screen: the project's app by default, or `bundle_id`. \
                `text_size` starts it with another Dynamic Type size (only this launch, the phone's setting is unchanged).",
            "inputSchema": {"type": "object", "properties": {"project": project, "bundle_id": {"type": "string"},
                "text_size": {"type": "string", "enum": TEXT_SIZES.iter().map(|(k, _)| *k).collect::<Vec<_>>()}}}}),
        json!({"name": "terminate_app", "title": "Close app",
            "description": "Close a running app: the project's app by default, or `bundle_id`.",
            "inputSchema": {"type": "object", "properties": {"project": project, "bundle_id": {"type": "string"}}}}),
        json!({"name": "release", "title": "TestFlight release",
            "description": "Send the project to a cloud Mac (GitHub Actions, Xcode 26) to build and upload to TestFlight.",
            "inputSchema": {"type": "object", "properties": {"project": project,
                "wait": {"type": "boolean", "description": "wait for the cloud build to finish (default true)"}}}}),
        json!({"name": "inspo_capture", "title": "Capture design inspiration",
            "description": "Open another app installed on the phone, capture its screens safely (only tab-bar taps and scrolls) and extract a style guide (palette, type scale, spacing, patterns) into <project>/.inspo/<app>/ with InspoTheme.swift. Call without bundle_id to list the apps. Inspiration only: never copy the other app's logo, icons, artwork, brand name or exact screens.",
            "inputSchema": {"type": "object", "properties": {"project": project, "bundle_id": {"type": "string"},
                "deep": {"type": "boolean", "description": "also open every button that doesn't buy, send, delete, sign out, like or add (up to 40 screens, takes several minutes)"}}}}),
    ]
}

fn text(t: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": t.into()}], "isError": false})
}

fn fail(e: impl std::fmt::Display) -> Value {
    json!({"content": [{"type": "text", "text": format!("Error: {e}")}], "isError": true})
}

pub fn call(s: &mut Server, name: &str, args: &Value) -> Value {
    match run(s, name, args) {
        Ok(v) => v,
        Err(e) => fail(e),
    }
}

fn project(s: &Server, args: &Value) -> PathBuf {
    args["project"].as_str().map(PathBuf::from).unwrap_or_else(|| s.project_dir.clone())
}

fn num(args: &Value, k: &str) -> Option<f64> {
    args[k].as_f64()
}

fn run(s: &mut Server, name: &str, args: &Value) -> Result<Value> {
    if name == "show_phone" {
        return Ok(text(crate::open_phone_window(s.mock)?)); // needs no phone connection here
    }
    let ws = s.ws()?;
    let phone = ws.phone.clone();
    match name {
        "build" => {
            let started = std::time::Instant::now();
            let out = ws.build(&project(s, args), args["release"].as_bool().unwrap_or(false))?;
            let mut t = format!("Build succeeded in {:.0} s: {}\n", started.elapsed().as_secs_f64(), out.app.display());
            for w in &out.warnings {
                t.push_str(&format!("asset warning: {w}\n"));
            }
            if !out.compiler_log.trim().is_empty() {
                t.push_str(&format!("compiler output:\n{}", out.compiler_log));
            }
            Ok(text(t))
        }
        "install_run" => {
            let started = std::time::Instant::now();
            let r = ws.install_run(&project(s, args), args["release"].as_bool().unwrap_or(false))?;
            let secs = started.elapsed().as_secs_f64();
            let mut t = format!(
                "Installed and launched {} (pid {}) in {secs:.0} s ({}). Take a screenshot to see it.\n",
                r.bundle_id,
                r.pid,
                r.timing_text()
            );
            for w in &r.warnings {
                t.push_str(&format!("asset warning: {w}\n"));
            }
            if !r.compiler_log.trim().is_empty() {
                t.push_str(&format!("compiler output:\n{}", r.compiler_log));
            }
            Ok(text(t))
        }
        "screenshot" => {
            let mut shot = phone.screenshot()?;
            if args["settle"].as_bool().unwrap_or(true) {
                // Mid-animation frames differ from one shot to the next: wait until two match.
                let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while std::time::Instant::now() < until {
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    let next = phone.screenshot()?;
                    let same = next.png == shot.png;
                    shot = next;
                    if same {
                        break;
                    }
                }
            }
            let max_w = match num(args, "scale") {
                Some(f) if (0.1..=1.0).contains(&f) => ((shot.width as f64) * f).round() as u32,
                Some(f) => return Err(SwiftWinError::invalid(format!("scale must be 0.1-1, not {f}"))),
                None => args["max_width"].as_u64().unwrap_or(600).max(100) as u32,
            };
            let (png, w, h) = downscale(&shot.png, shot.width, shot.height, max_w)?;
            let (pw, ph) = phone.control.window_size().unwrap_or((shot.width as f64, shot.height as f64));
            Ok(json!({"content": [
                {"type": "image", "data": swiftwin_core::base64_bytes::encode(&png), "mimeType": "image/png"},
                {"type": "text", "text": format!(
                    "Screenshot {w}x{h} px (original {}x{}). Screen is {pw:.0}x{ph:.0} points: point = pixel x {:.3}.",
                    shot.width, shot.height, pw / w as f64)}
            ], "isError": false}))
        }
        "ui_tree" => {
            let t = phone.control.ui_tree()?;
            let alert = match phone.control.alert().ok().flatten() {
                Some(a) => format!("Alert \"{}\" with buttons: {} (tap one by its label)\n", a.text.replace('\n', " "), a.buttons.join(", ")),
                None => String::new(),
            };
            match args["format"].as_str() {
                Some("json") => Ok(text(serde_json::to_string_pretty(&t)?)),
                Some("text") => Ok(text(alert + &t.to_text())),
                _ => {
                    let (w, h) = phone.control.window_size()?;
                    Ok(text(alert + &t.to_compact_text(w, h)))
                }
            }
        }
        "tap" => {
            let at = if let Some(l) = args["label"].as_str() {
                if let Some(a) = phone.control.alert().ok().flatten() {
                    if let Some(b) = a.buttons.iter().find(|b| b.eq_ignore_ascii_case(l)) {
                        phone.control.alert_tap(b)?;
                        return Ok(text(format!("Tapped alert button \"{b}\"")));
                    }
                }
                let tree = phone.control.ui_tree()?;
                let (w, h) = phone.control.window_size()?;
                match tree.find_visible(l, w, h) {
                    Ok(n) => {
                        let at = n.visible_center(w, h);
                        if !n.kind.ends_with("Key") && tree.keyboard(w, h).is_some_and(|k| contains(&k, at)) {
                            return Err(SwiftWinError::invalid(format!(
                                "{l:?} is behind the keyboard: dismiss it first (e.g. type \"\\n\" or tap outside the field), then tap again"
                            )));
                        }
                        at
                    }
                    Err(Some(_)) => {
                        return Err(SwiftWinError::NotFound(format!(
                            "{l:?} exists but is not on screen (off-screen or hidden): swipe to scroll it into view, then tap again"
                        )))
                    }
                    Err(None) => {
                        return Err(SwiftWinError::NotFound(format!("no element labelled {l:?}; call ui_tree to see what is on screen")))
                    }
                }
            } else {
                match (num(args, "x"), num(args, "y")) {
                    (Some(x), Some(y)) => Point { x, y },
                    _ => return Err(SwiftWinError::invalid("give `label`, or both `x` and `y`")),
                }
            };
            let mut warn = String::new();
            if args["label"].is_null() {
                if let (Ok(tree), Ok((w, h))) = (phone.control.ui_tree(), phone.control.window_size()) {
                    if tree.keyboard(w, h).is_some_and(|k| contains(&k, at)) {
                        warn.push_str("\nwarning: that point is on the keyboard (it typed a key, if any)");
                    }
                    // shortcut: fixed insets of Face ID iPhones; read the real safe area if WDA ever reports it.
                    if at.y < 54.0 || at.y > h - 34.0 {
                        warn.push_str("\nwarning: that point is in the status bar / home indicator area, outside the safe area");
                    }
                }
            }
            phone.control.tap(at)?;
            Ok(text(format!("Tapped ({:.0}, {:.0}){warn}", at.x, at.y)))
        }
        "swipe" => {
            let d = num(args, "duration").unwrap_or(0.3);
            let (from, to) = if let Some(dir) = args["direction"].as_str() {
                let (w, h) = phone.control.window_size()?;
                let (cx, cy) = (w / 2.0, h / 2.0);
                let (dx, dy) = (w * 0.3, h * 0.25);
                match dir {
                    // "up" = content moves up (finger moves up), i.e. scroll down the page.
                    "up" => (Point { x: cx, y: cy + dy }, Point { x: cx, y: cy - dy }),
                    "down" => (Point { x: cx, y: cy - dy }, Point { x: cx, y: cy + dy }),
                    "left" => (Point { x: cx + dx, y: cy }, Point { x: cx - dx, y: cy }),
                    "right" => (Point { x: cx - dx, y: cy }, Point { x: cx + dx, y: cy }),
                    o => return Err(SwiftWinError::invalid(format!("unknown direction {o}"))),
                }
            } else {
                match (num(args, "x1"), num(args, "y1"), num(args, "x2"), num(args, "y2")) {
                    (Some(a), Some(b), Some(c), Some(e)) => (Point { x: a, y: b }, Point { x: c, y: e }),
                    _ => return Err(SwiftWinError::invalid("give x1,y1,x2,y2 or direction")),
                }
            };
            phone.control.swipe(from, to, d)?;
            Ok(text(format!("Swiped ({:.0},{:.0}) -> ({:.0},{:.0})", from.x, from.y, to.x, to.y)))
        }
        "type" => {
            let t = args["text"].as_str().ok_or_else(|| SwiftWinError::invalid("`text` is required"))?;
            let into = match phone.control.focused() {
                Ok(Some(n)) => format!(" into {}", n.line()),
                Ok(None) => {
                    return Err(SwiftWinError::invalid("no element has keyboard focus: tap the text field first (nothing was typed)"))
                }
                Err(e) => format!(" (could not check keyboard focus: {e})"),
            };
            phone.control.type_text(t)?;
            Ok(text(format!("Typed {} character(s){into}", t.chars().count())))
        }
        "press_button" => {
            let b: Button = args["button"].as_str().unwrap_or("").parse()?;
            phone.control.press_button(b)?;
            Ok(text(format!("Pressed {}", b.wda_name())))
        }
        "logs" => {
            let q = LogQuery {
                process: args["process"].as_str().map(str::to_string),
                contains: args["contains"].as_str().map(str::to_string),
                min_level: args["level"].as_str().map(LogLevel::parse),
                follow: false,
                limit: Some(args["limit"].as_u64().unwrap_or(100) as usize),
                subsystem: match args["app_only"].as_bool() {
                    Some(true) => Some(swiftwin_builder::Project::load(&project(s, args))?.app().bundle_id.clone()),
                    _ => None,
                },
                since: args["since"].as_u64(),
            };
            let lines = swiftwin_device::collect_logs(phone.device.as_ref(), &q)?;
            let mut t = log_header(phone.device.log_status());
            if lines.is_empty() {
                t.push_str("(no matching log lines)");
                if q.subsystem.is_some() {
                    t.push_str(" - app_only matches Logger(subsystem:) starting with the project's bundle id; print() and NSLog lines have none: use `process` instead");
                }
            }
            let body: Vec<String> =
                lines.iter().map(|e| format!("{} {}[{}] <{:?}> {}", e.timestamp, e.process, e.pid, e.level, e.message)).collect();
            t.push_str(&body.join("\n"));
            Ok(text(t))
        }
        "device_info" => Ok(text(serde_json::to_string_pretty(&phone.device.info()?)?)),
        "wait_for" => wait_for(&phone, args),
        "appearance" => {
            let dark = match args["mode"].as_str() {
                Some("dark") => true,
                Some("light") => false,
                _ => return Err(SwiftWinError::invalid("`mode` must be dark or light")),
            };
            phone.control.set_appearance(dark)?;
            Ok(text(format!("The iPhone is now in {} mode (a phone setting: switch it back when you are done).", if dark { "dark" } else { "light" })))
        }
        "crash_reports" => {
            let proj = swiftwin_builder::Project::load(&project(s, args))?;
            let dir = proj.build_dir().join("crashes");
            if let Some(f) = args["file"].as_str() {
                // Only a file name from the list, never a path elsewhere.
                let p = swiftwin_core::paths::join_inside(&dir, std::path::Path::new(f))
                    .filter(|p| p.is_file())
                    .ok_or_else(|| SwiftWinError::NotFound(format!("no crash report {f:?}: call crash_reports for the list")))?;
                let body = String::from_utf8_lossy(&std::fs::read(&p)?).into_owned();
                let cut: String = body.chars().take(30_000).collect();
                let more = if cut.len() < body.len() { "\n… (cut at 30,000 characters)" } else { "" };
                return Ok(text(format!("{}:\n{cut}{more}", p.display())));
            }
            let exe = proj.executable().to_lowercase();
            let mut files: Vec<(std::time::SystemTime, String, u64)> = phone
                .device
                .crash_reports(&dir)?
                .into_iter()
                .filter_map(|p| {
                    let rel = p.strip_prefix(&dir).ok()?.to_string_lossy().replace('\\', "/");
                    let name = p.file_name()?.to_string_lossy().to_lowercase();
                    let mine = name.starts_with(&format!("{exe}-")) || name.starts_with(&format!("{exe}."));
                    let m = p.metadata().ok()?;
                    (args["all"].as_bool().unwrap_or(false) || mine).then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), rel, m.len()))
                })
                .collect();
            files.sort_by(|a, b| b.0.cmp(&a.0));
            if files.is_empty() {
                return Ok(text(format!("No crash reports for {} on the phone (all=true lists every app's).", proj.executable())));
            }
            let list: Vec<String> = files.iter().take(30).map(|(_, n, len)| format!("- {n} ({} KB)", len / 1024 + 1)).collect();
            Ok(text(format!("{} crash report(s), newest first; read one with `file`:\n{}", files.len(), list.join("\n"))))
        }
        "launch_app" | "terminate_app" => {
            // The project's app is installed under its bundle id, or one a free-account signer extended.
            let id = match args["bundle_id"].as_str() {
                Some(id) => id.to_string(),
                None => {
                    let want = swiftwin_builder::Project::load(&project(s, args))?.app().bundle_id.clone();
                    let found = ws.installed_as(&want).map_err(|_| {
                        SwiftWinError::NotFound(format!("{want} is not installed on the phone: call install_run first"))
                    })?;
                    found.bundle_id
                }
            };
            if name == "launch_app" {
                let mut opts = swiftwin_device::LaunchOptions { kill_existing: true, ..Default::default() };
                if let Some(size) = args["text_size"].as_str() {
                    let (_, category) = TEXT_SIZES.iter().find(|(k, _)| *k == size).ok_or_else(|| SwiftWinError::invalid(format!("unknown text_size {size}")))?;
                    // UNTESTED-ON-DEVICE: a launch argument overrides the user default UIKit reads the size from.
                    opts.args = vec!["-UIPreferredContentSizeCategoryName".into(), category.to_string()];
                }
                let pid = phone.device.launch(&id, &opts)?;
                Ok(text(format!("Launched {id} (pid {pid})")))
            } else {
                phone.device.terminate(&id)?;
                Ok(text(format!("Closed {id}")))
            }
        }
        "release" => {
            let backend = s.release.clone().ok_or_else(|| {
                SwiftWinError::Unsupported("cloud release is not configured: run `swiftwin release setup`".into())
            })?;
            Ok(text(backend.release(&project(s, args), args)?))
        }
        "inspo_capture" => {
            let Some(id) = args["bundle_id"].as_str() else {
                let apps = swiftwin_inspo::user_apps(&phone)?;
                let list: Vec<String> = apps.iter().map(|a| format!("- {} ({})", a.name, a.bundle_id)).collect();
                return Ok(text(format!("Give `bundle_id`, one of:
{}", list.join("
"))));
            };
            // Only into a real project (a folder with a valid swiftwin.toml), never an arbitrary folder.
            let proj = swiftwin_builder::Project::load(&project(s, args))?;
            let (dir, back) = (proj.dir.clone(), Some(proj.app().bundle_id.clone()));
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let (out, guide) = swiftwin_inspo::capture(&phone, id, &dir, back.as_deref(), args["deep"].as_bool().unwrap_or(false), &cancel, &mut |_, _| {})?;
            Ok(text(format!(
                "{}
Files in {}: INSPO.md, inspo.json, InspoTheme.swift (copy it into Sources/ to use it), overview.png (all screens in one grid: look at it, then at every NN.png), NN.png screenshots, NN.json UI trees.
{}
{}",
                guide.summary(),
                out.display(),
                swiftwin_inspo::GUARDRAIL,
                swiftwin_inspo::PRIVACY
            )))
        }
        other => Err(SwiftWinError::invalid(format!("unknown tool {other}"))),
    }
}

/// Dynamic Type sizes for `launch_app text_size`: UIKit's UIContentSizeCategory raw values.
const TEXT_SIZES: &[(&str, &str)] = &[
    ("xs", "UICTContentSizeCategoryXS"),
    ("s", "UICTContentSizeCategoryS"),
    ("m", "UICTContentSizeCategoryM"),
    ("l", "UICTContentSizeCategoryL"),
    ("xl", "UICTContentSizeCategoryXL"),
    ("xxl", "UICTContentSizeCategoryXXL"),
    ("xxxl", "UICTContentSizeCategoryXXXL"),
    ("accessibility-m", "UICTContentSizeCategoryAccessibilityM"),
    ("accessibility-l", "UICTContentSizeCategoryAccessibilityL"),
    ("accessibility-xl", "UICTContentSizeCategoryAccessibilityXL"),
    ("accessibility-xxl", "UICTContentSizeCategoryAccessibilityXXL"),
    ("accessibility-xxxl", "UICTContentSizeCategoryAccessibilityXXXL"),
];

fn contains(r: &swiftwin_core::Rect, p: Point) -> bool {
    p.x >= r.x && p.x <= r.x + r.width && p.y >= r.y && p.y <= r.y + r.height
}

/// Poll the screen until `label` / `text` is there (or gone). The alert counts as screen text.
fn wait_for(phone: &swiftwin_control::Phone, args: &Value) -> Result<Value> {
    let (label, txt) = (args["label"].as_str(), args["text"].as_str());
    let what = match (label, txt) {
        (Some(l), _) => format!("label {l:?}"),
        (None, Some(t)) => format!("text {t:?}"),
        _ => return Err(SwiftWinError::invalid("give `label` or `text`")),
    };
    let gone = args["gone"].as_bool().unwrap_or(false);
    let timeout = num(args, "timeout").unwrap_or(10.0).clamp(0.0, 60.0);
    let started = std::time::Instant::now();
    loop {
        let tree = phone.control.ui_tree()?;
        let (w, h) = phone.control.window_size()?;
        let alert = phone.control.alert().ok().flatten();
        let found: Option<String> = match (label, txt) {
            (Some(l), _) => tree.find_visible(l, w, h).ok().map(|n| n.line()).or_else(|| {
                alert.as_ref().and_then(|a| a.buttons.iter().find(|b| b.eq_ignore_ascii_case(l))).map(|b| format!("alert button \"{b}\""))
            }),
            (None, Some(t)) => tree.containing(t, w, h).last().map(|n| n.line()).or_else(|| {
                alert.as_ref().filter(|a| a.text.to_lowercase().contains(&t.to_lowercase())).map(|a| format!("alert \"{}\"", a.text.replace('\n', " ")))
            }),
            _ => None,
        };
        let secs = started.elapsed().as_secs_f64();
        match (&found, gone) {
            (Some(line), false) => return Ok(text(format!("Found {what} after {secs:.1} s: {line}"))),
            (None, true) => return Ok(text(format!("{what} is not on screen (after {secs:.1} s)"))),
            _ => {}
        }
        if secs >= timeout {
            let state = if gone { "still on screen" } else { "not on screen" };
            return Err(SwiftWinError::NotFound(format!(
                "{what} {state} after {timeout:.0} s. On screen now:\n{}",
                tree.to_compact_text(w, h)
            )));
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
}

/// `since=N` cursor plus a warning when the phone's log stream is silent or was restarted.
fn log_header(st: Option<swiftwin_device::LogStatus>) -> String {
    let Some(st) = st else { return String::new() };
    let mut h = format!("since={}\n", st.next);
    if let Some(s) = st.silent_secs.filter(|s| *s >= 5.0) {
        h.push_str(&format!("WARNING: no log line from the phone for {s:.0} s: the log stream may have stalled (it restarts by itself)\n"));
    }
    if st.restarts > 0 {
        h.push_str(&format!("note: the log stream was restarted {} time(s); last: {}\n", st.restarts, st.problem.unwrap_or_default()));
    }
    h
}

fn downscale(png: &[u8], w: u32, h: u32, max_w: u32) -> Result<(Vec<u8>, u32, u32)> {
    if w <= max_w {
        return Ok((png.to_vec(), w, h));
    }
    let img = swiftwin_builder::assets::Image::decode(png)?;
    let nh = ((h as f64) * (max_w as f64) / (w as f64)).round().max(1.0) as u32;
    Ok((img.resize(max_w, nh).encode_rgb(), max_w, nh))
}
