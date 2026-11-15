//! Cloud release: Apple requires Xcode 26+ for App Store / TestFlight uploads,
//! so this one step runs on a macOS GitHub Actions runner.
//!
//! `swiftwin release`: commit the project + generated workflow/XcodeGen spec to
//! the `swiftwin/release` branch of the user's GitHub repo (Git Data API),
//! which triggers the workflow; then follow the run until it finishes.
//! `swiftwin release setup`: store the GitHub token (Credential Manager) and
//! upload the App Store Connect key as encrypted repository secrets.

pub mod github;
pub mod templates;

use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;
use swiftwin_builder::Project;
use swiftwin_core::{Result, SwiftWinError};

pub use github::{GitHub, RunInfo};
pub use templates::ReleaseSettings;

const MAX_FILE: u64 = 50 * 1024 * 1024;

/// Project files to upload (skips build output, VCS and hidden folders) + generated files.
pub fn release_files(project: &Project, settings: &ReleaseSettings) -> Result<Vec<(String, Vec<u8>)>> {
    let root = &project.dir;
    let mut files = Vec::new();
    let mut too_big = Vec::new();
    swiftwin_builder::project::walk(root, &mut |p| {
        let rel = p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/");
        let hidden = rel.split('/').any(|c| c.starts_with('.'));
        if hidden || rel.starts_with("build/") {
            return;
        }
        match std::fs::metadata(p) {
            Ok(m) if m.len() > MAX_FILE => too_big.push(rel),
            Ok(_) => {
                if let Ok(d) = std::fs::read(p) {
                    files.push((rel, d));
                }
            }
            Err(_) => {}
        }
    })?;
    if !too_big.is_empty() {
        return Err(SwiftWinError::invalid(format!("files over 50 MB cannot be uploaded: {}", too_big.join(", "))));
    }
    files.retain(|(p, _)| p != templates::WORKFLOW_PATH && p != templates::XCODEGEN_PATH && p != templates::EXPORT_OPTIONS_PATH);
    files.push((templates::WORKFLOW_PATH.into(), templates::workflow(project, settings).into_bytes()));
    files.push((templates::XCODEGEN_PATH.into(), templates::xcodegen(project).into_bytes()));
    files.push((templates::EXPORT_OPTIONS_PATH.into(), templates::export_options().into_bytes()));
    Ok(files)
}

/// Upload the App Store Connect key and team id as encrypted repository secrets.
// UNTESTED-ON-DEVICE: against the owner's real GitHub repository.
pub fn setup_secrets(gh: &GitHub, key_id: &str, issuer_id: &str, p8_pem: &str, team_id: &str) -> Result<()> {
    gh.set_secret("SWIFTWIN_ASC_KEY_ID", key_id.as_bytes())?;
    gh.set_secret("SWIFTWIN_ASC_ISSUER_ID", issuer_id.as_bytes())?;
    gh.set_secret("SWIFTWIN_ASC_KEY_P8", swiftwin_core::base64_bytes::encode(p8_pem.as_bytes()).as_bytes())?;
    gh.set_secret("SWIFTWIN_TEAM_ID", team_id.as_bytes())?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct ReleaseResult {
    pub commit: String,
    pub run_url: Option<String>,
    pub conclusion: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReleaseOptions {
    pub settings: ReleaseSettings,
    pub wait: bool,
    pub poll: Duration,
    pub timeout: Duration,
}

impl Default for ReleaseOptions {
    fn default() -> Self {
        ReleaseOptions {
            settings: ReleaseSettings::default(),
            wait: true,
            poll: Duration::from_secs(15),
            timeout: Duration::from_secs(75 * 60),
        }
    }
}

/// Push the project to the release branch and follow the cloud build.
// UNTESTED-ON-DEVICE: real GitHub runner + Xcode 26 + TestFlight upload.
pub fn release(gh: &GitHub, project_dir: &Path, opts: &ReleaseOptions, progress: &mut dyn FnMut(&str)) -> Result<ReleaseResult> {
    let project = Project::load(project_dir)?;
    let files = release_files(&project, &opts.settings)?;
    progress(&format!("Uploading {} files to {}/{} ({}) ...", files.len(), gh.owner, gh.repo, templates::BRANCH));
    let msg = format!("SwiftWin release: {} {}", project.app().name, project.app().version);
    let commit = gh.commit_files(templates::BRANCH, &msg, &files)?;
    progress(&format!("Committed {}; waiting for the macOS runner ...", &commit[..commit.len().min(7)]));
    let start = Instant::now();
    let deadline = start + opts.timeout;
    let mut run: Option<RunInfo> = None;
    let mut last_status = String::new();
    loop {
        if let Some(r) = gh.run_for_commit(&commit)? {
            if r.status != last_status {
                progress(&format!("Cloud build {}: {}", r.status, r.url));
                last_status = r.status.clone();
            }
            let done = r.status == "completed";
            run = Some(r);
            if done || !opts.wait {
                break;
            }
        } else if !opts.wait && start.elapsed() > Duration::from_secs(60) {
            break; // run not visible yet; the caller can check GitHub later
        }
        if Instant::now() > deadline {
            return Err(SwiftWinError::Timeout("the cloud build did not finish in time; check GitHub Actions".into()));
        }
        std::thread::sleep(opts.poll);
    }
    let (run_url, conclusion) = match &run {
        Some(r) => (Some(r.url.clone()), r.conclusion.clone()),
        None => (None, None),
    };
    if opts.wait {
        if let Some(c) = &conclusion {
            if c != "success" {
                return Err(SwiftWinError::tool(
                    "cloud build",
                    format!("finished with `{c}`; open {} for the Xcode log", run_url.clone().unwrap_or_default()),
                ));
            }
            progress("Uploaded to App Store Connect. It appears in TestFlight after Apple finishes processing (usually 5–30 min).");
        }
    }
    Ok(ReleaseResult { commit, run_url, conclusion })
}

#[cfg(test)]
mod fake_github;

#[cfg(test)]
mod tests {
    use super::*;

    fn project(tag: &str) -> (std::path::PathBuf, Project) {
        let d = std::env::temp_dir().join(format!("swiftwin-rel-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let dir = swiftwin_builder::scaffold::new_project(&d, "Shop", Some("com.example.shop")).unwrap();
        std::fs::write(dir.join("swiftwin.toml"), std::fs::read_to_string(dir.join("swiftwin.toml")).unwrap() + "NSCameraUsageDescription = \"Scan \\\"codes\\\"\"\n").unwrap();
        std::fs::create_dir_all(dir.join("build/debug")).unwrap();
        std::fs::write(dir.join("build/debug/junk"), b"x").unwrap();
        (d, Project::load(&dir).unwrap())
    }

    #[test]
    fn generated_files_are_valid() {
        let (d, p) = project("tpl");
        let files = release_files(&p, &ReleaseSettings::default()).unwrap();
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"Sources/ContentView.swift"));
        assert!(names.contains(&"Assets.xcassets/AppIcon.appiconset/icon-1024.png"));
        assert!(!names.iter().any(|n| n.starts_with("build/") || n.starts_with(".git/") || *n == ".gitignore"));
        let wf = String::from_utf8(files.iter().find(|(n, _)| n == templates::WORKFLOW_PATH).unwrap().1.clone()).unwrap();
        let y: serde_yaml::Value = serde_yaml::from_str(&wf).unwrap();
        assert_eq!(y["jobs"]["testflight"]["runs-on"], "macos-26");
        assert_eq!(y["on"]["push"]["branches"][0], "swiftwin/release");
        let steps = y["jobs"]["testflight"]["steps"].as_sequence().unwrap();
        assert!(steps.iter().any(|s| s["run"].as_str().is_some_and(|r| r.contains("xcodebuild archive"))));
        // Least privilege: read-only token, the key only where it is used, actions pinned by commit.
        assert_eq!(y["permissions"]["contents"], "read");
        assert!(y["jobs"]["testflight"]["env"].is_null(), "no secrets for every step");
        for s in steps {
            let uses_key = s["env"]["ASC_KEY_P8_BASE64"].is_string();
            assert_eq!(uses_key, s["name"] == "App Store Connect API key", "{s:?}");
            if let Some(u) = s["uses"].as_str() {
                let sha = u.split_once('@').unwrap().1;
                assert!(sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()), "{u} is not pinned to a commit");
            }
            if s["name"] == "Install XcodeGen" {
                assert!(s["env"].is_null(), "brew runs without secrets");
            }
        }
        let xg = String::from_utf8(files.iter().find(|(n, _)| n == templates::XCODEGEN_PATH).unwrap().1.clone()).unwrap();
        let x: serde_yaml::Value = serde_yaml::from_str(&xg).unwrap_or_else(|e| panic!("{e}\n{xg}"));
        let t = &x["targets"]["Shop"];
        assert_eq!(t["settings"]["base"]["PRODUCT_BUNDLE_IDENTIFIER"], "com.example.shop");
        assert_eq!(t["info"]["properties"]["NSCameraUsageDescription"], "Scan \"codes\"");
        assert_eq!(x["options"]["deploymentTarget"]["iOS"], "17.0");
        let eo = files.iter().find(|(n, _)| n == templates::EXPORT_OPTIONS_PATH).unwrap();
        assert!(String::from_utf8_lossy(&eo.1).contains("<key>destination</key><string>upload</string>"));
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn release_flow_against_fake_github() {
        let (d, p) = project("flow");
        let fake = fake_github::FakeGitHub::start();
        let gh = GitHub::with_base(&fake.url(), "me/shop", "ghp_test").unwrap();
        setup_secrets(&gh, "KEY1", "issuer", "-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----\n", "TEAM123456").unwrap();
        let secrets = fake.secrets();
        assert_eq!(secrets.get("SWIFTWIN_TEAM_ID").map(|v| v.as_slice()), Some(&b"TEAM123456"[..]));
        assert_eq!(secrets.len(), 4);

        let opts = ReleaseOptions { poll: Duration::from_millis(20), timeout: Duration::from_secs(10), ..Default::default() };
        let mut log = Vec::new();
        let r = release(&gh, &p.dir, &opts, &mut |m| log.push(m.to_string())).unwrap();
        assert_eq!(r.conclusion.as_deref(), Some("success"));
        assert!(r.run_url.unwrap().contains("/actions/runs/"));
        assert!(log.iter().any(|l| l.contains("in_progress")));
        let st = fake.state();
        assert!(st.files.contains(&templates::WORKFLOW_PATH.to_string()));
        assert!(st.ref_created);
        // Second release updates the branch (force) with the previous commit as parent.
        fake.set_next_conclusion("failure");
        let e = release(&gh, &p.dir, &opts, &mut |_| {}).unwrap_err().to_string();
        assert!(e.contains("failure"), "{e}");
        assert!(fake.state().ref_updated);
        std::fs::remove_dir_all(d).ok();
    }
}
