//! SwiftWin's MCP server (Model Context Protocol, stdio transport).
//!
//! Newline-delimited JSON-RPC 2.0 on stdin/stdout. Implements `initialize`,
//! `notifications/initialized`, `ping`, `tools/list` and `tools/call`.
//! Tool failures are returned as `isError: true` results (so the model sees
//! the message); protocol problems are JSON-RPC errors.
//!
//! Register with Claude Code: `claude mcp add swiftwin -- swiftwin mcp`, or Codex: `codex mcp add swiftwin -- swiftwin mcp`
//! (or `swiftwin --mock mcp` to try it without a phone).

pub mod tools;

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use swiftwin_workflow::Workspace;

pub const SUPPORTED_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Something that can publish a TestFlight build (implemented by the cloud release crate).
pub trait ReleaseBackend: Send + Sync {
    fn release(&self, project: &std::path::Path, args: &Value) -> swiftwin_core::Result<String>;
}

/// Hands a tool call to another process (the open SwiftWin desktop app), so
/// the person watching the app sees what Claude does. `None` = run it here.
pub trait Forward: Send + Sync {
    fn call(&self, tool: &str, args: &Value) -> Option<Value>;
}

type Connect = Box<dyn Fn() -> swiftwin_core::Result<Workspace> + Send + Sync>;

pub struct Server {
    ws: Mutex<Option<Arc<Workspace>>>,
    connect: Option<Connect>,
    mock: bool,
    /// Default project folder for build/install_run/release.
    pub project_dir: PathBuf,
    pub release: Option<Arc<dyn ReleaseBackend>>,
    pub forward: Option<Arc<dyn Forward>>,
    initialized: bool,
}

impl Server {
    pub fn new(ws: Workspace, project_dir: PathBuf) -> Self {
        Self::with_workspace(Arc::new(ws), project_dir)
    }

    pub fn with_workspace(ws: Arc<Workspace>, project_dir: PathBuf) -> Self {
        let mock = ws.is_mock();
        Server { ws: Mutex::new(Some(ws)), connect: None, mock, project_dir, release: None, forward: None, initialized: false }
    }

    /// Connect to the phone on first use, so the server starts even before the iPhone is plugged in.
    pub fn lazy(connect: Connect, mock: bool, project_dir: PathBuf) -> Self {
        Server { ws: Mutex::new(None), connect: Some(connect), mock, project_dir, release: None, forward: None, initialized: false }
    }

    pub fn ws(&self) -> swiftwin_core::Result<Arc<Workspace>> {
        let mut g = self.ws.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(w) = g.as_ref() {
            return Ok(w.clone());
        }
        let connect = self.connect.as_ref().ok_or(swiftwin_core::SwiftWinError::NoDevice)?;
        let w = Arc::new(connect()?);
        // From now on the `logs` tool also sees what happened before it was first called (e.g. the
        // log line of a tap Claude just made). Best effort: logs are not needed for other tools.
        let _ = w.phone.device.keep_logs();
        *g = Some(w.clone());
        Ok(w)
    }

    pub fn with_release(mut self, r: Arc<dyn ReleaseBackend>) -> Self {
        self.release = Some(r);
        self
    }

    pub fn with_forward(mut self, f: Arc<dyn Forward>) -> Self {
        self.forward = Some(f);
        self
    }

    /// Serve until EOF.
    pub fn serve(&mut self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(Value::Array(batch)) => {
                    let replies: Vec<Value> = batch.iter().filter_map(|m| self.handle(m)).collect();
                    (!replies.is_empty()).then_some(Value::Array(replies))
                }
                Ok(msg) => self.handle(&msg),
                Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
            };
            if let Some(r) = reply {
                writeln!(output, "{}", serde_json::to_string(&r)?)?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// Handle one message; `None` for notifications and responses.
    pub fn handle(&mut self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned();
        let Some(method) = msg.get("method").and_then(|m| m.as_str()) else {
            // A response to a request we never sent, or garbage.
            return id.map(|id| error(id, -32600, "invalid request"));
        };
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        let Some(id) = id else {
            if method == "notifications/initialized" {
                self.initialized = true;
            }
            return None; // notifications get no reply
        };
        let result = match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools::definitions() })),
            "tools/call" => {
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                if !tools::definitions().iter().any(|t| t["name"] == name) {
                    return Some(error(id, -32602, &format!("unknown tool: {name}")));
                }
                if let Some(v) = self.forward.clone().and_then(|f| f.call(name, &args)) {
                    return Some(json!({"jsonrpc": "2.0", "id": id, "result": v}));
                }
                Ok(tools::call(self, name, &args))
            }
            "resources/list" => Ok(json!({ "resources": [] })),
            "prompts/list" => Ok(json!({ "prompts": [] })),
            _ => Err((-32601, format!("method not found: {method}"))),
        };
        Some(match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err((code, m)) => error(id, code, &m),
        })
    }

    fn initialize(&mut self, params: &Value) -> Value {
        let requested = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or(SUPPORTED_VERSIONS[0]);
        let version = if SUPPORTED_VERSIONS.contains(&requested) { requested } else { SUPPORTED_VERSIONS[0] };
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "swiftwin", "title": "SwiftWin (iPhone from Windows)", "version": env!("CARGO_PKG_VERSION") },
            "instructions": format!(
                "SwiftWin controls a real iPhone{} connected to this Windows PC. Typical loop: edit Swift code in {}, \
                 call install_run, then screenshot/ui_tree to see the result, tap/swipe/type to interact, and logs to debug. \
                 Coordinates are in points (see device_info). Prefer tap with `label` from ui_tree over raw coordinates. \
                 Use launch_app / terminate_app to open or close the app (never look for it on the home screen), and \
                 logs with app_only=true to see only the app's own log lines (pass each reply's since=N back as `since` to read only new lines). \
                 After an action, use wait_for (not sleep + screenshot) until the next screen's element appears; wait_for with timeout=0 \
                 is an assertion. crash_reports lists and reads the app's crash reports; appearance switches dark/light mode and \
                 launch_app text_size tests larger text. \
                 The user watches the iPhone while you work: before your first other phone tool call, call show_phone yourself, \
                 without being asked. In the Claude desktop app it shows the phone in the Browser pane beside the chat (with the \
                 swiftwin-phone plugin it opens there by itself; otherwise open its link right away with preview_start, without \
                 asking); elsewhere it opens a window on the PC.",
                if self.mock { " (currently the simulated mock phone)" } else { "" },
                self.project_dir.display()
            ),
        })
    }
}

/// Open SwiftWin's phone-only window (the installed desktop app with `--phone`: a live mirror of the
/// iPhone that shows Claude's taps). If the app is already open it adds the window there.
pub fn open_phone_window(mock: bool) -> swiftwin_core::Result<String> {
    launch_app("--phone", mock)?;
    Ok("Opened the iPhone window on the PC: the user sees the phone live, with your taps and swipes.".into())
}

/// Start the installed desktop app with `flag` (`--phone`, `--pane`); a running app gets the flag
/// through its single-instance handler instead.
pub fn launch_app(flag: &str, mock: bool) -> swiftwin_core::Result<()> {
    let is_file = |p: &PathBuf| p.is_file();
    let exe = std::env::var_os("SWIFTWIN_APP")
        .map(PathBuf::from)
        .filter(is_file)
        // Installed layout: <app dir>/bin/swiftwin.exe next to <app dir>/swiftwin-app.exe.
        .or_else(|| std::env::current_exe().ok()?.parent()?.parent().map(|d| d.join("swiftwin-app.exe")).filter(is_file))
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("SwiftWin").join("swiftwin-app.exe")).filter(is_file))
        .ok_or_else(|| swiftwin_core::SwiftWinError::NotFound("the SwiftWin desktop app (swiftwin-app.exe) is not installed".into()))?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg(flag);
    if mock {
        cmd.arg("--mock");
    }
    // Null stdio: the app must not hold on to this server's stdout (Claude Code's pipe).
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn()?;
    Ok(())
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}
