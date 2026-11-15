//! Preparing WebDriverAgent for the user's phone.
//!
//! Building WDA from source needs Xcode (Objective-C + XCTest), so SwiftWin
//! starts from appium's unsigned real-device build
//! (`WebDriverAgentRunner-Runner.zip`), gives it bundle ids under the user's
//! control, and then signs it like any other app with a development profile.

use std::path::{Path, PathBuf};

use swiftwin_core::{Result, SwiftWinError};

/// Bundle id for the runner, unique per team so App IDs don't collide.
pub fn runner_bundle_id(team_id: &str) -> String {
    format!("dev.swiftwin.{}.WebDriverAgentRunner.xctrunner", team_id.to_ascii_lowercase())
}

/// Find `*.app` that contains `PlugIns/*.xctest` below `root`.
pub fn find_runner_app(root: &Path) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).ok()?.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            if p.extension().is_some_and(|x| x == "app") && p.join("PlugIns").is_dir() {
                return Some(p);
            }
            stack.push(p);
        }
    }
    None
}

fn set_bundle_id(info: &Path, id: &str) -> Result<()> {
    let mut d: plist::Dictionary =
        plist::from_file(info).map_err(|e| SwiftWinError::invalid(format!("{}: {e}", info.display())))?;
    d.insert("CFBundleIdentifier".into(), id.into());
    plist::Value::Dictionary(d).to_file_xml(info).map_err(|e| SwiftWinError::other(e.to_string()))
}

/// Delete every `*.dSYM` folder below `dir`. appium's zip ships
/// `PlugIns/WebDriverAgentRunner.xctest.dSYM` inside the bundle, which only adds size and
/// one more nested bundle for a signer to trip over.
pub fn remove_dsyms(dir: &Path) -> Result<()> {
    for e in std::fs::read_dir(dir)?.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dSYM")) {
            std::fs::remove_dir_all(&p)?;
        } else {
            remove_dsyms(&p)?;
        }
    }
    Ok(())
}

/// Unpack the runner zip into `work` (emptied first) and drop what a signer must not see:
/// dSYMs and any old provisioning profile. Bundle ids are left as appium ships them.
pub fn unpack_runner(zip_bytes: &[u8], work: &Path) -> Result<PathBuf> {
    let _ = std::fs::remove_dir_all(work);
    std::fs::create_dir_all(work)?;
    crate::tools::unzip_all(zip_bytes, work)?;
    let app = find_runner_app(work).ok_or_else(|| SwiftWinError::invalid("no WebDriverAgentRunner .app with PlugIns in the zip"))?;
    remove_dsyms(&app)?;
    let _ = std::fs::remove_file(app.join("embedded.mobileprovision"));
    Ok(app)
}

/// Unpack the runner zip into `work`, rename bundle ids. Returns (runner .app, its bundle id).
pub fn prepare_runner(zip_bytes: &[u8], work: &Path, team_id: &str) -> Result<(PathBuf, String)> {
    let app = unpack_runner(zip_bytes, work)?;
    let runner_id = runner_bundle_id(team_id);
    set_bundle_id(&app.join("Info.plist"), &runner_id)?;
    let plugin_id = runner_id.trim_end_matches(".xctrunner").to_string();
    for e in std::fs::read_dir(app.join("PlugIns"))?.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "xctest") && p.join("Info.plist").exists() {
            set_bundle_id(&p.join("Info.plist"), &plugin_id)?;
        }
    }
    crate::codesign::strip_signatures(&app)?;
    Ok((app, runner_id))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    pub fn fake_wda_zip() -> Vec<u8> {
        let info = |id: &str| {
            let mut d = plist::Dictionary::new();
            d.insert("CFBundleIdentifier".into(), id.into());
            d.insert("CFBundleName".into(), "WebDriverAgentRunner-Runner".into());
            let mut v = Vec::new();
            plist::Value::Dictionary(d).to_writer_xml(&mut v).unwrap();
            v
        };
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        // Same layout as appium's v16.15.3 zip: the .app at the root, with a dSYM inside.
        let base = "WebDriverAgentRunner-Runner.app";
        for (name, data) in [
            (format!("{base}/Info.plist"), info("com.facebook.WebDriverAgentRunner.xctrunner")),
            (format!("{base}/WebDriverAgentRunner-Runner"), vec![0xcf, 0xfa, 0xed, 0xfe]),
            (format!("{base}/_CodeSignature/CodeResources"), b"old".to_vec()),
            (format!("{base}/PlugIns/WebDriverAgentRunner.xctest/Info.plist"), info("com.facebook.WebDriverAgentRunner")),
            (format!("{base}/PlugIns/WebDriverAgentRunner.xctest/WebDriverAgentRunner"), vec![0xcf, 0xfa, 0xed, 0xfe]),
            (format!("{base}/PlugIns/WebDriverAgentRunner.xctest.dSYM/Contents/Info.plist"), info("com.apple.xcode.dsym")),
        ] {
            z.start_file(name, o).unwrap();
            z.write_all(&data).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    #[test]
    fn rebundles_runner() {
        let work = std::env::temp_dir().join(format!("swiftwin-wda-{}", std::process::id()));
        let (app, id) = prepare_runner(&fake_wda_zip(), &work, "ABCDE12345").unwrap();
        assert_eq!(id, "dev.swiftwin.abcde12345.WebDriverAgentRunner.xctrunner");
        let d: plist::Dictionary = plist::from_file(app.join("Info.plist")).unwrap();
        assert_eq!(d["CFBundleIdentifier"].as_string(), Some(id.as_str()));
        let p: plist::Dictionary = plist::from_file(app.join("PlugIns/WebDriverAgentRunner.xctest/Info.plist")).unwrap();
        assert_eq!(p["CFBundleIdentifier"].as_string(), Some("dev.swiftwin.abcde12345.WebDriverAgentRunner"));
        assert!(!app.join("_CodeSignature").exists());
        assert!(!app.join("PlugIns/WebDriverAgentRunner.xctest.dSYM").exists());
        std::fs::remove_dir_all(work).ok();
    }

    #[test]
    fn unpacks_runner_for_another_signer() {
        let work = std::env::temp_dir().join(format!("swiftwin-wda-unpack-{}", std::process::id()));
        let app = unpack_runner(&fake_wda_zip(), &work).unwrap();
        assert_eq!(app.file_name().unwrap(), "WebDriverAgentRunner-Runner.app");
        // Ids stay appium's; the sideloader picks the final one.
        let d: plist::Dictionary = plist::from_file(app.join("Info.plist")).unwrap();
        assert_eq!(d["CFBundleIdentifier"].as_string(), Some("com.facebook.WebDriverAgentRunner.xctrunner"));
        assert!(!app.join("PlugIns/WebDriverAgentRunner.xctest.dSYM").exists());
        assert!(app.join("PlugIns/WebDriverAgentRunner.xctest/WebDriverAgentRunner").exists());
        std::fs::remove_dir_all(work).ok();
    }
}
