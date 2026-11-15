//! A full MCP session as Claude Code would run it, against the mock phone.

use serde_json::{json, Value};
use swiftwin_control::Phone;
use swiftwin_mcp::Server;
use swiftwin_workflow::Workspace;

fn session(project: std::path::PathBuf, msgs: &[Value]) -> Vec<Value> {
    let mut s = Server::new(Workspace::new(Phone::mock()), project);
    let input: String = msgs.iter().map(|m| m.to_string() + "\n").collect();
    let mut out = Vec::new();
    s.serve(input.as_bytes(), &mut out).unwrap();
    String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn call(id: u64, name: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}})
}

fn text_of(r: &Value) -> String {
    r["result"]["content"].as_array().unwrap().iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")
}

#[test]
fn claude_style_session() {
    let dir = std::env::temp_dir().join(format!("swiftwin-mcp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let proj = swiftwin_builder::scaffold::new_project(&dir, "Notes", Some("com.example.notes")).unwrap();
    let r = session(
        proj.clone(),
        &[
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "claude-code", "version": "x"}}}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            call(3, "install_run", json!({})),
            call(4, "ui_tree", json!({})),
            call(5, "tap", json!({"label": "Tap me"})),
            call(6, "tap", json!({"label": "Your name"})),
            call(7, "type", json!({"text": "Ada"})),
            call(8, "swipe", json!({"direction": "up"})),
            call(9, "screenshot", json!({})),
            call(10, "logs", json!({"process": "Notes", "limit": 5})),
            call(11, "tap", json!({"label": "Does not exist"})),
            call(12, "press_button", json!({"button": "home"})),
            call(13, "device_info", json!({})),
            call(14, "release", json!({})),
            json!({"jsonrpc": "2.0", "id": 15, "method": "ping"}),
            call(16, "no_such_tool", json!({})),
            json!({"jsonrpc": "2.0", "id": 17, "method": "bogus/method"}),
        ],
    );
    assert_eq!(r.len(), 17, "every request (not the notification) gets one reply");
    assert_eq!(r[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(r[0]["result"]["serverInfo"]["name"], "swiftwin");
    let names: Vec<&str> = r[1]["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for want in ["show_phone", "build", "install_run", "screenshot", "ui_tree", "tap", "swipe", "type", "logs", "release", "inspo_capture"] {
        assert!(names.contains(&want), "missing tool {want}");
    }
    assert!(text_of(&r[2]).contains("Installed and launched com.example.notes"), "{}", r[2]);
    assert!(text_of(&r[3]).contains("Button \"Tap me\""));
    assert_eq!(r[6]["result"]["isError"], false, "{}", r[6]);
    let shot = &r[8]["result"]["content"][0];
    assert_eq!(shot["type"], "image");
    assert_eq!(shot["mimeType"], "image/png");
    let png = swiftwin_core::base64_bytes::decode(shot["data"].as_str().unwrap()).unwrap();
    assert_eq!(swiftwin_core::png_dimensions(&png), Some((393, 852)));
    assert!(text_of(&r[9]).contains("Button tapped, count = 1"), "{}", text_of(&r[9]));
    assert_eq!(r[10]["result"]["isError"], true);
    assert!(text_of(&r[10]).contains("ui_tree"));
    assert!(text_of(&r[12]).contains("\"connection\": \"mock\""));
    assert_eq!(r[13]["result"]["isError"], true, "release without backend is a tool error");
    assert_eq!(r[14]["result"], json!({}));
    assert_eq!(r[15]["error"]["code"], -32602);
    assert_eq!(r[16]["error"]["code"], -32601);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn version_negotiation_and_parse_errors() {
    let mut s = Server::new(Workspace::new(Phone::mock()), ".".into());
    let mut out = Vec::new();
    s.serve(&b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"1999-01-01\"}}\nnot json\n"[..], &mut out).unwrap();
    let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines[0]["result"]["protocolVersion"], swiftwin_mcp::SUPPORTED_VERSIONS[0]);
    assert_eq!(lines[1]["error"]["code"], -32700);
}

/// The tools Claude's own test report asked for: open/close the app, app-only logs, a compact
/// element list, and Inspo refusing a folder that is not a project.
#[test]
fn app_tools_and_app_only_logs() {
    let dir = std::env::temp_dir().join(format!("swiftwin-mcp-app-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let proj = swiftwin_builder::scaffold::new_project(&dir, "Notes", Some("com.example.notes")).unwrap();
    let not_a_project = dir.join("elsewhere");
    std::fs::create_dir_all(&not_a_project).unwrap();
    let r = session(
        proj.clone(),
        &[
            call(1, "install_run", json!({})),
            call(2, "press_button", json!({"button": "home"})),
            call(3, "launch_app", json!({})),
            call(4, "tap", json!({"label": "Tap me"})),
            call(5, "logs", json!({"app_only": true})),
            call(6, "ui_tree", json!({})),
            call(7, "terminate_app", json!({})),
            call(8, "inspo_capture", json!({"project": not_a_project, "bundle_id": "com.apple.mobilesafari"})),
            call(9, "screenshot", json!({"scale": 0.25})),
        ],
    );
    assert!(text_of(&r[2]).contains("Launched com.example.notes"), "{}", r[2]);
    assert_eq!(r[3]["result"]["isError"], false, "the app is in front again: {}", r[3]);
    let logs = text_of(&r[4]);
    assert!(logs.contains("Button tapped, count = 1"), "{logs}");
    assert!(logs.lines().skip(1).all(|l| l.contains("Notes[")), "only the app's own lines: {logs}");
    assert!(logs.starts_with("since="), "{logs}");
    let tree = text_of(&r[5]);
    assert!(tree.contains("Button \"Tap me\"") && !tree.contains("\n  Other ["), "compact by default: {tree}");
    assert!(text_of(&r[6]).contains("Closed com.example.notes"), "{}", r[6]);
    assert_eq!(r[7]["result"]["isError"], true, "Inspo only writes into a project");
    assert!(!not_a_project.join(".inspo").exists());
    assert_eq!(r[8]["result"]["content"][0]["type"], "image");
    // "Screenshot 302x655 px (original 1206x2622) ...": scale is honoured.
    let shot = text_of(&r[8]);
    let nums: Vec<f64> = shot.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).take(4).map(|x| x.parse().unwrap()).collect();
    assert_eq!(nums[0], (nums[2] * 0.25).round(), "{shot}");
    std::fs::remove_dir_all(dir).ok();
}

/// The tools the Tempo report asked for: wait_for (and as an assertion), type checking focus,
/// crash reports, appearance and text size, and tap warnings.
#[test]
fn tempo_report_tools() {
    let dir = std::env::temp_dir().join(format!("swiftwin-mcp-tempo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let proj = swiftwin_builder::scaffold::new_project(&dir, "Notes", Some("com.example.notes")).unwrap();
    let r = session(
        proj.clone(),
        &[
            call(1, "install_run", json!({})),
            call(2, "wait_for", json!({"label": "Tap me", "timeout": 2})),
            call(3, "wait_for", json!({"text": "no such words", "timeout": 0})),
            call(4, "wait_for", json!({"text": "no such words", "gone": true})),
            call(5, "type", json!({"text": "nobody is focused"})),
            call(6, "tap", json!({"label": "Your name"})),
            call(7, "type", json!({"text": "Ada"})),
            call(8, "tap", json!({"x": 100, "y": 10})),
            call(9, "tap", json!({"label": "Crash"})),
            call(10, "crash_reports", json!({})),
            call(11, "crash_reports", json!({"file": "../../swiftwin.toml"})),
            call(12, "appearance", json!({"mode": "dark"})),
            call(13, "launch_app", json!({"text_size": "accessibility-xl"})),
            call(14, "launch_app", json!({"text_size": "huge"})),
        ],
    );
    assert!(text_of(&r[1]).starts_with("Found label \"Tap me\""), "{}", r[1]);
    assert_eq!(r[2]["result"]["isError"], true, "assertion fails: {}", r[2]);
    assert!(text_of(&r[2]).contains("On screen now:") && text_of(&r[2]).contains("Tap me"), "{}", r[2]);
    assert!(text_of(&r[3]).contains("is not on screen"), "{}", r[3]);
    assert_eq!(r[4]["result"]["isError"], true, "no focus, nothing typed: {}", r[4]);
    assert!(text_of(&r[6]).contains("into TextField \"Your name\""), "{}", r[6]);
    assert!(text_of(&r[7]).contains("outside the safe area"), "{}", r[7]);
    let list = text_of(&r[9]);
    assert!(list.contains("1 crash report(s)") && list.contains("Notes-"), "{list}");
    let name = list.lines().nth(1).unwrap().trim_start_matches("- ").split(' ').next().unwrap().to_string();
    assert_eq!(r[10]["result"]["isError"], true, "no paths outside the crash folder");
    assert!(text_of(&r[11]).contains("dark mode"), "{}", r[11]);
    assert!(text_of(&r[12]).contains("Launched com.example.notes"), "{}", r[12]);
    assert_eq!(r[13]["result"]["isError"], true);
    let read = session(proj.clone(), &[call(1, "crash_reports", json!({"file": name}))]);
    assert!(text_of(&read[0]).contains("EXC_BREAKPOINT"), "{}", read[0]);
    std::fs::remove_dir_all(dir).ok();
}
