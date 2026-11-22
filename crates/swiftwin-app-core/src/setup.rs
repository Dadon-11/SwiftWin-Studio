//! The Setup dialog's commands (`setup_*`): install what this PC is missing and get the iOS SDK.
//!
//! They install software, so like `account_*` the MCP bridge refuses them (`account::is_private`).
//! The Apple session cookie for the Xcode download comes from the desktop shell's sign-in window
//! straight into [`SetupUi::start_sdk_download`]: it never passes through the web UI, is kept in
//! memory only for the download, and is never logged or returned.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use swiftwin_core::{Result, SwiftWinError};
use swiftwin_doctor::setup::{self, FakeSetupEnv, Item, RealSetupEnv, SetupEnv};

pub struct SetupUi {
    env: Arc<dyn SetupEnv>,
    /// `--mock`: the pretend PC (also what the SDK download "finishes" on).
    fake: Option<Arc<FakeSetupEnv>>,
    /// `{running, items}` of "Set up everything".
    job: Arc<Mutex<Value>>,
    /// `{running, phase, done, total, error}` of the SDK download.
    sdk: Arc<Mutex<Value>>,
}

impl SetupUi {
    pub fn new(mock: bool) -> SetupUi {
        // mock: Apple's service and Python are there, the rest installs in a moment.
        let fake = mock.then(|| Arc::new(FakeSetupEnv::new(&["apple", "python"])));
        let env: Arc<dyn SetupEnv> = match &fake {
            Some(f) => f.clone(),
            None => Arc::new(RealSetupEnv),
        };
        SetupUi { env, fake, job: Arc::new(Mutex::new(json!({"running": false}))), sdk: Arc::new(Mutex::new(json!({"running": false}))) }
    }

    fn state(&self, items: Option<Vec<Item>>) -> Value {
        let mut job = self.job.lock().unwrap().clone();
        if let Some(items) = items {
            job["items"] = json!(items);
        }
        json!({"job": job, "sdk": self.sdk.lock().unwrap().clone(), "mock": self.fake.is_some()})
    }

    pub fn dispatch(&self, cmd: &str, _args: &Value) -> Result<Value> {
        match cmd {
            // Fresh probes, unless a run is updating them.
            "setup_status" => {
                let running = self.job.lock().unwrap()["running"] == true;
                Ok(self.state((!running).then(|| setup::status(self.env.as_ref()))))
            }
            "setup_job" => Ok(self.state(None)),
            // "Choose the folder myself": for when iTunes is installed but SwiftWin can't find it.
            "setup_choose_apple_dir" => {
                if self.fake.is_some() {
                    return Err(SwiftWinError::invalid("the simulated PC has nothing to choose"));
                }
                let Some(picked) = pick_folder("Choose the folder that has AppleMobileDeviceService.exe")? else {
                    return Ok(json!({"cancelled": true}));
                };
                let found = setup::apple_support_folder(&picked).ok_or_else(|| {
                    SwiftWinError::invalid(format!(
                        "{} has no AppleMobileDeviceService.exe. It is usually in C:\\Program Files\\Common Files\\Apple\\Mobile Device Support (or the same folder under Program Files (x86))",
                        picked.display()
                    ))
                })?;
                let mut c = swiftwin_core::config::Config::load();
                c.apple_support_dir = Some(found.display().to_string());
                c.save()?;
                Ok(json!({"path": found.display().to_string()}))
            }
            "setup_run" => {
                {
                    let mut j = self.job.lock().unwrap();
                    if j["running"] == true {
                        return Ok(self.state(None));
                    }
                    *j = json!({"running": true});
                }
                let (env, job) = (self.env.clone(), self.job.clone());
                std::thread::spawn(move || {
                    let items = setup::run(env.as_ref(), &mut |items| job.lock().unwrap()["items"] = json!(items));
                    *job.lock().unwrap() = json!({"running": false, "items": items});
                });
                Ok(self.state(None))
            }
            // In the desktop app the shell opens Apple's sign-in window instead (app/src-tauri).
            "setup_sdk_signin" => match &self.fake {
                Some(_) => {
                    self.start_sdk_download(setup_url(), "ADCDownloadAuth=mock".into());
                    Ok(json!({"started": true}))
                }
                None => Err(SwiftWinError::invalid(format!(
                    "the Apple sign-in opens in the SwiftWin app; here, download Xcode {} yourself and run `swiftwin sdk import <file>`",
                    setup::XCODE
                ))),
            },
            other => Err(SwiftWinError::invalid(format!("unknown command {other}"))),
        }
    }

    /// Download Xcode from `url` with the Apple session `cookie`, import its iOS SDK and delete it.
    pub fn start_sdk_download(&self, url: String, cookie: String) {
        {
            let mut s = self.sdk.lock().unwrap();
            if s["running"] == true {
                return;
            }
            *s = json!({"running": true, "phase": "Starting the download", "done": 0});
        }
        let (env, sdk, fake) = (self.env.clone(), self.sdk.clone(), self.fake.clone());
        std::thread::spawn(move || {
            let mut progress = |phase: &str, done: u64, total: Option<u64>| {
                *sdk.lock().unwrap() = json!({"running": true, "phase": phase, "done": done, "total": total});
            };
            let r = match &fake {
                Some(f) => {
                    for i in 1..=10u64 {
                        progress("Downloading Xcode", i * 320_000_000, Some(3_200_000_000));
                        std::thread::sleep(std::time::Duration::from_millis(120));
                    }
                    progress("Extracting the iOS SDK (10–20 minutes)", 0, None);
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    f.finish_sdk();
                    Ok("iPhoneOS26.2 (simulated)".to_string())
                }
                None => {
                    let root = swiftwin_core::paths::sdk_dir();
                    let import = |xip: &std::path::Path, p: &mut dyn FnMut(&str)| {
                        swiftwin_sdk::import_xip(xip, &root, &mut |s: &swiftwin_sdk::Progress| {
                            p(&if s.files > 0 { format!("{}: {} files", s.phase, s.files) } else { s.phase.clone() })
                        })
                    };
                    setup::get_sdk(env.as_ref(), &url, &cookie, &root, &import, &mut progress).and_then(|info| {
                        let mut c = swiftwin_core::config::Config::load();
                        c.sdk = Some(info.name.clone());
                        c.save()?;
                        Ok(info.name)
                    })
                }
            };
            drop(cookie);
            *sdk.lock().unwrap() = match r {
                Ok(name) => json!({"running": false, "done_name": name}),
                Err(e) => json!({"running": false, "error": e.to_string()}),
            };
        });
    }
}

/// Windows' own "choose a folder" window, through PowerShell, so no extra dependency is needed.
/// `None` when the person cancels. The window is owned by a top-most form so it can't hide behind SwiftWin.
// UNTESTED-ON-DEVICE: only the folder checks are unit-tested; the window needs a click.
fn pick_folder(title: &str) -> Result<Option<std::path::PathBuf>> {
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; $o = New-Object System.Windows.Forms.Form -Property @{{TopMost=$true}}; \
         $d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = '{title}'; $d.ShowNewFolderButton = $false; \
         if ($d.ShowDialog($o) -eq 'OK') {{ $d.SelectedPath }}"
    );
    let out = swiftwin_device::real::pmd::command("powershell").args(["-NoProfile", "-STA", "-Command", &script]).output()?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok((!path.is_empty()).then(|| path.into()))
}

/// The page Apple's sign-in returns to: it sets the download cookie and starts the Xcode download.
/// Signed out, Apple redirects it to idmsa.apple.com's sign-in with `path=` pointing back here
/// (checked 2026-11-11).
pub fn setup_url() -> String {
    swiftwin_sdk::download::xcode_url(setup::XCODE)
}

pub fn signin_url() -> String {
    let path = setup_url().replace("https://download.developer.apple.com", "");
    format!("https://developer.apple.com/services-account/download?path={path}")
}

/// Is `url` the Xcode download SwiftWin wants (Apple's host, the pinned version)?
pub fn is_wanted_download(url: &str) -> bool {
    let file = setup_url().rsplit('/').next().unwrap_or_default().to_string();
    let universal = file.replace("_Apple_silicon", "_Universal");
    url.starts_with("https://download.developer.apple.com/")
        && url.split(['?', '#']).next().is_some_and(|p| p.ends_with(&format!("/{file}")) || p.ends_with(&format!("/{universal}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_setup_runs_and_downloads_the_sdk() {
        let ui = SetupUi::new(true);
        let st = ui.dispatch("setup_status", &json!({})).unwrap();
        assert_eq!(st["job"]["items"][0]["state"], "done");
        assert_eq!(st["job"]["items"][2]["state"], "missing");
        ui.dispatch("setup_run", &json!({})).unwrap();
        let wait = |f: &dyn Fn(&Value) -> bool| {
            for _ in 0..100 {
                let v = ui.dispatch("setup_job", &json!({})).unwrap();
                if f(&v) {
                    return v;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            panic!("timed out");
        };
        let v = wait(&|v| v["job"]["running"] == false);
        assert_eq!(v["job"]["items"][4]["state"], "done");
        assert_eq!(v["job"]["items"][5]["state"], "action");
        ui.dispatch("setup_sdk_signin", &json!({})).unwrap();
        let v = wait(&|v| v["sdk"]["running"] == false);
        assert!(v["sdk"]["done_name"].as_str().unwrap().starts_with("iPhoneOS"));
        assert!(!v.to_string().contains("ADCDownloadAuth"), "the cookie is never returned");
        let st = ui.dispatch("setup_status", &json!({})).unwrap();
        assert!(st["job"]["items"].as_array().unwrap().iter().all(|i| i["state"] == "done"));
    }

    #[test]
    fn only_the_pinned_xcode_from_apple_is_taken() {
        assert!(is_wanted_download("https://download.developer.apple.com/Developer_Tools/Xcode_26.3/Xcode_26.3_Apple_silicon.xip"));
        assert!(is_wanted_download("https://download.developer.apple.com/Developer_Tools/Xcode_26.3/Xcode_26.3_Universal.xip?x=1"));
        assert!(!is_wanted_download("https://download.developer.apple.com/Developer_Tools/Xcode_26.4/Xcode_26.4_Apple_silicon.xip"));
        assert!(!is_wanted_download("https://evil.example/Developer_Tools/Xcode_26.3/Xcode_26.3_Apple_silicon.xip"));
        assert_eq!(
            signin_url(),
            "https://developer.apple.com/services-account/download?path=/Developer_Tools/Xcode_26.3/Xcode_26.3_Apple_silicon.xip"
        );
    }
}
