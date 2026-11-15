//! First-run setup ("Set up everything"): Apple's USB service, Python, Swift 6.2.4, LLVM,
//! SwiftWin's helper tools and the iOS SDK. Shared by the desktop app's Setup dialog and
//! `swiftwin setup`.
//!
//! Nothing of Apple's is bundled: iTunes comes from Apple's own servers through winget, and the
//! iOS SDK is extracted from the Xcode .xip the user downloads with their own Apple ID. Every step
//! checks first, so running setup again only does what is still missing.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use swiftwin_core::{Result, SwiftWinError};

/// What a step looks like right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Done,
    Missing,
    Working,
    Failed,
    /// Waiting for the user: Apple sign-in for the SDK, or a restart of the PC.
    Action,
}

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: &'static str,
    pub title: &'static str,
    pub state: State,
    pub detail: String,
    /// How to fix it by hand (shown when it is not done).
    pub fix: Option<String>,
}

/// One probe result.
#[derive(Debug, Clone, PartialEq)]
pub enum Probe {
    Ok(String),
    Missing(String),
    /// Installed, but needs a restart of the PC before it works.
    NeedsRestart(String),
}

pub const XCODE: &str = swiftwin_sdk::download::DEFAULT_XCODE;
/// Free space asked for before the SDK download: the .xip (~3 GB) and the extracted SDK, with room to spare.
pub const SDK_FREE_BYTES: u64 = 30_000_000_000;

/// (id, title, winget package, pinned version). The versions are the ones the owner's PC runs.
const STEPS: &[(&str, &str, Option<&str>, Option<&str>)] = &[
    ("apple", "Apple's USB service (iTunes)", Some("Apple.iTunes"), None),
    ("python", "Python 3.11 or newer", Some("Python.Python.3.12"), None),
    ("swift", "Swift 6.2.4 for Windows", Some("Swift.Toolchain"), Some("6.2.4")),
    ("llvm", "LLVM 22.1.8 (the iOS linker)", Some("LLVM.LLVM"), Some("22.1.8")),
    ("helpers", "SwiftWin's helper tools", None, None),
    ("sdk", "iOS SDK (from Xcode 26.3)", None, None),
];

/// How to do a step by hand.
pub fn manual(id: &str) -> String {
    match id {
        "apple" => "Install iTunes from https://www.apple.com/itunes/ (the web version: free Apple ID signing does not work with \
                    the Microsoft Store's Apple Devices or iTunes; uninstall those first), then restart the PC."
            .into(),
        "python" => "Install Python 3.11 or newer from https://www.python.org/downloads/ (tick \"Add python.exe to PATH\").".into(),
        "swift" => "Install Swift 6.2.4 from https://www.swift.org/install/windows/ (6.2.4 matches Xcode 26.3; newer Swift can't read its SDK yet).".into(),
        "llvm" => "Install LLVM from https://github.com/llvm/llvm-project/releases (LLVM-22.1.8-win64.exe) into C:\\Program Files\\LLVM.".into(),
        "helpers" => "Run `swiftwin doctor --fix` (needs Python and an internet connection).".into(),
        _ => format!(
            "Sign in at https://developer.apple.com/download/all/ with your Apple ID (a free one works), download Xcode {XCODE} \
             ({}), then run `swiftwin sdk import <that file>`.",
            swiftwin_sdk::download::xcode_url(XCODE).rsplit('/').next().unwrap_or_default()
        ),
    }
}

/// The winget command line that installs `id`, if winget installs it.
pub fn winget_args(id: &str) -> Option<Vec<String>> {
    let (_, _, pkg, version) = STEPS.iter().find(|s| s.0 == id)?;
    let mut a: Vec<String> = ["install", "--id", (*pkg)?, "--exact", "--source", "winget"].iter().map(|s| s.to_string()).collect();
    if let Some(v) = version {
        a.extend(["--version".into(), v.to_string()]);
    }
    a.extend(["--silent", "--accept-package-agreements", "--accept-source-agreements", "--disable-interactivity"].map(String::from));
    Some(a)
}

/// The machine: probes, winget, helper installs.
pub trait SetupEnv: Send + Sync {
    fn probe(&self, id: &str) -> Probe;
    fn has_winget(&self) -> bool;
    /// Run winget with these arguments; `Err` carries winget's own message.
    fn winget(&self, args: &[String]) -> Result<()>;
    fn install_helpers(&self, progress: &dyn Fn(&str)) -> Result<()>;
    /// Pick up PATH changes made by installers (a running process keeps its old PATH).
    fn refresh_path(&self) {}
    /// Free bytes on the drive holding `dir`.
    fn free_bytes(&self, dir: &Path) -> Option<u64>;
}

fn item(id: &'static str, p: Probe) -> Item {
    let title = STEPS.iter().find(|s| s.0 == id).map(|s| s.1).unwrap_or(id);
    let (state, detail, fix) = match p {
        Probe::Ok(d) => (State::Done, d, None),
        Probe::Missing(d) => (State::Missing, d, Some(manual(id))),
        Probe::NeedsRestart(d) => (State::Action, d, Some("Restart the PC, then open SwiftWin again.".into())),
    };
    Item { id, title, state, detail, fix }
}

/// Every step's state, without changing anything.
pub fn status(env: &dyn SetupEnv) -> Vec<Item> {
    STEPS.iter().map(|s| item(s.0, env.probe(s.0))).collect()
}

/// Install whatever is missing, in order, calling `report` after every change. The SDK step is
/// left as [`State::Action`]: it needs the user's Apple sign-in ([`get_sdk`]).
pub fn run(env: &dyn SetupEnv, report: &mut dyn FnMut(&[Item])) -> Vec<Item> {
    let mut items = status(env);
    report(&items);
    for i in 0..items.len() {
        let id = items[i].id;
        if items[i].state != State::Missing {
            continue;
        }
        if id == "sdk" {
            items[i].state = State::Action;
            items[i].detail = format!("Sign in with your Apple ID so SwiftWin can download Xcode {XCODE} from Apple and keep only the iOS SDK.");
            report(&items);
            continue;
        }
        items[i].state = State::Working;
        items[i].detail = if id == "helpers" { "Installing…".into() } else { "Downloading and installing (Windows may ask for permission)…".into() };
        report(&items);
        let r = if id == "helpers" {
            env.install_helpers(&|_| {})
        } else if !env.has_winget() {
            Err(SwiftWinError::NotFound(
                "winget (Windows Package Manager) is missing: install \"App Installer\" from the Microsoft Store \
                 (https://apps.microsoft.com/detail/9NBLGGH4NNS1), then run setup again."
                    .into(),
            ))
        } else {
            env.winget(&winget_args(id).expect("winget step"))
        };
        env.refresh_path();
        let after = env.probe(id);
        items[i] = match (r, after) {
            (_, Probe::Ok(d)) => item(id, Probe::Ok(d)),
            (_, Probe::NeedsRestart(d)) => item(id, Probe::NeedsRestart(d)),
            (Err(e), Probe::Missing(_)) => Item { state: State::Failed, detail: e.to_string(), ..item(id, Probe::Missing(String::new())) },
            (Ok(()), Probe::Missing(why)) if id == "apple" => {
                item(id, Probe::NeedsRestart(format!("Installed. Restart the PC so Apple's service starts ({why}).")))
            }
            (Ok(()), Probe::Missing(why)) => Item {
                state: State::Failed,
                detail: format!("Installed, but SwiftWin can't find it yet: {why}. Close and reopen SwiftWin, or restart the PC."),
                ..item(id, Probe::Missing(String::new()))
            },
        };
        report(&items);
    }
    items
}

/// Download `url` with the user's Apple session `cookie` (kept in memory only), extract the iOS
/// SDK with `import`, and delete the .xip. A broken download resumes from its `.part` file.
// UNTESTED-ON-DEVICE: Apple's real download with a cookie from the in-app sign-in.
pub fn get_sdk(
    env: &dyn SetupEnv,
    url: &str,
    cookie: &str,
    sdk_root: &Path,
    import: &dyn Fn(&Path, &mut dyn FnMut(&str)) -> Result<swiftwin_sdk::SdkInfo>,
    progress: &mut dyn FnMut(&str, u64, Option<u64>),
) -> Result<swiftwin_sdk::SdkInfo> {
    let file = url.rsplit('/').next().filter(|f| f.ends_with(".xip")).unwrap_or("Xcode.xip");
    let dest: PathBuf = sdk_root.join("downloads").join(file);
    std::fs::create_dir_all(dest.parent().unwrap())?;
    let have = std::fs::metadata(dest.with_extension("xip.part")).map(|m| m.len()).unwrap_or(0);
    if let Some(free) = env.free_bytes(sdk_root) {
        let need = SDK_FREE_BYTES.saturating_sub(have);
        if free < need {
            return Err(SwiftWinError::invalid(format!(
                "not enough disk space: the iOS SDK needs about {} GB free on the drive with {}, and it has {} GB. Free some space and try again.",
                need / 1_000_000_000 + 1,
                sdk_root.display(),
                free / 1_000_000_000
            )));
        }
    }
    if !dest.exists() {
        swiftwin_sdk::download::download(url, &dest, Some(cookie), &mut |d, t| progress("Downloading Xcode", d, t))?;
    }
    progress("Extracting the iOS SDK (10–20 minutes)", 0, None);
    let info = import(&dest, &mut |m| progress(m, 0, None))?;
    let _ = std::fs::remove_file(&dest); // only the SDK is kept
    Ok(info)
}

/// The real PC.
#[derive(Default)]
pub struct RealSetupEnv;

fn quiet(program: &str) -> std::process::Command {
    swiftwin_device::real::pmd::command(program)
}

impl SetupEnv for RealSetupEnv {
    // UNTESTED-ON-DEVICE: installing on a clean PC (the probes ran on the owner's PC).
    fn probe(&self, id: &str) -> Probe {
        use swiftwin_core::config::Config;
        match id {
            "apple" => {
                let mux = swiftwin_device::real::usbmux::Usbmux::platform().list_devices().is_ok();
                let amds = std::env::var_os("CommonProgramFiles")
                    .map(|c| PathBuf::from(c).join("Apple").join("Mobile Device Support"))
                    .is_some_and(|p| p.is_dir());
                match (amds, mux) {
                    (true, true) => Probe::Ok("Apple Mobile Device Service is running".into()),
                    (true, false) => Probe::NeedsRestart("Apple Mobile Device Support is installed, but its service isn't answering yet".into()),
                    (false, true) => Probe::Missing("Apple's service is running from the Microsoft Store app, which free Apple ID signing can't use".into()),
                    (false, false) => Probe::Missing("Apple Mobile Device Service is not installed".into()),
                }
            }
            "python" => match swiftwin_device::real::pmd::find_python().and_then(|p| swiftwin_device::real::pmd::python_version(&p)) {
                Some((a, b)) => Probe::Ok(format!("Python {a}.{b}")),
                None => Probe::Missing("no Python 3.11 or newer found".into()),
            },
            "swift" => {
                let Some(exe) = swiftwin_builder::compile::find_swiftc(Config::load().swift_bin.as_deref()) else {
                    return Probe::Missing("swiftc not found".into());
                };
                let out = swiftwin_builder::compile::swift_command(&exe).arg("--version").output();
                let text = out.map(|o| format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))).unwrap_or_default();
                let line = text.lines().find(|l| l.contains("Swift version")).unwrap_or("").trim().to_string();
                if line.contains("Swift version 6.2.4") {
                    Probe::Ok(line)
                } else if line.is_empty() {
                    Probe::Missing(format!("{} does not run", exe.display()))
                } else {
                    Probe::Missing(format!("found {line}; SwiftWin needs 6.2.4, which matches Xcode {XCODE}"))
                }
            }
            "llvm" => {
                let swiftc = swiftwin_builder::compile::find_swiftc(Config::load().swift_bin.as_deref()).unwrap_or_else(|| "swiftc".into());
                match swiftwin_builder::compile::find_ld64(&swiftc) {
                    Some(p) => Probe::Ok(format!("ld64.lld at {}", p.display())),
                    None => Probe::Missing("no ld64.lld linker found".into()),
                }
            }
            "helpers" => {
                let missing: Vec<&str> = [
                    ("pymobiledevice3", swiftwin_device::real::pmd::locate().is_some()),
                    ("plumesign", swiftwin_signer::plumesign::find_plumesign().is_some()),
                    ("rcodesign", swiftwin_signer::tools::find_rcodesign().is_some()),
                ]
                .iter()
                .filter(|t| !t.1)
                .map(|t| t.0)
                .collect();
                if missing.is_empty() {
                    Probe::Ok("pymobiledevice3, plumesign and rcodesign are installed".into())
                } else {
                    Probe::Missing(format!("missing: {}", missing.join(", ")))
                }
            }
            _ => match swiftwin_sdk::active(&swiftwin_core::paths::sdk_dir(), Config::load().sdk.as_deref()) {
                Ok(s) => Probe::Ok(format!("{} at {}", s.name, s.sdk_path.display())),
                Err(_) => Probe::Missing("no iOS SDK imported yet".into()),
            },
        }
    }

    fn has_winget(&self) -> bool {
        quiet("winget").arg("--version").output().is_ok_and(|o| o.status.success())
    }

    // UNTESTED-ON-DEVICE: winget installs of iTunes, Swift and LLVM on a clean PC (UAC prompts).
    fn winget(&self, args: &[String]) -> Result<()> {
        let out = quiet("winget").args(args).output()?;
        let text = String::from_utf8_lossy(&out.stdout).replace('\r', "\n");
        if out.status.success() || text.contains("already installed") || text.contains("No newer package") {
            return Ok(());
        }
        let code = out.status.code().unwrap_or(-1) as u32;
        let hint = match code {
            0x8A15_0011 | 0x8A15_0010 => " (Windows' permission prompt was declined, or the installer failed)",
            0x8A15_0014 => " (no package found: check the internet connection)",
            0x8A15_0109 | 0x8A15_010A => " (restart the PC to finish)",
            _ => "",
        };
        let tail: Vec<&str> = text.lines().map(str::trim).filter(|l| l.len() > 2 && !l.chars().all(|c| "-\\|/█▒ ".contains(c))).collect();
        let tail = tail[tail.len().saturating_sub(2)..].join(" ");
        Err(SwiftWinError::tool("winget", format!("{} failed with 0x{code:08X}{hint}: {tail}", args.get(2).map(String::as_str).unwrap_or("install"))))
    }

    fn install_helpers(&self, progress: &dyn Fn(&str)) -> Result<()> {
        if swiftwin_device::real::pmd::locate().is_none() {
            swiftwin_device::real::pmd::install(&mut |m| progress(m))?;
        }
        swiftwin_signer::setup::ensure_apple_id_tools(progress)?;
        Ok(())
    }

    fn refresh_path(&self) {
        // shortcut: PowerShell reads the registry PATH; fine for a step that already waited minutes on an installer.
        let out = quiet("powershell")
            .args(["-NoProfile", "-Command", "[Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User')"])
            .output();
        if let Some(o) = out.ok().filter(|o| o.status.success()) {
            let fresh = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let old = std::env::var("PATH").unwrap_or_default();
            std::env::set_var("PATH", format!("{fresh};{old}"));
        }
    }

    fn free_bytes(&self, dir: &Path) -> Option<u64> {
        let root = dir.ancestors().last()?.to_string_lossy().replace('\'', "''");
        let out = quiet("powershell")
            .args(["-NoProfile", "-Command", &format!("[System.IO.DriveInfo]::new('{root}').AvailableFreeSpace")])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }
}

/// A pretend PC for `--mock` and tests: `installed` steps are done; winget installs what it is asked
/// for, unless it is in `broken`.
pub struct FakeSetupEnv {
    pub installed: Mutex<Vec<String>>,
    pub broken: Vec<String>,
    pub winget: bool,
    pub free: u64,
    pub calls: Mutex<Vec<String>>,
}

impl FakeSetupEnv {
    pub fn new(installed: &[&str]) -> Self {
        FakeSetupEnv {
            installed: Mutex::new(installed.iter().map(|s| s.to_string()).collect()),
            broken: vec![],
            winget: true,
            free: 100_000_000_000,
            calls: Mutex::new(vec![]),
        }
    }
    fn add(&self, id: &str) {
        self.installed.lock().unwrap().push(id.into());
    }
}

impl SetupEnv for FakeSetupEnv {
    fn probe(&self, id: &str) -> Probe {
        if self.installed.lock().unwrap().iter().any(|i| i == id) {
            Probe::Ok(format!("{id} (simulated)"))
        } else {
            Probe::Missing(format!("{id} is not installed (simulated)"))
        }
    }
    fn has_winget(&self) -> bool {
        self.winget
    }
    fn winget(&self, args: &[String]) -> Result<()> {
        self.calls.lock().unwrap().push(args.join(" "));
        let id = STEPS.iter().find(|s| s.2 == args.get(2).map(String::as_str)).map(|s| s.0).unwrap_or("?");
        if self.broken.iter().any(|b| b == id) {
            return Err(SwiftWinError::tool("winget", "install failed with 0x8A150011 (simulated)"));
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
        self.add(id);
        Ok(())
    }
    fn install_helpers(&self, _: &dyn Fn(&str)) -> Result<()> {
        self.calls.lock().unwrap().push("helpers".into());
        if !self.installed.lock().unwrap().iter().any(|i| i == "python") {
            return Err(SwiftWinError::ToolMissing { tool: "Python 3.11+".into(), hint: "install Python first".into() });
        }
        self.add("helpers");
        Ok(())
    }
    fn free_bytes(&self, _: &Path) -> Option<u64> {
        Some(self.free)
    }
}

impl FakeSetupEnv {
    /// Mark the SDK imported (mock download finished).
    pub fn finish_sdk(&self) {
        self.add("sdk");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn states(items: &[Item]) -> Vec<(&str, State)> {
        items.iter().map(|i| (i.id, i.state)).collect()
    }

    #[test]
    fn installs_only_what_is_missing_and_leaves_the_sdk_to_the_user() {
        let env = FakeSetupEnv::new(&["apple", "python"]);
        let mut reports = 0;
        let items = run(&env, &mut |_| reports += 1);
        assert!(reports > 3);
        assert_eq!(
            states(&items),
            vec![
                ("apple", State::Done),
                ("python", State::Done),
                ("swift", State::Done),
                ("llvm", State::Done),
                ("helpers", State::Done),
                ("sdk", State::Action)
            ]
        );
        let calls = env.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 3, "{calls:?}");
        assert!(calls[0].contains("Swift.Toolchain --exact --source winget --version 6.2.4 --silent"), "{}", calls[0]);
        assert!(calls[1].contains("LLVM.LLVM --exact --source winget --version 22.1.8"));
        // Running again changes nothing.
        let again = run(&env, &mut |_| {});
        assert_eq!(env.calls.lock().unwrap().len(), 3);
        assert_eq!(states(&again), states(&items));
    }

    #[test]
    fn failures_explain_the_fix_and_the_rest_continues() {
        let mut env = FakeSetupEnv::new(&[]);
        env.broken = vec!["python".into()];
        let items = run(&env, &mut |_| {});
        let python = &items[1];
        assert_eq!(python.state, State::Failed);
        assert!(python.detail.contains("0x8A150011") && python.fix.as_deref().unwrap().contains("python.org"));
        assert_eq!(items[2].state, State::Done, "Swift still installs");
        assert_eq!(items[4].state, State::Failed, "helpers need Python");
    }

    #[test]
    fn no_winget_says_how_to_get_it() {
        let mut env = FakeSetupEnv::new(&["helpers", "sdk"]);
        env.winget = false;
        let items = run(&env, &mut |_| {});
        assert!(items[..4].iter().all(|i| i.state == State::Failed && i.detail.contains("App Installer")));
        assert!(env.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn itunes_installed_but_service_not_running_asks_for_a_restart() {
        struct NeedsReboot;
        impl SetupEnv for NeedsReboot {
            fn probe(&self, id: &str) -> Probe {
                if id == "apple" { Probe::Missing("not answering".into()) } else { Probe::Ok("ok".into()) }
            }
            fn has_winget(&self) -> bool { true }
            fn winget(&self, _: &[String]) -> Result<()> { Ok(()) }
            fn install_helpers(&self, _: &dyn Fn(&str)) -> Result<()> { Ok(()) }
            fn free_bytes(&self, _: &Path) -> Option<u64> { None }
        }
        let items = run(&NeedsReboot, &mut |_| {});
        assert_eq!(items[0].state, State::Action);
        assert!(items[0].fix.as_deref().unwrap().contains("Restart the PC"));
    }

    #[test]
    fn sdk_download_checks_space_imports_and_deletes_the_xip() {
        let dir = std::env::temp_dir().join(format!("swiftwin-setup-sdk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut env = FakeSetupEnv::new(&[]);
        env.free = 1_000_000_000;
        let import = |_: &Path, _: &mut dyn FnMut(&str)| -> Result<swiftwin_sdk::SdkInfo> { unreachable!() };
        let e = get_sdk(&env, "http://127.0.0.1:9/Xcode_26.3_Apple_silicon.xip", "c", &dir, &import, &mut |_, _, _| {}).unwrap_err();
        assert!(e.to_string().contains("disk space"), "{e}");

        // An already finished download goes straight to the import, and the .xip is deleted after.
        env.free = SDK_FREE_BYTES;
        let xip = dir.join("downloads").join("Xcode_26.3_Apple_silicon.xip");
        std::fs::write(&xip, b"xip").unwrap();
        let import = |p: &Path, _: &mut dyn FnMut(&str)| -> Result<swiftwin_sdk::SdkInfo> {
            assert!(p.ends_with("Xcode_26.3_Apple_silicon.xip"));
            Ok(swiftwin_sdk::SdkInfo {
                name: "iPhoneOS26.2".into(),
                version: "26.2".into(),
                root: p.into(),
                sdk_path: p.into(),
                artifact_bundle: p.into(),
                swift_lib_dir: None,
                clang_rt: None,
            })
        };
        let mut phases = vec![];
        let info = get_sdk(&env, "http://127.0.0.1:9/Xcode_26.3_Apple_silicon.xip", "c", &dir, &import, &mut |m, _, _| phases.push(m.to_string())).unwrap();
        assert_eq!(info.name, "iPhoneOS26.2");
        assert!(!xip.exists(), "the .xip is deleted");
        assert!(phases[0].starts_with("Extracting"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn manual_fixes_point_to_official_sources() {
        assert!(manual("apple").contains("apple.com/itunes"));
        assert!(manual("sdk").contains("Xcode_26.3_Apple_silicon.xip"));
        assert_eq!(winget_args("helpers"), None);
        assert!(winget_args("apple").unwrap().join(" ").starts_with("install --id Apple.iTunes --exact"));
    }
}
