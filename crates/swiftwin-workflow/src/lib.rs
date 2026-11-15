//! High-level workflows: build → sign → package → install → launch, and
//! preparing WebDriverAgent. One `Workspace` per connected phone; the CLI,
//! the MCP server and the desktop app all go through it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use swiftwin_builder::{BuildOptions, BuildOutput, Project};
use swiftwin_control::Phone;
use swiftwin_core::config::Config;
use swiftwin_core::{AppInfo, Result, SwiftWinError};
use swiftwin_device::LaunchOptions;
use swiftwin_signer::{secrets, CodeSigner, Rcodesign, SigningService};

pub struct Workspace {
    pub phone: Phone,
    /// What a running build / install_run is doing right now, for the app to show.
    step: std::sync::Mutex<Option<String>>,
    /// Held while a build, install_run or WebDriverAgent setup runs: the app's Run button and
    /// Claude's install_run share this workspace, and two at once write the same build folder.
    job: std::sync::Mutex<()>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    pub bundle_id: String,
    pub pid: u32,
    pub app: PathBuf,
    pub ipa: PathBuf,
    pub team_id: String,
    pub warnings: Vec<String>,
    /// swiftc and linker output (warnings).
    pub compiler_log: String,
    /// Seconds per step, in order: compile, sign + install (one step with a free Apple ID), launch.
    pub timings: Vec<(String, f64)>,
}

impl RunResult {
    /// "compile 41 s, sign + install 12 s, launch 3 s"
    pub fn timing_text(&self) -> String {
        self.timings.iter().map(|(k, v)| format!("{k} {v:.0} s")).collect::<Vec<_>>().join(", ")
    }
}

impl Workspace {
    pub fn new(phone: Phone) -> Self {
        Workspace { phone, step: Default::default(), job: Default::default() }
    }

    /// The step a running build / install_run is at ("Compiling…"), if any.
    pub fn step(&self) -> Option<String> {
        self.step.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn set_step(&self, s: Option<String>) {
        *self.step.lock().unwrap_or_else(|p| p.into_inner()) = s;
    }

    /// Refuse to start while another build / install_run / WebDriverAgent setup runs.
    fn start_job(&self) -> Result<std::sync::MutexGuard<'_, ()>> {
        match self.job.try_lock() {
            Ok(g) => Ok(g),
            Err(std::sync::TryLockError::Poisoned(p)) => Ok(p.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => Err(SwiftWinError::invalid(match self.step() {
                Some(step) => format!("another build is running ({step}): wait for it to finish, then try again"),
                None => "another build is running: wait for it to finish, then try again".to_string(),
            })),
        }
    }

    pub fn is_mock(&self) -> bool {
        self.phone.is_mock
    }

    /// Compile the project (fake compiler on the mock phone).
    pub fn build(&self, project_dir: &Path, release: bool) -> Result<BuildOutput> {
        let _job = self.start_job()?;
        let r = self.build_inner(project_dir, release);
        self.set_step(None);
        r
    }

    fn build_inner(&self, project_dir: &Path, release: bool) -> Result<BuildOutput> {
        let project = Project::load(project_dir)?;
        if self.is_mock() {
            self.set_step(Some(format!("Compiling {}…", project.app().name)));
            let opts = BuildOptions { release, sdk_path: "mock.sdk".into(), sdk_version: "26.1".into(), module_cache: None };
            return swiftwin_builder::build(&project, &opts, &swiftwin_builder::FakeCompiler);
        }
        let cfg = Config::load();
        let sdk = swiftwin_sdk::active(&swiftwin_core::paths::sdk_dir(), cfg.sdk.as_deref())?;
        let swiftc = swiftwin_builder::compile::find_swiftc(cfg.swift_bin.as_deref()).ok_or_else(|| SwiftWinError::ToolMissing {
            tool: "Swift toolchain".into(),
            hint: "install it from https://www.swift.org/install/windows/".into(),
        })?;
        let cache = swiftwin_builder::shared_module_cache(&sdk.version);
        self.set_step(Some(if swiftwin_builder::module_cache_is_cold(&cache) {
            format!("Compiling {} – first build with iOS SDK {}: its modules are compiled too (a few minutes, once)…", project.app().name, sdk.version)
        } else {
            format!("Compiling {}…", project.app().name)
        }));
        let opts = BuildOptions { release, sdk_path: sdk.sdk_path.clone(), sdk_version: sdk.version.clone(), module_cache: Some(cache) };
        swiftwin_builder::build(&project, &opts, &swiftwin_builder::Swiftc { path: swiftc })
    }

    fn rcodesign() -> Result<Rcodesign> {
        let exe = swiftwin_signer::tools::find_rcodesign().ok_or_else(|| SwiftWinError::ToolMissing {
            tool: "rcodesign".into(),
            hint: "run `swiftwin doctor --fix` to download it".into(),
        })?;
        Ok(Rcodesign { exe })
    }

    fn real_signer() -> Result<(SigningService, Box<dyn CodeSigner>)> {
        let store: Arc<dyn secrets::SecretStore> = Arc::from(secrets::platform_store());
        let svc = SigningService::from_config(&Config::load(), store)?;
        Ok((svc, Box::new(Self::rcodesign()?)))
    }

    /// Signing with a free Apple ID through plumesign (`swiftwin signing apple-id`).
    pub fn uses_apple_id(&self) -> bool {
        !self.is_mock() && Config::load().uses_apple_id()
    }

    /// usbmuxd's DeviceID for this phone: plumesign's `--udid` takes it, not the UDID.
    fn usbmux_device_id(&self) -> Result<u64> {
        use swiftwin_device::real::usbmux::{same_udid, Usbmux};
        let udid = self.phone.device.info()?.udid;
        let mut devices = Usbmux::platform().list_devices()?;
        devices.sort_by_key(|d| !d.connection_type.eq_ignore_ascii_case("usb"));
        devices.into_iter().find(|d| same_udid(&d.udid, &udid)).map(|d| d.device_id).ok_or(SwiftWinError::DeviceNotFound(udid))
    }

    /// Free Apple ID: plumesign signs the unsigned `.ipa` as `bundle_id`, registers the
    /// phone and installs it in one step (`signed_out` also keeps the signed `.ipa`).
    /// Returns the installed app and the unsigned `.ipa`.
    fn apple_id_install(&self, app: &Path, bundle_id: &str, signed_out: Option<&Path>) -> Result<(AppInfo, PathBuf)> {
        let ipa = app.with_extension("ipa");
        swiftwin_builder::package_unsigned_ipa(app, &ipa)?;
        let device_id = self.usbmux_device_id()?;
        swiftwin_signer::plumesign::Plumesign::find()?.sign_and_install(&ipa, bundle_id, device_id, signed_out).map_err(|e| {
            let msg = e.to_string();
            if swiftwin_signer::plumesign::is_connection_error(&msg) {
                SwiftWinError::tool("plumesign", format!("{msg}\nThe connection dropped three times: check the internet connection and run it again."))
            } else {
                SwiftWinError::tool(
                    "plumesign",
                    format!("{msg}\nIf Apple says the bundle id is not available, set a unique `bundle_id` in swiftwin.toml."),
                )
            }
        })?;
        Ok((self.installed_as(bundle_id)?, ipa))
    }

    /// The app plumesign installed: `bundle_id` itself, or an id that extends it
    /// (signers may append the team id for free accounts).
    pub fn installed_as(&self, bundle_id: &str) -> Result<AppInfo> {
        let apps = self.phone.device.list_apps()?;
        let longer = format!("{bundle_id}.");
        apps.iter()
            .find(|a| a.bundle_id == bundle_id)
            .or_else(|| apps.iter().find(|a| a.bundle_id.starts_with(&longer)))
            .cloned()
            .ok_or_else(|| SwiftWinError::NotFound(format!("plumesign finished, but {bundle_id} is not installed on the phone")))
    }

    /// Sign an `.app` for the connected phone. Returns the team id used.
    pub fn sign(&self, app: &Path) -> Result<String> {
        let info = self.phone.device.info()?;
        if self.is_mock() {
            return Ok(swiftwin_signer::mock_sign(app, &info.udid)?.team_id);
        }
        if self.uses_apple_id() {
            return Err(SwiftWinError::Unsupported(
                "with a free Apple ID, plumesign signs while installing: use `swiftwin run` (or `swiftwin build --ipa`)".into(),
            ));
        }
        let (svc, signer) = Self::real_signer()?;
        let profile = svc.sign_app(app, &info.udid, &info.name, signer.as_ref())?;
        let mut cfg = Config::load();
        if cfg.team_id.as_deref() != Some(profile.team_id.as_str()) {
            cfg.team_id = Some(profile.team_id.clone());
            cfg.save()?;
        }
        Ok(profile.team_id)
    }

    /// Build, sign, package, install and launch. Logs go to the device log.
    pub fn install_run(&self, project_dir: &Path, release: bool) -> Result<RunResult> {
        let _job = self.start_job()?;
        let r = self.install_run_inner(project_dir, release);
        self.set_step(None);
        r
    }

    fn install_run_inner(&self, project_dir: &Path, release: bool) -> Result<RunResult> {
        let mut timings = Vec::new();
        let mut t = std::time::Instant::now();
        let mut lap = |name: &str, timings: &mut Vec<(String, f64)>| {
            timings.push((name.to_string(), t.elapsed().as_secs_f64()));
            t = std::time::Instant::now();
        };
        let out = self.build_inner(project_dir, release)?;
        lap("compile", &mut timings);
        let (installed, ipa, team_id) = if self.uses_apple_id() {
            self.set_step(Some("Signing with your Apple ID and installing on the iPhone…".into()));
            let (installed, ipa) = self.apple_id_install(&out.app, &out.bundle_id, None)?;
            (installed, ipa, "Apple ID".to_string())
        } else {
            self.set_step(Some("Signing…".into()));
            let team_id = self.sign(&out.app)?;
            let ipa = out.app.with_extension("ipa");
            swiftwin_builder::package_ipa(&out.app, &ipa)?;
            lap("sign", &mut timings);
            self.set_step(Some("Installing on the iPhone…".into()));
            (self.phone.device.install(&ipa)?, ipa, team_id)
        };
        lap(if self.uses_apple_id() { "sign + install" } else { "install" }, &mut timings);
        self.set_step(Some(format!("Launching {}…", installed.bundle_id)));
        let pid = self
            .phone
            .device
            .launch(&installed.bundle_id, &LaunchOptions { kill_existing: true, ..Default::default() })?;
        lap("launch", &mut timings);
        Ok(RunResult {
            bundle_id: installed.bundle_id,
            pid,
            app: out.app,
            ipa,
            team_id,
            warnings: out.warnings,
            compiler_log: out.compiler_log,
            timings,
        })
    }

    /// Download appium's WebDriverAgent runner, re-bundle it for the user's
    /// team, sign it, install it and remember its bundle id. With a free Apple ID,
    /// plumesign signs and installs it under `<id_prefix>.WebDriverAgentRunner.xctrunner`.
    // UNTESTED-ON-DEVICE: the App Store Connect branch. The free-Apple-ID branch (download, plumesign,
    // rcodesign re-sign, install) ran on an iPhone with iOS 27.0.1 and WDA started.
    pub fn prepare_wda(&self, runner_zip: Option<&Path>, progress: &mut dyn FnMut(&str)) -> Result<AppInfo> {
        let _job = self.start_job()?;
        if self.is_mock() {
            progress("The mock phone is controlled directly; WebDriverAgent is not needed.");
            return Ok(AppInfo { bundle_id: swiftwin_core::config::DEFAULT_WDA_BUNDLE_ID.into(), name: "WebDriverAgent".into(), version: None });
        }
        let cfg = Config::load();
        // What signing needs, checked before downloading anything.
        let apple_id_prefix = match (cfg.uses_apple_id(), cfg.id_prefix.clone()) {
            (true, Some(p)) => Some(p),
            (true, None) => return Err(SwiftWinError::Auth("run `swiftwin signing apple-id --email <your Apple ID>` first".into())),
            (false, _) => None,
        };
        if apple_id_prefix.is_none() && cfg.team_id.is_none() {
            return Err(SwiftWinError::Auth("team id unknown: run `swiftwin signing setup ... --team-id <TEAMID>`".into()));
        }
        let zip = match runner_zip {
            Some(p) => std::fs::read(p)?,
            None => {
                progress(&format!("Downloading WebDriverAgent {} ...", swiftwin_signer::tools::WDA_VERSION));
                swiftwin_signer::tools::download_wda()?
            }
        };
        let work = swiftwin_core::paths::tools_dir().join("wda");
        if let Some(prefix) = apple_id_prefix {
            let app = swiftwin_signer::wda::unpack_runner(&zip, &work)?;
            let runner_id = swiftwin_signer::plumesign::wda_runner_id(&prefix);
            progress(&format!("Registering {runner_id} with your Apple ID (plumesign) ..."));
            let signed = work.join("plumesign-signed.ipa");
            self.apple_id_install(&app, &runner_id, Some(&signed))?;
            // plumesign leaves PlugIns/WebDriverAgentRunner.xctest unsigned, so XCTest could
            // not load it: sign the whole runner again with plumesign's identity and profile.
            progress("Signing the test bundle with the same certificate (rcodesign) ...");
            let resigned = work.join("resigned");
            let _ = std::fs::remove_dir_all(&resigned);
            swiftwin_signer::tools::unzip_all(&std::fs::read(&signed)?, &resigned)?;
            let app = swiftwin_signer::wda::find_runner_app(&resigned)
                .ok_or_else(|| SwiftWinError::invalid("no WebDriverAgentRunner .app in plumesign's signed .ipa"))?;
            let (identity, profile) = swiftwin_signer::plumesign::identity_for(&app)?;
            // listTeams' `xcodeFreeOnly` is not shown by plumesign; a profile valid for a year
            // means the Apple ID has a paid team, so the app can suggest the paid mode.
            if profile.is_paid_team() {
                let mut c = Config::load();
                c.paid_team_hint = Some(profile.team_id.clone());
                c.save()?;
            }
            swiftwin_signer::sign_with(&app, &runner_id, &identity, &profile, &Self::rcodesign()?)?;
            let ipa = work.join("WebDriverAgentRunner.ipa");
            swiftwin_builder::package_ipa(&app, &ipa)?;
            progress("Installing WebDriverAgent on the phone ...");
            return self.phone.install_wda(&ipa);
        }
        let team = cfg.team_id.clone().unwrap_or_default();
        let (app, runner_id) = swiftwin_signer::wda::prepare_runner(&zip, &work, &team)?;
        progress(&format!("Signing {runner_id} ..."));
        self.sign(&app)?;
        let ipa = work.join("WebDriverAgentRunner.ipa");
        swiftwin_builder::package_ipa(&app, &ipa)?;
        progress("Installing WebDriverAgent on the phone ...");
        self.phone.install_wda(&ipa)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_install_run_end_to_end() {
        let dir = std::env::temp_dir().join(format!("swiftwin-wf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let proj = swiftwin_builder::scaffold::new_project(&dir, "Counter", Some("com.example.counter")).unwrap();
        let ws = Workspace::new(Phone::mock());
        let r = ws.install_run(&proj, false).unwrap();
        assert_eq!(r.bundle_id, "com.example.counter");
        assert_eq!(r.team_id, "MOCKTEAM01");
        assert!(r.ipa.exists());
        let steps: Vec<&str> = r.timings.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(steps, ["compile", "sign", "install", "launch"]);
        assert!(ws.step().is_none(), "no step left over after the run");
        assert!(r.app.join("embedded.mobileprovision").exists());
        // The app is now in the foreground on the mock phone.
        let tree = ws.phone.control.ui_tree().unwrap();
        assert_eq!(tree.label.as_deref(), Some("Counter"));
        assert!(ws.phone.device.list_apps().unwrap().iter().any(|a| a.bundle_id == "com.example.counter"));
        // The free-Apple-ID path finds the installed app by its id, or an id extending it.
        assert_eq!(ws.installed_as("com.example.counter").unwrap().bundle_id, "com.example.counter");
        assert_eq!(ws.installed_as("com.example").unwrap().bundle_id, "com.example.counter");
        assert!(ws.installed_as("com.example.count").is_err(), "a prefix that is not a whole component");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_second_build_is_refused_while_one_runs() {
        let dir = std::env::temp_dir().join(format!("swiftwin-wf-busy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let proj = swiftwin_builder::scaffold::new_project(&dir, "Busy", Some("com.example.busy")).unwrap();
        let ws = Workspace::new(Phone::mock());
        {
            let _running = ws.start_job().unwrap();
            let e = ws.install_run(&proj, false).unwrap_err().to_string();
            assert!(e.contains("another build is running"), "{e}");
            assert!(ws.build(&proj, false).is_err());
        }
        assert!(ws.build(&proj, false).is_ok(), "free again once the first one ends");
        std::fs::remove_dir_all(dir).ok();
    }
}
