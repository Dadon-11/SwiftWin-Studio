//! Prototype backend: drives the phone through the `pymobiledevice3` CLI
//! (GPL-3.0, run as a separate process; see docs/dependencies.md).
//!
//! Command shapes were checked against pymobiledevice3 11.49.5 (`--help` and
//! source): `lockdown info` and `apps list` print JSON, `developer dvt launch`
//! prints `Process launched with pid N`, `syslog live --format json` prints
//! NDJSON with `pid, timestamp, level, filename, message`.
//!
//! iOS 17+ tunnel: with no `--rsd/--tunnel/--userspace` flag, a `developer`
//! command first tries plain lockdown, gets `InvalidServiceError`, and the CLI
//! retries itself once over its in-process userspace tunnel (CoreDeviceProxy,
//! iOS 17.4+, no admin rights). iOS 17.0–17.3 falls back to `remote tunneld`
//! instead. `mounter auto-mount` on iOS 27+ raises `RSDRequiredError` and is retried the same way.
//!
//! Failures are logged to stderr as `<date> <time> <host> <logger>[pid] ERROR <message>`,
//! where the message is the exception's docstring (e.g. "Device is not paired."),
//! followed by an `INFO Run again with -v ...` line.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use swiftwin_core::{AppInfo, LogEntry, LogLevel, Result, SwiftWinError};

/// Exact version SwiftWin was written against; `swiftwin doctor` installs this one.
pub const PMD_VERSION: &str = "11.49.5";

#[derive(Debug, Clone)]
pub struct CmdOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Kills a running process; callable from any thread.
pub type Killer = Arc<dyn Fn() + Send + Sync>;

/// A line-oriented running process (syslog stream, XCTest runner).
pub trait LineStream: Send {
    /// Next stdout line, or `None` when the process ended.
    fn next_line(&mut self) -> Option<String>;
    fn killer(&self) -> Killer;
}

/// Runs pymobiledevice3. Abstracted so tests can fake it.
pub trait Runner: Send + Sync {
    fn run(&self, args: &[String]) -> Result<CmdOutput>;
    /// [`Runner::run`] with extra environment variables (fakes may ignore them).
    fn run_env(&self, args: &[String], env: &[(&str, &str)]) -> Result<CmdOutput> {
        let _ = env;
        self.run(args)
    }
    fn stream(&self, args: &[String]) -> Result<Box<dyn LineStream>>;
}

/// Find a Python >= 3.11 interpreter. Returns the command prefix (e.g. `["py", "-3"]`).
pub fn find_python() -> Option<Vec<String>> {
    let mut candidates: Vec<Vec<String>> = Vec::new();
    if let Ok(p) = std::env::var("SWIFTWIN_PYTHON") {
        candidates.push(vec![p]);
    }
    if cfg!(windows) {
        candidates.push(vec!["py".into(), "-3".into()]);
        candidates.push(vec!["python".into()]);
    } else {
        candidates.push(vec!["python3".into()]);
        candidates.push(vec!["python".into()]);
    }
    candidates.into_iter().find(|c| python_version(c).is_some_and(|(maj, min)| maj == 3 && min >= 11))
}

pub fn python_version(cmd: &[String]) -> Option<(u32, u32)> {
    let out = command(&cmd[0]).args(&cmd[1..]).arg("--version").output().ok()?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    parse_python_version(&text)
}

pub fn parse_python_version(s: &str) -> Option<(u32, u32)> {
    let v = s.trim().strip_prefix("Python ")?;
    let mut it = v.split('.');
    Some((it.next()?.parse().ok()?, it.next()?.trim().parse().ok()?))
}

fn venv_exe(name: &str) -> PathBuf {
    let v = swiftwin_core::paths::venv_dir();
    if cfg!(windows) {
        v.join("Scripts").join(format!("{name}.exe"))
    } else {
        v.join("bin").join(name)
    }
}

/// Locate the pymobiledevice3 executable: `SWIFTWIN_PYMOBILEDEVICE3`, SwiftWin's venv, then PATH.
pub fn locate() -> Option<Vec<String>> {
    if let Ok(p) = std::env::var("SWIFTWIN_PYMOBILEDEVICE3") {
        return Some(vec![p]);
    }
    let v = venv_exe("pymobiledevice3");
    if v.exists() {
        return Some(vec![v.to_string_lossy().into_owned()]);
    }
    let ok = command("pymobiledevice3").arg("--help").stdout(Stdio::null()).stderr(Stdio::null()).status();
    if ok.is_ok_and(|s| s.success()) {
        return Some(vec!["pymobiledevice3".into()]);
    }
    None
}

/// Create SwiftWin's private venv and install the pinned pymobiledevice3.
pub fn install(progress: &mut dyn FnMut(&str)) -> Result<Vec<String>> {
    let py = find_python().ok_or_else(|| SwiftWinError::ToolMissing {
        tool: "Python 3.11+".into(),
        hint: "install it from https://www.python.org/downloads/ or `winget install Python.Python.3.12`".into(),
    })?;
    let venv = swiftwin_core::paths::venv_dir();
    if !venv_exe("python").exists() {
        progress(&format!("Creating Python environment in {}", venv.display()));
        let st = command(&py[0]).args(&py[1..]).arg("-m").arg("venv").arg(&venv).status()?;
        if !st.success() {
            return Err(SwiftWinError::tool("python -m venv", "failed to create the virtual environment"));
        }
    }
    progress(&format!("Installing pymobiledevice3=={PMD_VERSION} (this takes a minute)"));
    let st = command(&venv_exe("python").to_string_lossy())
        .args(["-m", "pip", "install", "--disable-pip-version-check", "-q"])
        .arg(format!("pymobiledevice3=={PMD_VERSION}"))
        .status()?;
    if !st.success() {
        return Err(SwiftWinError::tool("pip", "installing pymobiledevice3 failed"));
    }
    Ok(vec![venv_exe("pymobiledevice3").to_string_lossy().into_owned()])
}

/// `Command` that never flashes a console window on Windows.
pub fn command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Runs the real pymobiledevice3 executable.
pub struct ProcessRunner {
    pub prefix: Vec<String>,
}

impl ProcessRunner {
    pub fn locate() -> Result<Self> {
        let prefix = locate().ok_or_else(|| SwiftWinError::ToolMissing {
            tool: "pymobiledevice3".into(),
            hint: "run `swiftwin doctor --fix` to install it into SwiftWin's own Python environment".into(),
        })?;
        Ok(ProcessRunner { prefix })
    }

    fn cmd(&self, args: &[String]) -> Command {
        let mut c = command(&self.prefix[0]);
        c.args(&self.prefix[1..]).args(args).env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1");
        c
    }
}

impl Runner for ProcessRunner {
    fn run(&self, args: &[String]) -> Result<CmdOutput> {
        self.run_env(args, &[])
    }

    fn run_env(&self, args: &[String], env: &[(&str, &str)]) -> Result<CmdOutput> {
        let out = self.cmd(args).envs(env.iter().copied()).stdin(Stdio::null()).output()?;
        Ok(CmdOutput {
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    /// stdout and stderr are merged: pymobiledevice3 logs progress and errors (e.g. everything
    /// `xcuitest` says) to stderr, and those lines are what explains a failed start.
    fn stream(&self, args: &[String]) -> Result<Box<dyn LineStream>> {
        let mut child = self.cmd(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        let (tx, rx) = std::sync::mpsc::channel();
        forward_lines(child.stdout.take().expect("piped stdout"), tx.clone());
        forward_lines(child.stderr.take().expect("piped stderr"), tx);
        Ok(Box::new(ChildLines { child: Arc::new(std::sync::Mutex::new(child)), lines: rx }))
    }
}

/// Send each line of `r` to `tx` until EOF (lossy UTF-8, so one bad byte can't end a stream).
fn forward_lines(r: impl Read + Send + 'static, tx: Sender<String>) {
    std::thread::spawn(move || {
        let mut r = BufReader::new(r);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match r.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf).trim_end_matches(['\r', '\n']).to_string();
                    if tx.send(line).is_err() {
                        return;
                    }
                }
            }
        }
    });
}

struct ChildLines {
    child: Arc<std::sync::Mutex<Child>>,
    /// Ends (all senders dropped) once the process closed both stdout and stderr.
    lines: Receiver<String>,
}

impl LineStream for ChildLines {
    fn next_line(&mut self) -> Option<String> {
        self.lines.recv().ok()
    }
    fn killer(&self) -> Killer {
        let c = self.child.clone();
        Arc::new(move || {
            if let Ok(mut c) = c.lock() {
                let _ = c.kill();
                let _ = c.wait();
            }
        })
    }
}

impl Drop for ChildLines {
    fn drop(&mut self) {
        (self.killer())();
    }
}

/// Quote one argument for Python's `shlex.split` (POSIX rules).
pub fn shlex_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

/// Turn pymobiledevice3's stderr into a SwiftWin error with a useful message.
///
/// The CLI prints the exception's message (its docstring), not its class name; the class
/// names are matched too, for tracebacks (`-v`) and uncaught exceptions.
pub fn classify_error(what: &str, out: &CmdOutput) -> SwiftWinError {
    let all = format!("{}\n{}", out.stderr, out.stdout);
    let lower = all.to_ascii_lowercase();
    let has = |pats: &[&str]| pats.iter().any(|p| lower.contains(p));
    let msg = error_message(&all);
    if has(&[
        "notpaired",
        "device is not paired",
        "no pair record",
        "pairingdialogresponsepending",
        "trust dialog",
        "passwordprotected",
        "password protected",
        "userdeniedpairing",
        "refused to trust",
        "invalidhostid",
        "does not recognize this computer",
    ]) {
        return SwiftWinError::NotTrusted(msg);
    }
    if has(&["developermodeisnotenabled", "developer mode is disabled", "developermodeerror"]) {
        return SwiftWinError::DeveloperModeOff;
    }
    if what.contains("launch") && crate::signing_expired(&all) {
        return SwiftWinError::Other(crate::SIGNING_EXPIRED.into());
    }
    if has(&["nodeviceconnected", "device is not connected", "devicenotfounderror", "device not found", "no device"]) {
        return SwiftWinError::NoDevice;
    }
    SwiftWinError::tool(format!("pymobiledevice3 {what}"), msg)
}

/// The message of the last `ERROR` log line, else the text of a usage-error box, else the last
/// non-empty line (e.g. a traceback's).
fn error_message(s: &str) -> String {
    let lines: Vec<&str> = s.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if let Some(m) = lines.iter().rev().find_map(|l| l.split_once(" ERROR ").map(|(_, m)| m.trim())) {
        if m.starts_with("Failed to start service") {
            // INVALID_SERVICE_MESSAGE: a multi-line list of possible reasons follows.
            return "the iPhone refused to start the developer service (is Developer Mode on and the developer \
                    disk image mounted?)"
                .into();
        }
        return m.to_string();
    }
    if let Some(m) = boxed_error(&lines) {
        return m;
    }
    lines.last().copied().unwrap_or("no output").to_string()
}

/// Typer/rich print usage errors in a box, even with `--no-color`:
/// `┌─ Error ──┐` / `│ No such option: --udid │` / `└──────┘` (`╭ ╰` with rounded boxes).
fn boxed_error(lines: &[&str]) -> Option<String> {
    let start = lines.iter().position(|l| (l.starts_with('┌') || l.starts_with('╭')) && l.contains(" Error "))?;
    let body: Vec<&str> = lines[start + 1..]
        .iter()
        .take_while(|l| l.starts_with('│'))
        .map(|l| l.trim_matches(|c: char| c == '│' || c.is_whitespace()))
        .filter(|l| !l.is_empty())
        .collect();
    (!body.is_empty()).then(|| body.join(" "))
}

/// pymobiledevice3's own pairing record (`~/.pymobiledevice3/<usbmux serial>.plist`).
///
/// `lockdown pair` and autopair save the record only there, not to usbmuxd, so a phone paired
/// through pymobiledevice3 (`swiftwin pair`, `swiftwin doctor --fix`) has no usbmuxd record.
pub fn has_local_pair_record(udids: &[String]) -> bool {
    let home = if cfg!(windows) { std::env::var_os("USERPROFILE") } else { std::env::var_os("HOME") };
    home.is_some_and(|h| has_pair_record_in(&PathBuf::from(h).join(".pymobiledevice3"), udids))
}

pub fn has_pair_record_in(dir: &Path, udids: &[String]) -> bool {
    udids.iter().any(|u| !u.is_empty() && dir.join(format!("{u}.plist")).is_file())
}

/// High-level pymobiledevice3 calls for one device.
#[derive(Clone)]
pub struct Pmd {
    pub runner: Arc<dyn Runner>,
    pub udid: String,
}

impl Pmd {
    pub fn new(runner: Arc<dyn Runner>, udid: impl Into<String>) -> Self {
        Pmd { runner, udid: udid.into() }
    }

    pub fn args(&self, parts: &[&str]) -> Vec<String> {
        let mut v = vec!["--no-color".to_string()];
        v.extend(parts.iter().map(|s| s.to_string()));
        v.push("--udid".into());
        v.push(self.udid.clone());
        v
    }

    pub fn call(&self, what: &str, args: Vec<String>) -> Result<String> {
        self.call_env(what, args, &[])
    }

    fn call_env(&self, what: &str, args: Vec<String>, env: &[(&str, &str)]) -> Result<String> {
        let out = if env.is_empty() { self.runner.run(&args)? } else { self.runner.run_env(&args, env)? };
        if !out.success {
            return Err(classify_error(what, &out));
        }
        Ok(out.stdout)
    }

    fn json(&self, what: &str, args: Vec<String>) -> Result<serde_json::Value> {
        Self::parse_json(what, &self.call(what, args)?)
    }

    fn parse_json(what: &str, s: &str) -> Result<serde_json::Value> {
        let start = s.find(['{', '[']).unwrap_or(0);
        serde_json::from_str(s[start..].trim())
            .map_err(|e| SwiftWinError::protocol(format!("pymobiledevice3 {what}: unexpected output ({e})")))
    }

    pub fn lockdown_info(&self) -> Result<serde_json::Value> {
        self.json("lockdown info", self.args(&["lockdown", "info"]))
    }

    pub fn developer_mode_status(&self) -> Result<bool> {
        let s = self.call("amfi developer-mode-status", self.args(&["amfi", "developer-mode-status"]))?;
        Ok(s.trim().eq_ignore_ascii_case("true"))
    }

    pub fn reveal_developer_mode(&self) -> Result<()> {
        self.call("amfi reveal-developer-mode", self.args(&["amfi", "reveal-developer-mode"])).map(|_| ())
    }

    // UNTESTED-ON-DEVICE
    pub fn pair(&self) -> Result<()> {
        self.call("lockdown pair", self.args(&["lockdown", "pair"])).map(|_| ())
    }

    /// Confirmed on an iPhone with iOS 27.0.1: the personalized image is downloaded and mounted.
    pub fn mount_developer_image(&self) -> Result<()> {
        self.call("mounter auto-mount", self.args(&["mounter", "auto-mount"])).map(|_| ())
    }

    pub fn list_apps(&self) -> Result<Vec<AppInfo>> {
        let v = self.json("apps list", self.args(&["apps", "list", "--type", "User"]))?;
        Ok(parse_apps(&v))
    }

    pub fn install(&self, package: &Path) -> Result<()> {
        let p = package.to_string_lossy();
        self.call("apps install", self.args(&["apps", "install", &p])).map(|_| ())
    }

    // UNTESTED-ON-DEVICE
    pub fn uninstall(&self, bundle_id: &str) -> Result<()> {
        self.call("apps uninstall", self.args(&["apps", "uninstall", bundle_id])).map(|_| ())
    }

    pub fn launch(&self, bundle_id: &str, args: &[String], env: &[(String, String)], kill: bool) -> Result<u32> {
        let mut line = shlex_quote(bundle_id);
        for a in args {
            line.push(' ');
            line.push_str(&shlex_quote(a));
        }
        let mut parts = vec!["developer", "dvt", "launch"];
        parts.push(if kill { "--kill-existing" } else { "--no-kill-existing" });
        let envs: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        for e in &envs {
            parts.push("--env");
            parts.push(e);
        }
        parts.push(&line);
        let out = self.call("developer dvt launch", self.args(&parts))?;
        parse_launch_pid(&out)
            .ok_or_else(|| SwiftWinError::protocol(format!("could not read pid from launch output: {}", out.trim())))
    }

    /// Lists processes through DVT: proves the developer services (and on iOS 17+ the tunnel) work.
    /// On iOS 17+ DVT only exists once the developer disk image is mounted.
    // UNTESTED-ON-DEVICE
    pub fn proclist(&self) -> Result<()> {
        self.call("developer dvt proclist", self.args(&["developer", "dvt", "proclist"])).map(|_| ())
    }

    /// Brings up the iOS 17+ RemoteXPC tunnel and reads the RSD handshake, without needing the
    /// developer disk image (unlike `proclist`). `remote rsd-info` always requires an RSD, so with no
    /// tunnel flag it uses the no-root userspace tunnel (iOS 17.4+), or tunneld on iOS 17.0–17.3.
    /// It has no `--udid` option ("No such option"): the tunnel code reads the device from
    /// `PYMOBILEDEVICE3_UDID` instead. Confirmed on an iPhone with iOS 27.0.1 (about 3 s).
    pub fn tunnel_check(&self) -> Result<()> {
        let args: Vec<String> = ["--no-color", "remote", "rsd-info"].map(String::from).to_vec();
        let out = self.call_env("remote rsd-info", args, &[("PYMOBILEDEVICE3_UDID", &self.udid)])?;
        Self::parse_json("remote rsd-info", &out).map(|_| ())
    }

    // UNTESTED-ON-DEVICE
    pub fn pid_for_bundle(&self, bundle_id: &str) -> Result<Option<u32>> {
        let out = self.call(
            "developer dvt process-id-for-bundle-id",
            self.args(&["developer", "dvt", "process-id-for-bundle-id", bundle_id]),
        )?;
        Ok(out.trim().parse::<i64>().ok().filter(|p| *p > 0).map(|p| p as u32))
    }

    // UNTESTED-ON-DEVICE
    pub fn kill(&self, pid: u32) -> Result<()> {
        let p = pid.to_string();
        self.call("developer dvt kill", self.args(&["developer", "dvt", "kill", &p])).map(|_| ())
    }

    pub fn screenshot(&self, out: &Path) -> Result<()> {
        let p = out.to_string_lossy();
        self.call("developer dvt screenshot", self.args(&["developer", "dvt", "screenshot", &p])).map(|_| ())
    }

    // UNTESTED-ON-DEVICE
    pub fn crash_pull(&self, out: &Path) -> Result<()> {
        let p = out.to_string_lossy();
        self.call("crash pull", self.args(&["crash", "pull", &p])).map(|_| ())
    }

    pub fn syslog_stream(&self) -> Result<Box<dyn LineStream>> {
        self.runner.stream(&self.args(&["syslog", "live", "--format", "json"]))
    }

    /// Confirmed on iOS 27.0.1: the first start after trusting the developer can fail once with
    /// `initializationForUITestingDidFailWithError` (about 60 s); the next start works in ~30 s.
    pub fn xcuitest(&self, runner_bundle_id: &str, env: &[(String, String)]) -> Result<Box<dyn LineStream>> {
        let mut parts = vec!["developer", "dvt", "xcuitest"];
        let envs: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        for e in &envs {
            parts.push("--env");
            parts.push(e);
        }
        parts.push(runner_bundle_id);
        self.runner.stream(&self.args(&parts))
    }
}

pub fn parse_launch_pid(out: &str) -> Option<u32> {
    out.lines().find_map(|l| l.trim().strip_prefix("Process launched with pid ")?.trim().parse().ok())
}

pub fn parse_apps(v: &serde_json::Value) -> Vec<AppInfo> {
    let Some(map) = v.as_object() else { return vec![] };
    let mut apps: Vec<AppInfo> = map
        .iter()
        .map(|(id, info)| {
            let s = |k: &str| info.get(k).and_then(|x| x.as_str()).map(str::to_string);
            AppInfo {
                bundle_id: s("CFBundleIdentifier").unwrap_or_else(|| id.clone()),
                name: s("CFBundleDisplayName").or_else(|| s("CFBundleName")).unwrap_or_else(|| id.clone()),
                version: s("CFBundleShortVersionString"),
            }
        })
        .collect();
    apps.sort_by(|a, b| a.bundle_id.cmp(&b.bundle_id));
    apps
}

/// Parse one NDJSON line from `syslog live --format json`.
pub fn parse_syslog_line(line: &str) -> Option<LogEntry> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let path = v.get("filename").and_then(|x| x.as_str()).unwrap_or("");
    let process = path.rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).unwrap_or("?").to_string();
    Some(LogEntry {
        timestamp: v.get("timestamp").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        process,
        pid: v.get("pid").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
        level: LogLevel::parse(v.get("level").and_then(|x| x.as_str()).unwrap_or("info")),
        message: v.get("message").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        // pymobiledevice3 11.49: "label": {"subsystem", "category"} or null.
        subsystem: v.pointer("/label/subsystem").and_then(|x| x.as_str()).map(str::to_string),
    })
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// Records every call and answers from a script keyed by the subcommand.
    #[derive(Default)]
    pub struct FakeRunner {
        pub calls: Mutex<Vec<Vec<String>>>,
        /// Environment of each call made with `run_env`.
        pub envs: Mutex<Vec<Vec<(String, String)>>>,
        pub replies: Mutex<Vec<(String, CmdOutput)>>,
        pub stream_lines: Mutex<Vec<String>>,
        /// How many times a stream's killer was called.
        pub killed: Arc<std::sync::atomic::AtomicUsize>,
        /// Streams stay open after their lines (like a live log) until killed; else they end.
        pub stream_hold: std::sync::atomic::AtomicBool,
    }

    impl FakeRunner {
        pub fn reply(&self, contains: &str, success: bool, stdout: &str, stderr: &str) {
            self.replies.lock().unwrap().push((
                contains.to_string(),
                CmdOutput { success, stdout: stdout.into(), stderr: stderr.into() },
            ));
        }
        pub fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
        pub fn envs(&self) -> Vec<Vec<(String, String)>> {
            self.envs.lock().unwrap().clone()
        }
    }

    impl Runner for FakeRunner {
        fn run_env(&self, args: &[String], env: &[(&str, &str)]) -> Result<CmdOutput> {
            self.envs.lock().unwrap().push(env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect());
            self.run(args)
        }

        fn run(&self, args: &[String]) -> Result<CmdOutput> {
            self.calls.lock().unwrap().push(args.to_vec());
            let joined = args.join(" ");
            if args.iter().any(|a| a == "screenshot") {
                // Emulate pymobiledevice3 writing the file.
                let out = &args[args.iter().position(|a| a == "screenshot").unwrap() + 1];
                let mut c = crate::mock::canvas::Canvas::new(30, 60, crate::mock::canvas::Rgb(1, 2, 3));
                c.fill_rect(0.0, 0.0, 3.0, 3.0, crate::mock::canvas::Rgb(9, 9, 9));
                std::fs::write(out, c.to_png()).unwrap();
            }
            if args.iter().any(|a| a == "pull") {
                let out = &args[args.iter().position(|a| a == "pull").unwrap() + 1];
                std::fs::create_dir_all(out).unwrap();
                std::fs::write(Path::new(out).join("Hello-2026.ips"), "{}").unwrap();
            }
            for (k, v) in self.replies.lock().unwrap().iter() {
                if joined.contains(k.as_str()) {
                    return Ok(v.clone());
                }
            }
            Ok(CmdOutput { success: true, stdout: String::new(), stderr: String::new() })
        }

        fn stream(&self, args: &[String]) -> Result<Box<dyn LineStream>> {
            self.calls.lock().unwrap().push(args.to_vec());
            let hold = self.stream_hold.load(std::sync::atomic::Ordering::SeqCst);
            Ok(Box::new(FakeLines(self.stream_lines.lock().unwrap().clone().into(), self.killed.clone(), hold.then(Default::default))))
        }
    }

    /// Lines, the runner's kill counter, and (for a held stream) its "killed" flag.
    pub struct FakeLines(pub VecDeque<String>, pub Arc<std::sync::atomic::AtomicUsize>, pub Option<Arc<std::sync::atomic::AtomicBool>>);
    impl LineStream for FakeLines {
        fn next_line(&mut self) -> Option<String> {
            if let Some(l) = self.0.pop_front() {
                return Some(l);
            }
            while let Some(dead) = &self.2 {
                if dead.load(std::sync::atomic::Ordering::SeqCst) {
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            None
        }
        fn killer(&self) -> Killer {
            let (killed, dead) = (self.1.clone(), self.2.clone());
            Arc::new(move || {
                killed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if let Some(d) = &dead {
                    d.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeRunner;
    use super::*;

    #[test]
    fn quoting_matches_shlex() {
        assert_eq!(shlex_quote("com.example.app"), "com.example.app");
        assert_eq!(shlex_quote("hello world"), "'hello world'");
        assert_eq!(shlex_quote("it's"), "'it'\"'\"'s'");
        assert_eq!(shlex_quote(""), "''");
    }

    #[test]
    fn python_versions() {
        assert_eq!(parse_python_version("Python 3.11.9\n"), Some((3, 11)));
        assert_eq!(parse_python_version("Python 3.13.0rc1"), Some((3, 13)));
        assert_eq!(parse_python_version("nope"), None);
    }

    #[test]
    fn launch_builds_args_and_parses_pid() {
        let r = Arc::new(FakeRunner::default());
        r.reply("dvt launch", true, "Process launched with pid 4242\n", "");
        let p = Pmd::new(r.clone(), "UDID1");
        let pid = p.launch("com.x.app", &["-flag".into(), "two words".into()], &[("A".into(), "1".into())], true);
        assert_eq!(pid.unwrap(), 4242);
        let call = &r.calls()[0];
        assert_eq!(
            call,
            &[
                "--no-color",
                "developer",
                "dvt",
                "launch",
                "--kill-existing",
                "--env",
                "A=1",
                "com.x.app -flag 'two words'",
                "--udid",
                "UDID1"
            ]
        );
    }

    #[test]
    fn expired_signing_on_launch_says_run_again() {
        let r = Arc::new(FakeRunner::default());
        r.reply(
            "dvt launch",
            false,
            "",
            "2026-11-10 10:00:00 PC pymobiledevice3.__main__[1] ERROR Unable to launch com.x.app because it has an invalid              code signature, inadequate entitlements or its profile has not been explicitly trusted by the user.
",
        );
        let e = Pmd::new(r, "U").launch("com.x.app", &[], &[], true).unwrap_err().to_string();
        assert!(e.contains("7-day signing expired") && e.contains("install_run"), "{e}");
    }

    #[test]
    fn errors_are_classified() {
        let r = Arc::new(FakeRunner::default());
        r.reply("apps install", false, "", "Traceback...\npymobiledevice3.exceptions.NotPairedError: NotPaired");
        r.reply("dvt screenshot", false, "", "DeveloperModeIsNotEnabled");
        r.reply("apps uninstall", false, "", "something odd\n\n");
        let p = Pmd::new(r, "U");
        assert!(matches!(p.install(Path::new("a.ipa")), Err(SwiftWinError::NotTrusted(_))));
        assert!(matches!(p.screenshot(Path::new("x.png")), Err(SwiftWinError::DeveloperModeOff)));
        let e = p.uninstall("x").unwrap_err().to_string();
        assert!(e.contains("something odd"), "{e}");
    }

    /// Exact stderr of pymobiledevice3 11.49.5 on Windows (captured with no phone attached),
    /// plus the docstring messages of the exceptions SwiftWin classifies.
    #[test]
    fn real_11_49_5_messages_are_classified() {
        let log = |m: &str| {
            format!(
                "2026-11-03 19:48:51 DESKTOP-X pymobiledevice3.__main__[37124] ERROR {m}\n\
                 2026-11-03 19:48:51 DESKTOP-X pymobiledevice3.__main__[37124] INFO Run again with -v for the full traceback (include it when reporting an issue)\n"
            )
        };
        let err = |m: &str| classify_error("x", &CmdOutput { success: false, stdout: String::new(), stderr: log(m) });

        let e = err("Device not found: usbmux has no device matching udid 00008110-000A1B2C3D4E5F60");
        assert!(matches!(e, SwiftWinError::NoDevice), "{e}");
        assert!(matches!(err("Device is not connected."), SwiftWinError::NoDevice));
        for m in [
            "Device is not paired.",
            "usbmuxd has no pair record for device X. Pair it with `pymobiledevice3 lockdown pair`.",
            "Waiting for the user to answer the Trust dialog on the device.",
            "Device is password protected. Please unlock and retry.",
            "The user refused to trust this computer.",
            "The device does not recognize this computer's pairing record. Pair it again.",
        ] {
            match err(m) {
                SwiftWinError::NotTrusted(d) => assert_eq!(d, m),
                other => panic!("{m}: {other}"),
            }
        }
        assert!(matches!(
            err("Developer Mode is disabled. You can try to enable it using: python3 -m pymobiledevice3 amfi enable-developer-mode"),
            SwiftWinError::DeveloperModeOff
        ));

        // The message is the ERROR line, never the trailing "Run again with -v" hint.
        let e = err("Unable to connect to tunneld. Start one with `sudo python3 -m pymobiledevice3 remote tunneld`.").to_string();
        assert!(e.contains("Unable to connect to tunneld"), "{e}");
        assert!(!e.contains("Run again"), "{e}");
        let e = err("Failed to start service. Possible reasons are:").to_string();
        assert!(e.contains("developer disk image"), "{e}");
    }

    #[test]
    fn local_pair_records_are_found_under_any_given_spelling() {
        let dir = std::env::temp_dir().join(format!("swiftwin-pmd-pair-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("00008110-000A1B2C3D4E5F60.plist"), "<plist/>").unwrap();
        assert!(has_pair_record_in(&dir, &["000081100000A1B2C3D4E5F60".into(), "00008110-000A1B2C3D4E5F60".into()]));
        assert!(!has_pair_record_in(&dir, &["00008110-FFFFFFFFFFFFFFFF".into(), String::new()]));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn tunnel_check_uses_rsd_info_with_the_udid_in_the_environment() {
        let r = Arc::new(FakeRunner::default());
        r.reply("rsd-info", true, "{\n    \"Properties\": {}\n}\n", "");
        Pmd::new(r.clone(), "U").tunnel_check().unwrap();
        assert_eq!(r.calls()[0], ["--no-color", "remote", "rsd-info"]);
        assert_eq!(r.envs()[0], [("PYMOBILEDEVICE3_UDID".to_string(), "U".to_string())]);
    }

    /// Exact stderr of pymobiledevice3 11.49.5 for a usage error (from `remote rsd-info --udid X`).
    #[test]
    fn usage_error_boxes_give_their_message() {
        let stderr = "Usage: pymobiledevice3 remote rsd-info [OPTIONS]\n\
                      Try 'pymobiledevice3 remote rsd-info -h' for help.\n\
                      ┌─ Error ─────────────────────────────────────┐\n\
                      │ No such option: --udid                      │\n\
                      └─────────────────────────────────────────────┘\n";
        let e = classify_error("x", &CmdOutput { success: false, stdout: String::new(), stderr: stderr.into() });
        assert!(e.to_string().contains("No such option: --udid"), "{e}");
        let wrapped = "╭─ Error ──────╮\n│ first half   │\n│ second half  │\n╰──────────────╯\n";
        assert_eq!(error_message(wrapped), "first half second half");
    }

    #[test]
    fn streams_merge_stderr_and_end_with_the_process() {
        let prefix: Vec<String> = if cfg!(windows) {
            vec!["cmd".into(), "/c".into()]
        } else {
            vec!["sh".into(), "-c".into()]
        };
        let script = if cfg!(windows) { "echo out& echo err 1>&2" } else { "echo out; echo err 1>&2" };
        let mut s = ProcessRunner { prefix }.stream(&[script.to_string()]).unwrap();
        let mut lines = Vec::new();
        while let Some(l) = s.next_line() {
            lines.push(l.trim().to_string());
        }
        lines.sort();
        assert_eq!(lines, ["err", "out"]);
    }

    #[test]
    fn parses_apps_and_syslog() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"com.b": {"CFBundleIdentifier":"com.b","CFBundleName":"B"},
                "com.a": {"CFBundleIdentifier":"com.a","CFBundleDisplayName":"A","CFBundleShortVersionString":"2.0"}}"#,
        )
        .unwrap();
        let apps = parse_apps(&v);
        assert_eq!(apps[0].name, "A");
        assert_eq!(apps[0].version.as_deref(), Some("2.0"));
        assert_eq!(apps[1].bundle_id, "com.b");

        let e = parse_syslog_line(
            r#"{"pid": 312, "timestamp": "2026-11-03T12:00:00.5", "level": "ERROR", "filename": "/private/var/containers/Bundle/Application/X/Hello.app/Hello", "message": "boom", "label": null}"#,
        )
        .unwrap();
        assert_eq!(e.process, "Hello");
        assert_eq!(e.pid, 312);
        assert_eq!(e.level, LogLevel::Error);
        assert!(parse_syslog_line("not json").is_none());
    }
}
