use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use swiftwin_control::{connect, ConnectOptions, Phone};
use swiftwin_core::{Button, LogLevel, LogQuery, Point};
use swiftwin_device::LaunchOptions;

#[derive(Parser, Debug)]
#[command(name = "swiftwin", version, about = "Build, run and test SwiftUI iOS apps on a real iPhone from Windows")]
pub struct Cli {
    /// Use the simulated iPhone instead of a USB device (same as SWIFTWIN_MOCK=1).
    #[arg(long, global = true)]
    pub mock: bool,
    /// UDID of the phone to use when several are connected.
    #[arg(long, global = true, env = "SWIFTWIN_UDID")]
    pub udid: Option<String>,
    /// Print machine-readable JSON.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List connected iPhones.
    Devices,
    /// Show details about the phone.
    Info,
    /// List installed apps.
    Apps,
    /// Install a signed .ipa or .app.
    Install {
        /// Path to the .ipa or .app.
        package: PathBuf,
    },
    /// Uninstall an app.
    Uninstall {
        /// Bundle id, e.g. com.example.hello.
        bundle_id: String,
    },
    /// Launch an app.
    Launch {
        /// Bundle id, e.g. com.example.hello.
        bundle_id: String,
        /// Argument passed to the app (repeatable).
        #[arg(long = "arg")]
        args: Vec<String>,
    },
    /// Stop a running app.
    Terminate {
        /// Bundle id, e.g. com.example.hello.
        bundle_id: String,
    },
    /// Save a PNG screenshot.
    Screenshot {
        /// PNG file to write.
        #[arg(short, long, default_value = "screenshot.png")]
        out: PathBuf,
    },
    /// Print device logs.
    Logs {
        /// Only lines from this process (e.g. your app's name).
        #[arg(long)]
        process: Option<String>,
        /// Only lines containing this text.
        #[arg(long)]
        contains: Option<String>,
        /// Minimum level: debug, info, notice, error, fault.
        #[arg(long)]
        level: Option<String>,
        /// How many recent lines to print.
        #[arg(long, default_value_t = 200)]
        limit: usize,
        /// Keep printing new lines.
        #[arg(short, long)]
        follow: bool,
    },
    /// Copy crash reports to a folder.
    Crashes {
        /// Folder to copy the reports into.
        #[arg(short, long, default_value = "crashes")]
        out: PathBuf,
    },
    /// Tap at x y (points), or on the element with --label.
    Tap {
        /// X in points.
        x: Option<f64>,
        /// Y in points.
        y: Option<f64>,
        /// Tap the element with this label (see `swiftwin ui-tree`) instead of x y.
        #[arg(long)]
        label: Option<String>,
    },
    /// Swipe from (x1,y1) to (x2,y2) in points.
    Swipe {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        /// Seconds the swipe takes.
        #[arg(long, default_value_t = 0.3)]
        duration: f64,
    },
    /// Type text into the focused field.
    Type {
        /// The text to type.
        text: String,
    },
    /// Press a hardware button: home, volume-up, volume-down.
    Button {
        /// home, volume-up or volume-down.
        name: String,
    },
    /// Go to the home screen.
    Home,
    /// Print the on-screen UI element tree.
    UiTree,
    /// Run a scripted tour of the mock phone (no hardware needed).
    Demo,
    /// Ask the phone to trust this computer (shows the "Trust This Computer?" dialog).
    Pair,
    /// Set up this PC: Apple's USB service (iTunes), Python, Swift 6.2.4, LLVM and SwiftWin's helper
    /// tools, with winget; skips what is already there. The iOS SDK needs your Apple ID: see `swiftwin sdk`.
    Setup {
        /// Only show what is installed and what is missing.
        #[arg(long)]
        check: bool,
    },
    /// Create a new SwiftUI project.
    New {
        /// App name (also the folder name).
        name: String,
        /// Bundle id (default: com.example.<name>).
        #[arg(long)]
        bundle_id: Option<String>,
        /// Folder to create the project folder <name> in. If it is already named <name>
        /// (e.g. `swiftwin new Tempo` inside an empty `Tempo` folder), the project goes into it directly.
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
    /// Compile the project to an (unsigned) .app.
    Build {
        /// Optimized release build instead of debug.
        #[arg(long)]
        release: bool,
        /// Project folder (default: current folder).
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Also write an unsigned .ipa next to the .app, for a sideloader (free Apple ID).
        #[arg(long)]
        ipa: bool,
    },
    /// Build, sign, install and launch the project on the phone.
    Run {
        /// Optimized release build instead of debug.
        #[arg(long)]
        release: bool,
        /// Project folder (default: current folder).
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
    /// Sign an .app for the connected phone.
    Sign {
        /// Path to the .app.
        app: PathBuf,
    },
    /// Configure signing: a free Apple ID or a paid account's App Store Connect API key.
    Signing {
        #[command(subcommand)]
        cmd: SigningCmd,
    },
    /// Mirror the screen for a while, saving frames (USB screenshots, the USB video stream or AirPlay).
    Mirror {
        /// How long to mirror.
        #[arg(long, default_value_t = 5.0)]
        seconds: f64,
        /// auto, usb (screenshots only) or video.
        #[arg(long, default_value = "auto")]
        mode: String,
        /// Use AirPlay (UxPlay) as the video source instead of WebDriverAgent's USB video stream.
        #[arg(long)]
        airplay: bool,
        /// Folder to save frames into.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Frames per second to save.
        #[arg(long, default_value_t = 4.0)]
        fps: f64,
    },
    /// Open the phone-only window of the desktop app (live iPhone mirror, for working in Claude Code).
    Phone,
    /// Run the MCP server on stdio (Claude Code: `claude mcp add swiftwin -- swiftwin mcp`; Codex: `codex mcp add swiftwin -- swiftwin mcp`).
    Mcp {
        /// Project folder the build/install_run/release tools use by default.
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
    /// Serve the desktop UI in a browser (development/testing): http://127.0.0.1:<port>
    Ui {
        /// Local port.
        #[arg(long, default_value_t = 8787)]
        port: u16,
        /// Folder with the built web UI (app/dist).
        #[arg(long, default_value = "app/dist")]
        r#static: PathBuf,
        /// Delay every API call by this many ms (simulates a slow phone link for UI testing).
        #[arg(long, default_value_t = 0)]
        latency_ms: u64,
    },
    /// Sign in to your SwiftWin account: email + password, or --google / --github in the browser.
    Login {
        /// Email of the account (the password is asked for).
        #[arg(long, conflicts_with_all = ["google", "github"])]
        email: Option<String>,
        /// Sign in with Google in the browser.
        #[arg(long, conflicts_with = "github")]
        google: bool,
        /// Sign in with GitHub in the browser.
        #[arg(long)]
        github: bool,
    },
    /// Sign out here (also disconnects GitHub uploads).
    Logout,
    /// Show who is signed in (no network call).
    Whoami,
    /// Upload the project to GitHub. The first upload creates a private repository.
    Upload {
        /// Project folder (default: current folder).
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Repository name for the first upload (default: the folder name).
        #[arg(long)]
        name: Option<String>,
        /// Make a new repository public instead of private.
        #[arg(long)]
        public: bool,
        /// Commit message.
        #[arg(short, long)]
        message: Option<String>,
    },
    /// TestFlight build on a cloud Mac (GitHub Actions, Xcode 26).
    Release {
        #[command(subcommand)]
        cmd: Option<ReleaseCmd>,
        /// Project folder (default: current folder).
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Return as soon as the cloud build has started.
        #[arg(long)]
        no_wait: bool,
        /// Only write the generated workflow/XcodeGen files to build/release-preview (no upload).
        #[arg(long)]
        dry_run: bool,
    },
    /// Step-by-step checks for the first real-iPhone session (PASS/FAIL with fixes).
    Doctor {
        /// Apply automatic fixes (install helpers, ask for trust, reveal Developer Mode, prepare WDA).
        #[arg(long)]
        fix: bool,
    },
    /// Manage iOS SDKs extracted from Xcode.
    Sdk {
        #[command(subcommand)]
        cmd: SdkCmd,
    },
    /// Manage WebDriverAgent (the on-phone UI automation server).
    Wda {
        #[command(subcommand)]
        cmd: WdaCmd,
    },
}

#[derive(Subcommand, Debug)]
pub enum SigningCmd {
    /// Store your App Store Connect API key (the .p8 goes to the Windows Credential Manager).
    Setup {
        /// Key ID of the API key (also in the file name AuthKey_<KEYID>.p8).
        #[arg(long)]
        key_id: String,
        /// Issuer ID shown above the keys in App Store Connect.
        #[arg(long)]
        issuer_id: String,
        /// Path to AuthKey_<KEYID>.p8 downloaded from App Store Connect.
        #[arg(long)]
        p8: PathBuf,
        /// Your Apple Developer Team ID (Membership details page).
        #[arg(long)]
        team_id: String,
    },
    /// Sign with a free Apple ID through plumesign (no paid developer account needed).
    AppleId {
        /// Your Apple ID e-mail.
        #[arg(long)]
        email: String,
        /// Sign in here with plumesign (the password is shown as you type).
        #[arg(long)]
        login: bool,
    },
    /// Show what is configured.
    Status,
}

#[derive(Subcommand, Debug)]
pub enum ReleaseCmd {
    /// Store a GitHub token and upload the App Store Connect key as encrypted repo secrets.
    Setup {
        /// GitHub repository "owner/name" (must have at least one commit).
        #[arg(long)]
        repo: String,
        /// Personal access token with `repo` and `workflow` scopes.
        #[arg(long, env = "SWIFTWIN_GITHUB_TOKEN")]
        token: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum SdkCmd {
    /// List imported SDKs.
    List,
    /// Extract the iOS SDK from an Xcode .xip you downloaded with your Apple ID.
    Import {
        /// Path to the Xcode .xip.
        xip: PathBuf,
    },
    /// Download Xcode.xip using your developer.apple.com session cookie, then import it.
    Download {
        /// Xcode version, e.g. 26.3.
        #[arg(long, default_value = swiftwin_sdk::download::DEFAULT_XCODE)]
        xcode: String,
        /// Value of the ADCDownloadAuth cookie from developer.apple.com (after signing in).
        #[arg(long, env = "SWIFTWIN_ADC_COOKIE")]
        cookie: String,
        /// Download URL (default: derived from --xcode).
        #[arg(long)]
        url: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum WdaCmd {
    /// Is WDA installed and answering?
    Status,
    /// Start WDA on the phone (normally automatic).
    Start,
    /// Stop WDA.
    Stop,
    /// Download, re-sign and install WebDriverAgent for your team.
    Prepare {
        /// Use a local WebDriverAgentRunner-Runner.zip instead of downloading.
        #[arg(long)]
        zip: Option<PathBuf>,
    },
    /// Install a signed WebDriverAgentRunner .ipa/.app and remember its bundle id.
    Install {
        /// Path to the signed runner .ipa or .app.
        package: PathBuf,
    },
    /// Write an unsigned WebDriverAgentRunner .ipa to install with a sideloader (free Apple ID).
    Ipa {
        /// Use a local WebDriverAgentRunner-Runner.zip instead of downloading.
        #[arg(long)]
        zip: Option<PathBuf>,
        /// .ipa file to write.
        #[arg(short, long, default_value = "WebDriverAgentRunner.ipa")]
        out: PathBuf,
    },
    /// Use the WebDriverAgent runner installed under this bundle id (e.g. one a sideloader signed).
    Use {
        /// The runner's bundle id.
        bundle_id: String,
    },
    /// Serve a fake WDA (backed by the mock phone) on a local port, for developing clients.
    Serve {
        /// Local port.
        #[arg(long, default_value_t = 8100)]
        port: u16,
    },
}

fn phone(cli: &Cli) -> Result<Phone> {
    Ok(connect(&ConnectOptions { mock: cli.mock, udid: cli.udid.clone() })?)
}

/// One line on stderr when part of SwiftWin's folder lives in a Microsoft Store app's private storage
/// (Windows redirects it for programs started from such an app; `swiftwin doctor` shows the fix).
pub fn warn_if_sandboxed(cli: &Cli) {
    if swiftwin_core::mock_requested(cli.mock) || matches!(cli.cmd, Cmd::Doctor { .. }) {
        return;
    }
    for p in swiftwin_core::paths::sandboxed_copies() {
        eprintln!("warning: some SwiftWin files are in an app's private storage ({}), so the SwiftWin app can't see them; run `swiftwin doctor` for the fix", p.display());
    }
}

fn print_json<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// An account is needed only for the cloud features (cloud TestFlight `release`, `upload` to GitHub);
/// everything on the PC and the phone works without one. The simulated phone (`--mock`) is exempt.
fn require_account_for_cloud(mock: bool) -> swiftwin_core::Result<()> {
    if swiftwin_core::mock_requested(mock) {
        return Ok(());
    }
    let a = swiftwin_account::Accounts::platform();
    if !a.configured() {
        return Err(swiftwin_core::SwiftWinError::Auth(
            "cloud builds and uploads need a SwiftWin account, and this copy has no account service (build it with your own Supabase project, docs/accounts-setup.md)".into(),
        ));
    }
    if a.cached_user().is_none() {
        return Err(swiftwin_core::SwiftWinError::Auth("cloud builds and uploads need a SwiftWin account: sign in in SwiftWin Studio, or run `swiftwin login`".into()));
    }
    Ok(())
}

pub fn run(cli: Cli) -> Result<()> {
    if matches!(cli.cmd, Cmd::Release { .. } | Cmd::Upload { .. }) {
        require_account_for_cloud(cli.mock)?;
    }
    match &cli.cmd {
        Cmd::Devices => {
            if swiftwin_core::mock_requested(cli.mock) {
                let info = phone(&cli)?.device.info()?;
                if cli.json {
                    print_json(&vec![info])?;
                } else {
                    println!("{}  {}  {} (iOS {})  {:?}", info.udid, info.name, info.model, info.ios_version, info.connection);
                }
            } else {
                let list = swiftwin_device::real::list_devices(&swiftwin_device::real::usbmux::Usbmux::platform())?;
                if cli.json {
                    print_json(&list)?;
                } else if list.is_empty() {
                    println!("No iPhone found. Plug it in with a USB cable and unlock it.");
                } else {
                    for d in list {
                        println!(
                            "{}  {:?}  {}",
                            d.udid,
                            d.connection,
                            if d.paired { "trusted" } else { "NOT trusted: unlock the phone and tap Trust" }
                        );
                    }
                }
            }
        }
        Cmd::Info => {
            let info = phone(&cli)?.device.info()?;
            if cli.json {
                print_json(&info)?;
            } else {
                println!("Name:            {}", info.name);
                println!("UDID:            {}", info.udid);
                println!("Model:           {}", info.model);
                println!("iOS:             {}", info.ios_version);
                println!("Connection:      {:?}", info.connection);
                println!("Trusted:         {}", if info.paired { "yes" } else { "no" });
                println!(
                    "Developer Mode:  {}",
                    match info.developer_mode {
                        Some(true) => "on",
                        Some(false) => "off",
                        None => "unknown",
                    }
                );
                if let Some(s) = info.screen {
                    println!("Screen:          {}x{} pt @{}x", s.width_pts, s.height_pts, s.scale);
                }
            }
        }
        Cmd::Apps => {
            let apps = phone(&cli)?.device.list_apps()?;
            if cli.json {
                print_json(&apps)?;
            } else {
                for a in apps {
                    println!("{:<40} {} {}", a.bundle_id, a.name, a.version.unwrap_or_default());
                }
            }
        }
        Cmd::Install { package } => {
            let info = phone(&cli)?.device.install(package)?;
            if cli.json {
                print_json(&info)?;
            } else {
                println!("Installed {} ({})", info.bundle_id, info.name);
            }
        }
        Cmd::Uninstall { bundle_id } => {
            phone(&cli)?.device.uninstall(bundle_id)?;
            println!("Uninstalled {bundle_id}");
        }
        Cmd::Launch { bundle_id, args } => {
            let opts = LaunchOptions { args: args.clone(), kill_existing: true, ..Default::default() };
            let pid = phone(&cli)?.device.launch(bundle_id, &opts)?;
            if cli.json {
                print_json(&serde_json::json!({ "bundle_id": bundle_id, "pid": pid }))?;
            } else {
                println!("Launched {bundle_id} (pid {pid})");
            }
        }
        Cmd::Terminate { bundle_id } => {
            phone(&cli)?.device.terminate(bundle_id)?;
            println!("Terminated {bundle_id}");
        }
        Cmd::Screenshot { out } => {
            let s = phone(&cli)?.screenshot()?;
            std::fs::write(out, &s.png).with_context(|| format!("writing {}", out.display()))?;
            println!("Saved {}x{} screenshot to {}", s.width, s.height, out.display());
        }
        Cmd::Logs { process, contains, level, limit, follow } => {
            let p = phone(&cli)?;
            let q = LogQuery {
                process: process.clone(),
                contains: contains.clone(),
                min_level: level.as_deref().map(LogLevel::parse),
                follow: *follow,
                limit: Some(*limit),
                subsystem: None,
                since: None,
            };
            let json = cli.json;
            p.device.syslog(&q, &mut |e| {
                if json {
                    println!("{}", serde_json::to_string(&e).unwrap_or_default());
                } else {
                    println!("{} {}[{}] <{:?}> {}", e.timestamp, e.process, e.pid, e.level, e.message);
                }
                true
            })?;
        }
        Cmd::Crashes { out } => {
            let files = phone(&cli)?.device.crash_reports(out)?;
            println!("Copied {} crash report(s) to {}", files.len(), out.display());
            for f in files {
                println!("  {}", f.display());
            }
        }
        Cmd::Tap { x, y, label } => {
            let p = phone(&cli)?;
            let at = match (x, y, label) {
                (_, _, Some(l)) => {
                    let tree = p.control.ui_tree()?;
                    let n = tree.find(l).with_context(|| format!("no element labelled {l:?} on screen"))?;
                    n.rect.center()
                }
                (Some(x), Some(y), None) => Point { x: *x, y: *y },
                _ => bail!("give x and y, or --label"),
            };
            p.control.tap(at)?;
            println!("Tapped ({:.0}, {:.0})", at.x, at.y);
        }
        Cmd::Swipe { x1, y1, x2, y2, duration } => {
            phone(&cli)?.control.swipe(Point { x: *x1, y: *y1 }, Point { x: *x2, y: *y2 }, *duration)?;
            println!("Swiped");
        }
        Cmd::Type { text } => {
            phone(&cli)?.control.type_text(text)?;
            println!("Typed {} character(s)", text.chars().count());
        }
        Cmd::Button { name } => {
            let b: Button = name.parse()?;
            phone(&cli)?.control.press_button(b)?;
            println!("Pressed {name}");
        }
        Cmd::Home => {
            phone(&cli)?.control.press_button(Button::Home)?;
            println!("Pressed home");
        }
        Cmd::UiTree => {
            let t = phone(&cli)?.control.ui_tree()?;
            if cli.json {
                print_json(&t)?;
            } else {
                print!("{}", t.to_text());
            }
        }
        Cmd::Demo => demo(&cli)?,
        Cmd::Pair => {
            if swiftwin_core::mock_requested(cli.mock) {
                println!("The mock phone is always trusted.");
            } else {
                let d = swiftwin_device::RealDevice::open(cli.udid.as_deref(), Default::default())?;
                println!("Unlock your iPhone and tap \"Trust\", then enter your passcode...");
                d.pmd()?.pair()?;
                println!("Paired with {}", d.udid());
            }
        }
        Cmd::Wda { cmd } => wda(&cli, cmd)?,
        Cmd::New { name, bundle_id, dir } => {
            // A free Apple ID can't use com.example.* (bundle ids are global at Apple).
            let cfg = swiftwin_core::config::Config::load();
            let default_id = cfg.id_prefix.as_deref().filter(|_| cfg.uses_apple_id()).map(|p| {
                let leaf: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
                format!("{p}.{}", leaf.to_ascii_lowercase())
            });
            let p = swiftwin_builder::scaffold::new_project(dir, name, bundle_id.as_deref().or(default_id.as_deref()))?;
            println!("Created {}", p.display());
            println!("Next: cd {} && swiftwin build", p.display());
        }
        Cmd::Build { release, project, ipa } => {
            let out = build_project(&cli, project, *release)?;
            let ipa = if *ipa { Some(swiftwin_builder::package_unsigned_ipa(&out.app, &out.app.with_extension("ipa"))?) } else { None };
            if cli.json {
                match &ipa {
                    Some(p) => print_json(&serde_json::json!({ "build": out, "ipa": p }))?,
                    None => print_json(&out)?,
                }
            } else {
                for w in &out.warnings {
                    println!("warning: {w}");
                }
                println!("Built {} ({})", out.app.display(), out.bundle_id);
                if let Some(p) = &ipa {
                    println!("Wrote {} (unsigned). Install it with your sideloader, then `swiftwin launch <bundle id>`.", p.display());
                    println!("If the sideloader changed the bundle id, `swiftwin apps` shows the installed one.");
                }
            }
        }
        Cmd::Sdk { cmd } => sdk(&cli, cmd)?,
        Cmd::Run { release, project } => {
            let ws = swiftwin_workflow::Workspace::new(phone(&cli)?);
            let r = ws.install_run(project, *release)?;
            if cli.json {
                print_json(&r)?;
            } else {
                for w in &r.warnings {
                    println!("warning: {w}");
                }
                println!("Installed and launched {} (pid {}) – team {} ({})", r.bundle_id, r.pid, r.team_id, r.timing_text());
            }
        }
        Cmd::Sign { app } => {
            let ws = swiftwin_workflow::Workspace::new(phone(&cli)?);
            let team = ws.sign(app)?;
            println!("Signed {} for team {team}", app.display());
        }
        Cmd::Signing { cmd } => signing(cmd)?,
        Cmd::Ui { port, r#static, latency_ms } => {
            let core = std::sync::Arc::new(swiftwin_app_core::AppCore::new(cli.mock));
            let latency = std::time::Duration::from_millis(*latency_ms);
            let token = core.token().to_string();
            let port = swiftwin_app_core::http::serve(core, *port, swiftwin_app_core::http::Ui::Dir(r#static.clone()), latency)?;
            swiftwin_app_core::bridge::write(port, &token)?;
            println!("SwiftWin UI on http://127.0.0.1:{port}  (Ctrl+C to stop)");
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
        Cmd::Doctor { fix } => doctor(&cli, *fix)?,
        Cmd::Login { email, google, github } => login(&cli, email.as_deref(), *google, *github)?,
        Cmd::Logout => {
            accounts(&cli).0.sign_out()?;
            println!("Signed out.");
        }
        Cmd::Whoami => match accounts(&cli).0.cached_user() {
            Some(u) if cli.json => print_json(&u)?,
            Some(u) => println!("{} ({}, {} plan)", u.email, u.provider, u.plan),
            None => bail!("not signed in (run `swiftwin login`)"),
        },
        Cmd::Upload { project, name, public, message } => upload(&cli, project, name.clone(), *public, message.clone())?,
        Cmd::Release { cmd, project, no_wait, dry_run } => release(&cli, cmd.as_ref(), project, *no_wait, *dry_run)?,
        Cmd::Phone => println!("{}", swiftwin_mcp::open_phone_window(swiftwin_core::mock_requested(cli.mock))?),
        Cmd::Mcp { project } => {
            let dir = std::fs::canonicalize(project).unwrap_or_else(|_| project.clone());
            // Windows canonical paths look like \\?\C:\...; show the familiar form.
            let dir = PathBuf::from(dir.to_string_lossy().trim_start_matches(r"\\?\").to_string());
            let mock = swiftwin_core::mock_requested(cli.mock);
            let opts = ConnectOptions { mock: cli.mock, udid: cli.udid.clone() };
            // Connect on first use: Claude Code starts this server before the phone may be plugged in.
            let connect = Box::new(move || connect(&opts).map(swiftwin_workflow::Workspace::new));
            let mut server = swiftwin_mcp::Server::lazy(connect, mock, dir.clone())
                .with_forward(std::sync::Arc::new(AppBridge { mock, project: dir, dead: Default::default() }));
            if !mock {
                if let Ok(gh) = github_from_config() {
                    server = server.with_release(std::sync::Arc::new(CloudRelease(gh)));
                }
            }
            let stdin = std::io::stdin();
            server.serve(stdin.lock(), std::io::stdout())?;
        }
        Cmd::Mirror { seconds, mode, airplay, out, fps } => mirror(&cli, *seconds, mode, *airplay, out.as_deref(), *fps)?,
        Cmd::Setup { check } => setup(&cli, *check)?,
    }
    Ok(())
}

fn demo(cli: &Cli) -> Result<()> {
    if !swiftwin_core::mock_requested(cli.mock) {
        bail!("`swiftwin demo` drives the simulated phone; run `swiftwin --mock demo`");
    }
    let p = phone(cli)?;
    let info = p.device.info()?;
    println!("1. Connected to {} ({}, iOS {})", info.name, info.model, info.ios_version);
    let tree = p.control.ui_tree()?;
    let icon = tree.find("Hello").context("Hello app icon missing")?;
    p.control.tap(icon.rect.center())?;
    println!("2. Tapped the Hello icon -> app launched");
    for _ in 0..3 {
        let b = p.control.ui_tree()?.find("Tap me").context("button missing")?.rect.center();
        p.control.tap(b)?;
    }
    println!("3. Tapped \"Tap me\" three times");
    let f = p.control.ui_tree()?.find("Your name").context("field missing")?.rect.center();
    p.control.tap(f)?;
    p.control.type_text("SwiftWin")?;
    println!("4. Typed \"SwiftWin\" into the text field");
    p.control.swipe(Point { x: 196.0, y: 700.0 }, Point { x: 196.0, y: 500.0 }, 0.3)?;
    println!("5. Swiped up to scroll the list");
    let out = PathBuf::from("swiftwin-demo.png");
    std::fs::write(&out, p.device.screenshot()?.png)?;
    println!("6. Saved screenshot to {}", out.display());
    println!("\nUI tree now:\n{}", p.control.ui_tree()?.to_text());
    println!("Recent logs:");
    for e in swiftwin_device::collect_logs(p.device.as_ref(), &LogQuery { limit: Some(8), ..Default::default() })? {
        println!("  {} [{}] {}", e.process, e.pid, e.message);
    }
    Ok(())
}

fn wda(cli: &Cli, cmd: &WdaCmd) -> Result<()> {
    if let WdaCmd::Serve { port } = cmd {
        let p = Phone::mock();
        let f = swiftwin_control::fakewda::FakeWda::start_on(*port, p.control.clone(), p.device.clone())?;
        println!("Fake WebDriverAgent (mock phone) listening on {}. Ctrl+C to stop.", f.url());
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    if let WdaCmd::Ipa { zip, out } = cmd {
        let bytes = match zip {
            Some(p) => std::fs::read(p)?,
            None => {
                println!("Downloading WebDriverAgent {} ...", swiftwin_signer::tools::WDA_VERSION);
                swiftwin_signer::tools::download_wda()?
            }
        };
        let work = std::env::temp_dir().join(format!("swiftwin-wda-ipa-{}", std::process::id()));
        let app = swiftwin_signer::wda::unpack_runner(&bytes, &work)?;
        let ipa = swiftwin_builder::package_unsigned_ipa(&app, out);
        let _ = std::fs::remove_dir_all(&work);
        let ipa = ipa?;
        println!("Wrote {} (unsigned). Install it with your sideloader under a unique bundle id,", ipa.display());
        println!("e.g. com.<yourname>.WebDriverAgentRunner.xctrunner. SwiftWin finds the installed runner by itself.");
        return Ok(());
    }
    if let WdaCmd::Use { bundle_id } = cmd {
        let mut cfg = swiftwin_core::config::Config::load();
        cfg.wda_bundle_id = Some(bundle_id.clone());
        cfg.save()?;
        println!("SwiftWin will use the WebDriverAgent runner {bundle_id}");
        return Ok(());
    }
    let p = phone(cli)?;
    if let WdaCmd::Prepare { zip } = cmd {
        let ws = swiftwin_workflow::Workspace::new(p);
        let info = ws.prepare_wda(zip.as_deref(), &mut |m| println!("{m}"))?;
        println!("WebDriverAgent ready: {}", info.bundle_id);
        return Ok(());
    }
    let Some(w) = p.wda.clone() else {
        println!("The mock phone does not need WebDriverAgent (it is controlled directly).");
        return Ok(());
    };
    match cmd {
        WdaCmd::Status => {
            // is_installed() first: it may switch to a runner found under another bundle id.
            let installed = w.is_installed()?;
            println!("Bundle id:  {}", w.bundle_id());
            println!(
                "Installed:  {}",
                if installed { "yes" } else { "no (run `swiftwin wda prepare`, or `swiftwin wda ipa` and install it with a sideloader)" }
            );
            match w.client() {
                Ok(c) => println!("Running:    yes ({})", c.base_url()),
                Err(e) => println!("Running:    no ({e})"),
            }
        }
        WdaCmd::Start => {
            let c = w.client()?;
            println!("WebDriverAgent ready at {}", c.base_url());
        }
        WdaCmd::Stop => {
            w.stop()?;
            println!("Stopped WebDriverAgent");
        }
        WdaCmd::Install { package } => {
            let info = p.install_wda(package)?;
            println!("Installed {} – SwiftWin will use it for UI control", info.bundle_id);
        }
        WdaCmd::Prepare { .. } | WdaCmd::Serve { .. } | WdaCmd::Ipa { .. } | WdaCmd::Use { .. } => unreachable!(),
    }
    Ok(())
}

/// Build with the real toolchain, or the fake compiler under --mock.
pub fn build_project(cli: &Cli, dir: &std::path::Path, release: bool) -> Result<swiftwin_builder::BuildOutput> {
    let project = swiftwin_builder::Project::load(dir)?;
    if swiftwin_core::mock_requested(cli.mock) {
        let opts = swiftwin_builder::BuildOptions { release, sdk_path: "mock.sdk".into(), sdk_version: "26.1".into(), module_cache: None };
        return Ok(swiftwin_builder::build(&project, &opts, &swiftwin_builder::FakeCompiler)?);
    }
    let cfg = swiftwin_core::config::Config::load();
    let sdk = swiftwin_sdk::active(&swiftwin_core::paths::sdk_dir(), cfg.sdk.as_deref())?;
    let swiftc = swiftwin_builder::compile::find_swiftc(cfg.swift_bin.as_deref()).context(
        "Swift is not installed: get the Windows toolchain from https://www.swift.org/install/windows/",
    )?;
    let cache = swiftwin_builder::shared_module_cache(&sdk.version);
    if swiftwin_builder::module_cache_is_cold(&cache) {
        eprintln!("First build with iOS SDK {}: compiling its modules too (a few minutes, once).", sdk.version);
    }
    let opts = swiftwin_builder::BuildOptions { release, sdk_path: sdk.sdk_path.clone(), sdk_version: sdk.version.clone(), module_cache: Some(cache) };
    Ok(swiftwin_builder::build(&project, &opts, &swiftwin_builder::Swiftc { path: swiftc })?)
}

fn sdk(cli: &Cli, cmd: &SdkCmd) -> Result<()> {
    let root = swiftwin_core::paths::sdk_dir();
    let mut progress = |p: &swiftwin_sdk::Progress| {
        if p.files > 0 {
            eprint!("\r{}: {} files, {} MB      ", p.phase, p.files, p.bytes / 1_000_000);
        } else {
            eprintln!("{}", p.phase);
        }
    };
    match cmd {
        SdkCmd::List => {
            let all = swiftwin_sdk::installed(&root);
            if cli.json {
                print_json(&all)?;
            } else if all.is_empty() {
                println!("No iOS SDK yet. Download Xcode with your Apple ID, then `swiftwin sdk import Xcode_X.xip`.");
            } else {
                for s in all {
                    println!("{}  {}", s.name, s.sdk_path.display());
                }
            }
        }
        SdkCmd::Import { xip } => {
            let info = swiftwin_sdk::import_xip(xip, &root, &mut progress)?;
            eprintln!();
            let mut c = swiftwin_core::config::Config::load();
            c.sdk = Some(info.name.clone());
            c.save()?;
            println!("Imported {} -> {}", info.name, info.sdk_path.display());
        }
        SdkCmd::Download { xcode, cookie, url } => {
            let url = url.clone().unwrap_or_else(|| swiftwin_sdk::download::xcode_url(xcode));
            let dest = root.join("downloads").join(format!("Xcode_{xcode}.xip"));
            swiftwin_sdk::download::download(&url, &dest, Some(cookie), &mut |done, total| {
                match total {
                    Some(t) => eprint!("\rDownloading Xcode {xcode}: {} / {} MB   ", done / 1_000_000, t / 1_000_000),
                    None => eprint!("\rDownloading Xcode {xcode}: {} MB   ", done / 1_000_000),
                }
            })?;
            eprintln!();
            let info = swiftwin_sdk::import_xip(&dest, &root, &mut progress)?;
            eprintln!();
            let mut c = swiftwin_core::config::Config::load();
            c.sdk = Some(info.name.clone());
            c.save()?;
            println!("Imported {} (you can delete {} to free space)", info.name, dest.display());
        }
    }
    Ok(())
}

fn signing(cmd: &SigningCmd) -> Result<()> {
    use swiftwin_signer::{secrets, setup};
    let store = secrets::platform_store();
    let mut cfg = swiftwin_core::config::Config::load();
    match cmd {
        SigningCmd::Setup { key_id, issuer_id, p8, team_id } => {
            let pem = std::fs::read_to_string(p8).with_context(|| format!("reading {}", p8.display()))?;
            let key = swiftwin_signer::AscKey { key_id: key_id.clone(), issuer_id: issuer_id.clone(), p8_pem: pem };
            println!("PASS: {}", setup::check_asc(&key, team_id, swiftwin_signer::asc::API_BASE).map_err(|e| anyhow::anyhow!("FAIL: {e}"))?);
            setup::use_asc(&mut cfg, store.as_ref(), &key, team_id)?;
            cfg.save()?;
            println!("Saved. The .p8 key is in the {}; you can delete {} now.",
                if cfg!(windows) { "Windows Credential Manager" } else { "SwiftWin secret store" }, p8.display());
        }
        SigningCmd::AppleId { email, login } => {
            let ps = setup::ensure_apple_id_tools(&|m| println!("{m}"))?;
            if !ps.signed_in_as(email)? {
                if !*login {
                    return Err(setup::not_signed_in(email).into());
                }
                println!("plumesign asks for your Apple ID password and 2FA code (SwiftWin never sees them). The password is shown as you type.");
                ps.login(email)?;
                if !ps.signed_in_as(email)? {
                    bail!("plumesign did not save a session for {email}");
                }
            }
            let prefix = setup::use_apple_id(&mut cfg, email)?;
            cfg.save()?;
            println!("SwiftWin now signs with your free Apple ID. New projects get bundle ids under {prefix}.");
            println!("{}", setup::FREE_LIMITS);
        }
        SigningCmd::Status => {
            let s = setup::status(&cfg, store.as_ref())?;
            let v = |k: &str, none: &str| s[k].as_str().unwrap_or(none).to_string();
            let yes = |k: &str, no: &str| if s[k] == true { "yes".to_string() } else { no.to_string() };
            if s["mode"] == "apple-id" {
                println!("Signing with:   free Apple ID {} (plumesign)", v("apple_id", "?"));
                println!("Bundle prefix:  {}", v("id_prefix", "(not set)"));
                println!("plumesign:      {}", v("plumesign", "missing (swiftwin doctor --fix)"));
                println!("{}", setup::FREE_LIMITS);
                if let Some(t) = s["paid_team_hint"].as_str() {
                    println!("Your Apple ID signed with paid team {t}: `swiftwin signing setup` lifts the 7-day limit.");
                }
                return Ok(());
            }
            println!("Signing with:   App Store Connect API key");
            println!("Team ID:        {}", v("team_id", "(not set)"));
            println!("ASC key id:     {}", v("asc_key_id", "(not set)"));
            println!("ASC issuer id:  {}", v("asc_issuer_id", "(not set)"));
            println!("ASC .p8 stored: {}", yes("p8_stored", "no"));
            println!("Signing key:    {}", yes("dev_key", "not created yet"));
            println!("rcodesign:      {}", v("rcodesign", "missing (swiftwin doctor --fix)"));
        }
    }
    Ok(())
}

fn mirror(cli: &Cli, seconds: f64, mode: &str, airplay: bool, out: Option<&std::path::Path>, fps: f64) -> Result<()> {
    use swiftwin_mirror::{airplay::AirPlayReceiver, Feed, Mirror, Mode, Policy, SourceKind, Video};
    let mode = match mode {
        "auto" => Mode::Auto,
        "usb" => Mode::UsbOnly,
        "video" | "airplay" => Mode::VideoOnly,
        m => bail!("unknown mode {m:?} (auto, usb, video)"),
    };
    let p = phone(cli)?;
    let mut receiver = if airplay { Some(AirPlayReceiver::start("SwiftWin")?) } else { None };
    if receiver.is_some() {
        println!("AirPlay receiver \"SwiftWin\" is running: on the iPhone open Control Center > Screen Mirroring > SwiftWin");
    }
    let video = match receiver.as_mut().and_then(|r| r.frames.take()) {
        Some(rx) => Some(Video { kind: SourceKind::AirPlay, feed: Feed::Frames(rx) }),
        None if !p.is_mock => {
            let phone3 = p.clone();
            Some(Video { kind: SourceKind::Stream, feed: Feed::Mjpeg(std::sync::Arc::new(move || phone3.video_stream(30))) })
        }
        None => None,
    };
    let phone2 = p.clone();
    let m = Mirror::start(std::sync::Arc::new(move || phone2.screenshot()), video, Policy::new(mode), fps);
    if let Some(d) = out {
        std::fs::create_dir_all(d)?;
    }
    let end = std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds);
    let (mut seq, mut n) = (0u64, 0u32);
    while std::time::Instant::now() < end {
        if let Some(f) = m.wait_next(seq, std::time::Duration::from_millis(500)) {
            seq = f.seq;
            n += 1;
            if let Some(d) = out {
                let ext = if f.format == swiftwin_mirror::FrameFormat::Png { "png" } else { "jpg" };
                std::fs::write(d.join(format!("frame-{:05}.{ext}", f.seq)), &f.data)?;
            }
        }
    }
    m.stop();
    let st = m.status();
    if cli.json {
        print_json(&st)?;
    } else {
        println!(
            "Received {n} new frame(s) in {seconds}s ({:.1}/s); active source: {:?}; switches: {}",
            n as f64 / seconds.max(0.1),
            st.active,
            st.switches.len()
        );
        if let Some(e) = st.last_error {
            println!("Last USB error: {e}");
        }
        if let Some(e) = st.video_error {
            println!("Last video error: {e}");
        }
    }
    Ok(())
}

fn github_from_config() -> Result<swiftwin_release::GitHub> {
    let cfg = swiftwin_core::config::Config::load();
    let repo = cfg.release_repo.context("cloud release is not set up: run `swiftwin release setup --repo owner/name --token ...`")?;
    let token = swiftwin_signer::secrets::platform_store()
        .get(swiftwin_signer::secrets::GITHUB_TOKEN)?
        .context("GitHub token missing: run `swiftwin release setup` again")?;
    Ok(swiftwin_release::GitHub::new(&repo, &token)?)
}

/// MCP `release` tool backend.
struct CloudRelease(swiftwin_release::GitHub);

impl swiftwin_mcp::ReleaseBackend for CloudRelease {
    fn release(&self, project: &std::path::Path, args: &serde_json::Value) -> swiftwin_core::Result<String> {
        require_account_for_cloud(false)?;
        let opts = swiftwin_release::ReleaseOptions { wait: args["wait"].as_bool().unwrap_or(true), ..Default::default() };
        let mut log = Vec::new();
        let r = swiftwin_release::release(&self.0, project, &opts, &mut |m| log.push(m.to_string()))?;
        Ok(format!("{}\nCommit {} · run {}", log.join("\n"), r.commit, r.run_url.unwrap_or_default()))
    }
}

/// The account manager: the real one, or one against in-process fakes with `--mock`.
fn accounts(cli: &Cli) -> (swiftwin_account::Accounts, Option<swiftwin_account::fake::FakeCloud>) {
    if swiftwin_core::mock_requested(cli.mock) {
        let cloud = swiftwin_account::fake::FakeCloud::start();
        let dir = std::env::temp_dir().join(format!("swiftwin-mock-account-{}", std::process::id()));
        let a = swiftwin_account::Accounts::new(
            cloud.settings(),
            std::sync::Arc::new(swiftwin_signer::secrets::MemoryStore::default()),
            swiftwin_account::fake::fake_browser(),
            dir,
        );
        (a, Some(cloud))
    } else {
        (swiftwin_account::Accounts::platform(), None)
    }
}

fn prompt(label: &str) -> Result<String> {
    use std::io::Write;
    eprint!("{label}");
    std::io::stderr().flush()?;
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    Ok(s.trim().to_string())
}

fn login(cli: &Cli, email: Option<&str>, google: bool, github: bool) -> Result<()> {
    let (a, _fake) = accounts(cli);
    if !a.configured() {
        bail!("accounts are not set up in this build of SwiftWin (see docs/first-run.md, \"Accounts\")");
    }
    let user = if google || github {
        let provider = if google { "google" } else { "github" };
        let url = a.start_browser_sign_in(provider)?;
        println!("Finish signing in in your browser. If it didn't open, go to:
{url}");
        a.wait_browser_sign_in(std::time::Duration::from_secs(600))?
    } else {
        let email = match email {
            Some(e) => e.to_string(),
            None => prompt("Email: ")?,
        };
        let password = match std::env::var("SWIFTWIN_PASSWORD") {
            Ok(p) => p,
            Err(_) => rpassword::prompt_password("Password: ")?,
        };
        a.sign_in(&email, &password)?
    };
    println!("Signed in as {} ({}).", user.email, user.provider);
    Ok(())
}

fn upload(cli: &Cli, project: &std::path::Path, name: Option<String>, public: bool, message: Option<String>) -> Result<()> {
    let (a, _fake) = accounts(cli);
    if a.configured() && a.cached_user().is_none() && _fake.is_none() {
        bail!("sign in first (`swiftwin login`)");
    }
    if a.github.token().is_err() {
        let c = a.github.connect()?;
        println!(
            "Connect GitHub: open {} and enter the code {}",
            c["verification_uri"].as_str().unwrap_or(""),
            c["user_code"].as_str().unwrap_or("")
        );
        println!("Connected to GitHub as {}.", a.github.wait(std::time::Duration::from_secs(900))?);
    }
    let dir = std::fs::canonicalize(project).with_context(|| format!("project folder {}", project.display()))?;
    let dir = PathBuf::from(dir.to_string_lossy().trim_start_matches(r"\?\").to_string());
    let links = swiftwin_account::upload::Links::new(swiftwin_core::paths::data_dir().join("uploads.json"));
    let opts = swiftwin_account::upload::Options { name, public, message };
    let progress = |step: &str, done: usize, total: usize| {
        if total > 0 {
            eprintln!("[{done}/{total}] {step}");
        } else {
            eprintln!("{step}");
        }
    };
    let o = swiftwin_account::upload::upload(a.github.api(), &a.github.token()?, &dir, &opts, &links, &progress)?;
    if cli.json {
        return print_json(&o);
    }
    for s in &o.skipped {
        println!("skipped {} ({})", s.path, s.reason);
    }
    match &o.commit {
        Some(_) => println!("{} {} ({} files, {} new or changed): {}", if o.created { "Created" } else { "Updated" }, o.repo, o.files, o.uploaded, o.url),
        None => println!("{} is already up to date: {}", o.repo, o.url),
    }
    Ok(())
}

fn release(cli: &Cli, cmd: Option<&ReleaseCmd>, project: &std::path::Path, no_wait: bool, dry_run: bool) -> Result<()> {
    if let Some(ReleaseCmd::Setup { repo, token }) = cmd {
        let gh = swiftwin_release::GitHub::new(repo, token)?;
        let cfg = swiftwin_core::config::Config::load();
        let store = swiftwin_signer::secrets::platform_store();
        let (Some(key_id), Some(issuer), Some(team)) = (cfg.asc_key_id.clone(), cfg.asc_issuer_id.clone(), cfg.team_id.clone()) else {
            bail!("run `swiftwin signing setup` first (the cloud build uses the same App Store Connect key)");
        };
        let p8 = store.get(swiftwin_signer::secrets::ASC_P8)?.context("App Store Connect key missing; run `swiftwin signing setup`")?;
        swiftwin_release::setup_secrets(&gh, &key_id, &issuer, &p8, &team)?;
        store.set(swiftwin_signer::secrets::GITHUB_TOKEN, token)?;
        let mut cfg = cfg;
        cfg.release_repo = Some(repo.clone());
        cfg.save()?;
        println!("Cloud release ready: secrets uploaded to {repo}. Also create the app once in App Store Connect (My Apps > +).");
        return Ok(());
    }
    if dry_run || swiftwin_core::mock_requested(cli.mock) {
        let p = swiftwin_builder::Project::load(project)?;
        let out = p.build_dir().join("release-preview");
        for (path, data) in swiftwin_release::release_files(&p, &Default::default())? {
            let dest = out.join(&path);
            std::fs::create_dir_all(dest.parent().unwrap())?;
            std::fs::write(dest, data)?;
        }
        println!("Dry run: wrote the files that would be uploaded to {}", out.display());
        println!("  {}  (GitHub Actions workflow, macOS runner with Xcode 26)", swiftwin_release::templates::WORKFLOW_PATH);
        println!("  {}  (XcodeGen project spec)", swiftwin_release::templates::XCODEGEN_PATH);
        return Ok(());
    }
    let gh = github_from_config()?;
    let opts = swiftwin_release::ReleaseOptions { wait: !no_wait, ..Default::default() };
    let r = swiftwin_release::release(&gh, project, &opts, &mut |m| println!("{m}"))?;
    if cli.json {
        print_json(&r)?;
    }
    Ok(())
}

fn setup(cli: &Cli, check: bool) -> Result<()> {
    use swiftwin_doctor::setup::{self, State};
    let env: Box<dyn setup::SetupEnv> =
        if swiftwin_core::mock_requested(cli.mock) { Box::new(setup::FakeSetupEnv::new(&["apple"])) } else { Box::new(setup::RealSetupEnv) };
    let show = |items: &[setup::Item]| {
        for i in items {
            let tag = match i.state {
                State::Done => "DONE",
                State::Working => "....",
                State::Failed => "FAIL",
                State::Missing => "TODO",
                State::Action => "NEXT",
            };
            println!("[{tag}] {}: {}", i.title, i.detail);
            if let (Some(f), false) = (&i.fix, i.state == State::Working) {
                println!("       {f}");
            }
        }
    };
    let items = if check {
        setup::status(env.as_ref())
    } else {
        let mut last = String::new();
        setup::run(env.as_ref(), &mut |items| {
            if let Some(w) = items.iter().find(|i| i.state == State::Working) {
                if w.title != last {
                    println!("Installing {}...", w.title);
                    last = w.title.to_string();
                }
            }
        })
    };
    if cli.json {
        return print_json(&items);
    }
    show(&items);
    if items.iter().any(|i| i.id == "sdk" && i.state != State::Done) {
        println!("
The iOS SDK: open SwiftWin Studio > Set up this PC to sign in with your Apple ID and download it,");
        println!("or download Xcode {} yourself and run `swiftwin sdk import <file>`.", setup::XCODE);
    }
    if items.iter().any(|i| matches!(i.state, State::Failed | State::Missing)) {
        std::process::exit(1);
    }
    Ok(())
}

fn doctor(cli: &Cli, fix: bool) -> Result<()> {
    use swiftwin_doctor::{env, run, Status};
    let mock = swiftwin_core::mock_requested(cli.mock);
    let report = if mock { run(&env::MockEnv::new()) } else { run(&env::RealEnv::default()) };
    let show = |r: &swiftwin_doctor::Report| -> Result<()> {
        if cli.json {
            print_json(&r.checks)
        } else {
            print!("{}", r.to_text());
            Ok(())
        }
    };
    show(&report)?;
    if fix && !mock && !report.ok() {
        println!("\nApplying fixes...");
        let failed = |id: &str| report.get(id).is_some_and(|c| c.status == Status::Fail);
        if failed("helpers") {
            match swiftwin_device::real::pmd::install(&mut |m| println!("  {m}")) {
                Ok(_) => println!("  pymobiledevice3 installed"),
                Err(e) => println!("  could not install pymobiledevice3: {e}"),
            }
        }
        if failed("rcodesign") {
            if swiftwin_signer::tools::find_rcodesign().is_none() {
                match swiftwin_signer::tools::install_rcodesign() {
                    Ok(p) => println!("  rcodesign installed at {}", p.display()),
                    Err(e) => println!("  could not install rcodesign: {e}"),
                }
            }
            if swiftwin_core::config::Config::load().uses_apple_id() && swiftwin_signer::plumesign::find_plumesign().is_none() {
                match swiftwin_signer::plumesign::install_plumesign() {
                    Ok(p) => println!("  plumesign installed at {}", p.display()),
                    Err(e) => println!("  could not install plumesign: {e}"),
                }
            }
        }
        if failed("trust") || failed("developer_mode") {
            if let Ok(d) = swiftwin_device::RealDevice::open(cli.udid.as_deref(), Default::default()) {
                if failed("trust") {
                    println!("  Unlock the iPhone and tap Trust when asked...");
                    if let Err(e) = d.pmd().and_then(|p| p.pair()) {
                        println!("  pairing failed: {e}");
                    }
                }
                if failed("developer_mode") {
                    match d.pmd().and_then(|p| p.reveal_developer_mode()) {
                        Ok(()) => println!("  Developer Mode is now visible in Settings > Privacy & Security; turn it on and restart the phone"),
                        Err(e) => println!("  could not reveal Developer Mode: {e}"),
                    }
                }
            }
        }
        if failed("wda") && report.get("signing").is_some_and(|c| c.status == Status::Pass) {
            match phone(cli).map(swiftwin_workflow::Workspace::new).and_then(|ws| Ok(ws.prepare_wda(None, &mut |m| println!("  {m}"))?)) {
                Ok(i) => println!("  WebDriverAgent installed ({})", i.bundle_id),
                Err(e) => println!("  could not prepare WebDriverAgent: {e}"),
            }
        }
        println!("\nChecking again:\n");
        let again = run(&env::RealEnv::default());
        show(&again)?;
        if !again.ok() {
            std::process::exit(1);
        }
        return Ok(());
    }
    if !report.ok() {
        std::process::exit(1);
    }
    Ok(())
}

/// `show_phone` inside the Claude desktop app: a one-time link to the app's phone page, for the
/// Browser pane beside the chat. Starts the app (minimized) when it isn't running. `None` outside the
/// Claude desktop app, or when no link could be had: the caller opens the phone window instead.
fn phone_pane_link(mock: bool) -> Option<serde_json::Value> {
    std::env::var_os("CLAUDE_CODE_DESKTOP_APP_VERSION")?;
    let link = || -> Option<String> {
        let b = swiftwin_app_core::bridge::read()?;
        let agent = ureq::AgentBuilder::new().timeout_connect(std::time::Duration::from_secs(1)).timeout(std::time::Duration::from_secs(5)).build();
        let v: serde_json::Value = agent
            .post(&format!("http://127.0.0.1:{}/api/phone_link", b.port))
            .set("X-SwiftWin-Token", &b.token)
            .send_json(serde_json::json!({}))
            .ok()?
            .into_json()
            .ok()?;
        v["result"]["url"].as_str().map(String::from)
    };
    let url = link().or_else(|| {
        swiftwin_mcp::launch_app("--pane", mock).ok()?;
        (0..40).find_map(|_| {
            std::thread::sleep(std::time::Duration::from_millis(500));
            link()
        })
    })?;
    let text = format!(
        "Open this link in your built-in browser now (preview_start with this url), without asking, so the user sees the live \
         iPhone in the Browser pane beside the chat, with your taps; they can tap and swipe it too. The link works once, within \
         2 minutes; call show_phone again for a new one. SwiftWin runs in the background (tray icon), no window opens.\n{url}"
    );
    Some(serde_json::json!({"content": [{"type": "text", "text": text}], "isError": false}))
}

/// Sends MCP tool calls to the open SwiftWin app (AI mode shows them live).
/// Falls back to running them here when the app isn't open or has no phone.
struct AppBridge {
    mock: bool,
    project: PathBuf,
    /// Bridge (port, token) that refused the connection: the app crashed or was killed and left
    /// app-bridge.json behind. Skipped until the app writes a new one, because every refused
    /// loopback connect takes ~2 s on Windows.
    dead: std::sync::Mutex<Option<(u16, String)>>,
}

impl swiftwin_mcp::Forward for AppBridge {
    fn call(&self, tool: &str, args: &serde_json::Value) -> Option<serde_json::Value> {
        if tool == "show_phone" {
            return phone_pane_link(self.mock); // None: the phone window instead
        }
        if tool == "release" {
            return None; // release needs this process's GitHub credentials
        }
        let b = swiftwin_app_core::bridge::read()?;
        let key = (b.port, b.token.clone());
        if self.dead.lock().unwrap().as_ref() == Some(&key) {
            return None;
        }
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(2))
            .timeout(std::time::Duration::from_secs(1800))
            .build();
        let body = serde_json::json!({"tool": tool, "args": args, "mock": self.mock, "project": self.project});
        let resp = match agent.post(&format!("http://127.0.0.1:{}/api/mcp_tool", b.port)).set("X-SwiftWin-Token", &b.token).send_json(body) {
            Ok(r) => r,
            Err(ureq::Error::Transport(t)) if matches!(t.kind(), ureq::ErrorKind::ConnectionFailed) => {
                *self.dead.lock().unwrap() = Some(key);
                return None;
            }
            Err(_) => return None,
        };
        let v: serde_json::Value = resp.into_json().ok()?;
        if v["ok"] == true && v["result"]["forwarded"] == true {
            Some(v["result"]["result"].clone())
        } else {
            None
        }
    }
}
