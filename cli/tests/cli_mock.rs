//! End-to-end tests of the `swiftwin` binary against MockDevice.

use std::process::Command;

/// Runs the CLI with an empty data dir, so a developer's own SwiftWin setup (e.g. free Apple ID
/// signing, which changes default bundle ids) can't change the results.
fn swiftwin(args: &[&str]) -> (bool, String, String) {
    let home = std::env::temp_dir().join(format!("swiftwin-cli-home-{}", std::process::id()));
    let out = Command::new(env!("CARGO_BIN_EXE_swiftwin"))
        .args(args)
        .env("SWIFTWIN_HOME", &home)
        .env_remove("SWIFTWIN_MOCK")
        .output()
        .expect("run swiftwin");
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into(), String::from_utf8_lossy(&out.stderr).into())
}

#[test]
fn info_json_with_mock_flag() {
    let (ok, out, err) = swiftwin(&["--mock", "--json", "info"]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["connection"], "mock");
    assert_eq!(v["paired"], true);
}

#[test]
fn mock_env_var_works() {
    let out = Command::new(env!("CARGO_BIN_EXE_swiftwin")).args(["devices"]).env("SWIFTWIN_MOCK", "1").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Mock iPhone"));
}

#[test]
fn screenshot_writes_png() {
    let dir = std::env::temp_dir().join(format!("swiftwin-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("s.png");
    let (ok, _, err) = swiftwin(&["--mock", "screenshot", "-o", p.to_str().unwrap()]);
    assert!(ok, "{err}");
    let data = std::fs::read(&p).unwrap();
    assert_eq!(&data[1..4], b"PNG");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn ui_tree_and_tap_by_label() {
    let (ok, out, _) = swiftwin(&["--mock", "ui-tree"]);
    assert!(ok);
    assert!(out.contains("Icon \"Hello\""), "{out}");
    let (ok, out, err) = swiftwin(&["--mock", "tap", "--label", "Hello"]);
    assert!(ok, "{err}");
    assert!(out.starts_with("Tapped"));
}

#[test]
fn demo_runs_in_temp_dir() {
    let dir = std::env::temp_dir().join(format!("swiftwin-demo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_swiftwin")).args(["--mock", "demo"]).current_dir(&dir).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("Count: 3"), "{stdout}");
    assert!(stdout.contains("Hello, SwiftWin!") || stdout.contains("SwiftWin"));
    assert!(dir.join("swiftwin-demo.png").exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn errors_are_reported() {
    let (ok, _, err) = swiftwin(&["--mock", "type", "hi"]);
    assert!(!ok);
    assert!(err.contains("focus"), "{err}");
    let (ok, _, err) = swiftwin(&["--mock", "launch", "com.nope"]);
    assert!(!ok);
    assert!(err.contains("not installed"));
}

#[test]
fn new_and_mock_build() {
    let dir = std::env::temp_dir().join(format!("swiftwin-cli-new-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (ok, out, err) = swiftwin(&["new", "Hello", "--dir", dir.to_str().unwrap()]);
    assert!(ok, "{err}");
    assert!(out.contains("Created"));
    let proj = dir.join("Hello");
    let (ok, out, err) = swiftwin(&["--mock", "--json", "build", "--project", proj.to_str().unwrap()]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["bundle_id"], "com.example.hello");
    assert!(proj.join("build/debug/Hello.app/Info.plist").exists());
    // --ipa: an unsigned .ipa next to the .app, for a sideloader.
    let (ok, out, err) = swiftwin(&["--mock", "build", "--ipa", "--project", proj.to_str().unwrap()]);
    assert!(ok, "{err}");
    assert!(out.contains("Hello.ipa"), "{out}");
    let ipa = proj.join("build/debug/Hello.ipa");
    let mut z = zip::ZipArchive::new(std::fs::File::open(&ipa).unwrap()).unwrap();
    assert!(z.by_name("Payload/Hello.app/Info.plist").is_ok());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn new_project_gets_a_free_apple_id_bundle_id() {
    let home = std::env::temp_dir().join(format!("swiftwin-cli-appleid-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.json"), r#"{"signing":"apple-id","id_prefix":"dev.swiftwin.utest"}"#).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_swiftwin"))
        .args(["new", "My_App", "--dir", home.to_str().unwrap()])
        .env("SWIFTWIN_HOME", &home)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let toml = std::fs::read_dir(&home)
        .unwrap()
        .flatten()
        .find_map(|e| std::fs::read_to_string(e.path().join("swiftwin.toml")).ok())
        .expect("project created");
    assert!(toml.contains("bundle_id = \"dev.swiftwin.utest.myapp\""), "{toml}");
    std::fs::remove_dir_all(home).ok();
}

#[test]
fn sdk_list_without_sdk() {
    let home = std::env::temp_dir().join(format!("swiftwin-cli-home-{}", std::process::id()));
    let out = Command::new(env!("CARGO_BIN_EXE_swiftwin")).args(["sdk", "list"]).env("SWIFTWIN_HOME", &home).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("No iOS SDK yet"));
}

#[test]
fn mock_run_builds_signs_installs() {
    let dir = std::env::temp_dir().join(format!("swiftwin-cli-run-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(swiftwin(&["new", "Hello", "--dir", dir.to_str().unwrap()]).0);
    let proj = dir.join("Hello");
    let (ok, out, err) = swiftwin(&["--mock", "--json", "run", "--project", proj.to_str().unwrap()]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["team_id"], "MOCKTEAM01");
    assert!(proj.join("build/debug/Hello.ipa").exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn mock_mirror_saves_frames() {
    let dir = std::env::temp_dir().join(format!("swiftwin-cli-mirror-{}", std::process::id()));
    let (ok, out, err) = swiftwin(&["--mock", "mirror", "--seconds", "1", "--out", dir.to_str().unwrap()]);
    assert!(ok, "{err}");
    assert!(out.contains("active source: Usb"), "{out}");
    assert!(std::fs::read_dir(&dir).unwrap().count() >= 1);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn mcp_over_stdio() {
    use std::io::{BufRead, BufReader, Write};
    // Own data dir, so an open SwiftWin app on this machine (its app-bridge.json) can't take the calls.
    let home = std::env::temp_dir().join(format!("swiftwin-cli-mcp-{}", std::process::id()));
    let mut child = Command::new(env!("CARGO_BIN_EXE_swiftwin"))
        .args(["--mock", "mcp"])
        .env("SWIFTWIN_HOME", &home)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut ask = |msg: &str| -> serde_json::Value {
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let init = ask(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#);
    assert_eq!(init["result"]["serverInfo"]["name"], "swiftwin");
    let tree = ask(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ui_tree","arguments":{}}}"#);
    assert!(tree["result"]["content"][0]["text"].as_str().unwrap().contains("SpringBoard"));
    drop(stdin);
    assert!(child.wait().unwrap().success());
    std::fs::remove_dir_all(home).ok();
}

#[test]
fn mcp_skips_a_dead_app_bridge() {
    use std::io::{BufRead, BufReader, Write};
    // app-bridge.json left behind by a crashed app: its port no longer accepts connections.
    let home = std::env::temp_dir().join(format!("swiftwin-cli-deadbridge-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    std::fs::write(home.join("app-bridge.json"), format!(r#"{{"port":{port},"token":"stale","pid":1}}"#)).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_swiftwin"))
        .args(["--mock", "mcp"])
        .env("SWIFTWIN_HOME", &home)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut ask = |msg: &str| -> serde_json::Value {
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    ask(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#);
    let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ui_tree","arguments":{}}}"#;
    // The first call may pay for the refused connection (~2 s on Windows); later ones must not.
    assert!(ask(call)["result"]["content"][0]["text"].as_str().unwrap().contains("SpringBoard"), "falls back to this process");
    let t = std::time::Instant::now();
    for _ in 0..3 {
        assert!(ask(call)["result"]["content"][0]["text"].as_str().unwrap().contains("SpringBoard"));
    }
    assert!(t.elapsed() < std::time::Duration::from_millis(1500), "dead bridge retried: {:?}", t.elapsed());
    drop(stdin);
    assert!(child.wait().unwrap().success());
    std::fs::remove_dir_all(home).ok();
}

#[test]
fn mock_release_is_a_dry_run() {
    let dir = std::env::temp_dir().join(format!("swiftwin-cli-rel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(swiftwin(&["new", "Shop", "--dir", dir.to_str().unwrap()]).0);
    let proj = dir.join("Shop");
    let (ok, out, err) = swiftwin(&["--mock", "release", "--project", proj.to_str().unwrap()]);
    assert!(ok, "{err}");
    assert!(out.contains("Dry run"));
    assert!(proj.join("build/release-preview/.github/workflows/swiftwin-release.yml").exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn mock_doctor_passes() {
    let (ok, out, err) = swiftwin(&["--mock", "doctor"]);
    assert!(ok, "{err}");
    assert!(out.contains("PASS  Apple USB driver"), "{out}");
    assert!(out.contains("WARN  AirPlay receiver (optional)"));
    assert!(out.contains("No failures"));
}

/// An account is optional: only the cloud features (release, upload) need one.
#[test]
fn only_cloud_commands_need_an_account() {
    let home = std::env::temp_dir().join(format!("swiftwin-cli-noacct-{}", std::process::id()));
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_swiftwin"))
            .args(args)
            .env("SWIFTWIN_HOME", &home)
            .env("SWIFTWIN_SUPABASE_URL", "https://example.supabase.co")
            .env("SWIFTWIN_SUPABASE_KEY", "sb_publishable_test")
            .env_remove("SWIFTWIN_MOCK")
            .output()
            .expect("run swiftwin")
    };
    let out = run(&["sdk", "list"]);
    assert!(out.status.success(), "a plain command works signed out: {}", String::from_utf8_lossy(&out.stderr));
    for cloud in [&["release", "--dry-run"][..], &["upload"][..]] {
        let out = run(cloud);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success() && err.contains("need a SwiftWin account"), "{cloud:?}: {err}");
    }
}
