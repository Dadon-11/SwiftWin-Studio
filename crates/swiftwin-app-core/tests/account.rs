//! The app's sign-in, GitHub connection and Upload commands with `--mock` (fake services).

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use swiftwin_app_core::AppCore;

fn wait(core: &AppCore, cmd: &str, done: impl Fn(&Value) -> bool) -> Value {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let v = core.dispatch(cmd, &json!({})).unwrap();
        if done(&v) {
            return v;
        }
        assert!(Instant::now() < end, "timed out waiting on {cmd}: {v}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn sign_in_connect_github_and_upload_a_project() {
    let core = AppCore::new(true);
    let st = wait(&core, "account_status", |v| v["loading"] == false);
    assert_eq!((st["configured"].as_bool(), st["signed_in"].as_bool()), (Some(true), Some(false)));

    // Signed out: GitHub and Upload are refused.
    assert!(core.dispatch("github_connect", &json!({})).unwrap_err().to_string().contains("sign in first"));

    let e = core.dispatch("account_sign_in", &json!({"email": "demo@swiftwin.dev", "password": "nope"})).unwrap_err();
    assert!(e.to_string().contains("Wrong email or password"));
    let r = core.dispatch("account_sign_in", &json!({"email": "demo@swiftwin.dev", "password": "swiftwin123"})).unwrap();
    assert_eq!(r["user"]["email"], "demo@swiftwin.dev");
    let st = core.dispatch("account_status", &json!({})).unwrap();
    assert_eq!(st["signed_in"], true);
    assert!(!st.to_string().contains("token"), "status never carries tokens: {st}");

    let c = core.dispatch("github_connect", &json!({})).unwrap();
    assert_eq!(c["user_code"], "FAKE-CODE");
    let g = wait(&core, "github_status", |v| v["pending"].is_null());
    assert_eq!(g["login"], "octo");

    let parent = std::env::temp_dir().join(format!("swiftwin-appcore-up-{}", std::process::id()));
    std::fs::create_dir_all(&parent).unwrap();
    core.dispatch("new_project", &json!({"parent": parent, "name": "Uploady"})).unwrap();
    let p = core.dispatch("upload_preview", &json!({})).unwrap();
    assert_eq!(p["default_name"], "Uploady");
    assert!(p["repo"].is_null());
    core.dispatch("upload_start", &json!({"name": "Uploady"})).unwrap();
    let j = wait(&core, "upload_status", |v| v["running"] == false);
    assert_eq!(j["result"]["repo"], "octo/Uploady", "{j}");
    assert_eq!(j["result"]["created"], true);
    assert_eq!(j["result"]["url"], "https://github.com/octo/Uploady");
    assert_eq!(core.dispatch("upload_preview", &json!({})).unwrap()["repo"], "octo/Uploady");

    core.dispatch("account_sign_out", &json!({})).unwrap();
    let st = core.dispatch("account_status", &json!({})).unwrap();
    assert_eq!((st["signed_in"].as_bool(), st["github"]["connected"].as_bool()), (Some(false), Some(false)));
    assert!(core.dispatch("open_url", &json!({"url": "file:///C:/Windows/notepad.exe"})).is_err());
}
