//! Where SwiftWin keeps its data on disk.
//!
//! Windows: `%LOCALAPPDATA%\SwiftWin\data` (the installer puts the app itself in `%LOCALAPPDATA%\SwiftWin`). Elsewhere: `$XDG_DATA_HOME/swiftwin` or
//! `~/.local/share/swiftwin`. `SWIFTWIN_HOME` overrides everything (used by tests).

use std::path::{Path, PathBuf};

pub fn data_dir() -> PathBuf {
    if let Some(h) = std::env::var_os("SWIFTWIN_HOME") {
        return PathBuf::from(h);
    }
    if cfg!(windows) {
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(l).join("SwiftWin").join("data");
        }
    }
    if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(x).join("swiftwin");
    }
    if let Some(h) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        return PathBuf::from(h).join(".local").join("share").join("swiftwin");
    }
    PathBuf::from(".swiftwin")
}

/// Extracted iOS SDKs (never committed, never redistributed).
pub fn sdk_dir() -> PathBuf {
    data_dir().join("sdk")
}

/// Python virtualenv holding prototype helpers (pymobiledevice3).
pub fn venv_dir() -> PathBuf {
    data_dir().join("venv")
}

/// Signing material that is not secret (certificates, profiles). Private keys
/// go to the Windows Credential Manager instead.
pub fn signing_dir() -> PathBuf {
    data_dir().join("signing")
}

/// Downloaded/built tools (WebDriverAgent, rcodesign, ...).
pub fn tools_dir() -> PathBuf {
    data_dir().join("tools")
}

/// Private copies of SwiftWin's folder that Windows keeps for a Microsoft Store (MSIX) app, at
/// `%LOCALAPPDATA%\Packages\<package>\LocalCache\Local\SwiftWin`.
///
/// Programs started from such an app (for example `swiftwin mcp` started by the Claude desktop app)
/// have a *new* top-level AppData folder redirected into that app's private storage, and from then
/// on keep reading and writing the private copy, while the SwiftWin app and a normal terminal use the
/// real folder. Writes into a folder that already exists for real are not redirected, so this only
/// happens when SwiftWin's folder was first created from inside such an app. Empty when there are none.
pub fn sandboxed_copies() -> Vec<PathBuf> {
    sandboxed_copies_in(&data_dir(), std::env::var_os("LOCALAPPDATA").map(PathBuf::from).as_deref())
}

/// [`sandboxed_copies`] for an explicit data dir and `%LOCALAPPDATA%` (testable on any OS).
pub fn sandboxed_copies_in(data: &Path, local_app_data: Option<&Path>) -> Vec<PathBuf> {
    let Some(local) = local_app_data else { return Vec::new() };
    let Some(top) = data.strip_prefix(local).ok().and_then(|rel| rel.components().next()) else { return Vec::new() };
    let Ok(packages) = std::fs::read_dir(local.join("Packages")) else { return Vec::new() };
    let mut found: Vec<PathBuf> =
        packages.flatten().map(|e| e.path().join("LocalCache").join("Local").join(top.as_os_str())).filter(|p| p.exists()).collect();
    found.sort();
    found
}

/// The first `name` in a folder on `PATH`. Relative entries (`.`, empty ones from `;;`) are
/// skipped: they mean the current folder, which for `swiftwin mcp` and AI mode is the user's
/// project, so a project could otherwise ship its own `rcodesign.exe` and have it run.
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    find_in(std::env::var_os("PATH")?.as_os_str(), name)
}

fn find_in(path: &std::ffi::OsStr, name: &str) -> Option<PathBuf> {
    std::env::split_paths(path).filter(|d| d.is_absolute()).map(|d| d.join(name)).find(|p| p.is_file())
}

/// Is `rel` a plain relative path that stays inside whatever folder it is joined to, on Windows
/// too? Only normal components; none may be a Windows device name (`CON`, `nul.txt`, `COM1`…),
/// contain `:` (drive letters, NTFS alternate data streams), wildcards or control characters, or
/// end in a dot or space (Windows drops those, so `a.swift.` would alias `a.swift`).
pub fn is_safe_relative(rel: &Path) -> bool {
    use std::path::Component;
    let mut any = false;
    for c in rel.components() {
        let Component::Normal(name) = c else { return false };
        let Some(name) = name.to_str() else { return false };
        if !is_safe_file_name(name) {
            return false;
        }
        any = true;
    }
    any
}

fn is_safe_file_name(name: &str) -> bool {
    if name.is_empty() || name.ends_with('.') || name.ends_with(' ') {
        return false;
    }
    if name.chars().any(|c| c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\\' | '/')) {
        return false;
    }
    let stem = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem[3..].chars().all(|c| c.is_ascii_digit() || matches!(c, '¹' | '²' | '³')))
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && matches!(&stem[3..], "¹" | "²" | "³"));
    !device
}

/// Replace `path` with `data` whole: write a temp file next to it, then rename it over. A reader
/// (another process) never sees a half-written file, and a crash or full disk leaves the old one.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp-{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// `root.join(rel)`, refused unless `rel` [`is_safe_relative`] and the result, after following
/// any symlinks or junctions in the part that already exists, is still inside `root`.
pub fn join_inside(root: &Path, rel: &Path) -> Option<PathBuf> {
    if !is_safe_relative(rel) {
        return None;
    }
    let joined = root.join(rel);
    let real_root = std::fs::canonicalize(root).ok()?;
    // The deepest part of the path that exists decides where it really points.
    let mut existing = joined.as_path();
    while !existing.exists() {
        existing = existing.parent()?;
    }
    let real = std::fs::canonicalize(existing).ok()?;
    real.starts_with(&real_root).then_some(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_lookup_skips_relative_entries() {
        let base = std::env::temp_dir().join(format!("swiftwin-find-path-{}", std::process::id()));
        std::fs::create_dir_all(base.join("bin")).unwrap();
        std::fs::write(base.join("bin").join("tool.exe"), "").unwrap();
        // Run from a folder that has its own tool.exe: it must not be found through "." or "".
        let cwd = std::env::current_dir().unwrap();
        let rel = Path::new("swiftwin-find-path-rel-test-tool.exe");
        std::fs::write(cwd.join(rel), "").unwrap();
        let p = std::env::join_paths([PathBuf::from("."), PathBuf::from(""), base.join("bin")]).unwrap();
        assert_eq!(find_in(&p, "tool.exe"), Some(base.join("bin").join("tool.exe")));
        let only_rel = std::env::join_paths([PathBuf::from("."), PathBuf::from("sub/..")]).unwrap();
        assert_eq!(find_in(&only_rel, rel.to_str().unwrap()), None);
        std::fs::remove_file(cwd.join(rel)).ok();
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn safe_relative_paths() {
        for ok in ["a.swift", "Sources/ContentView.swift", "Assets.xcassets/AppIcon.appiconset/Contents.json", "console/x", "COM10", "nulls.txt"] {
            assert!(is_safe_relative(Path::new(ok)), "{ok}");
        }
        for bad in [
            "", "..", "../x", "a/../../x", "/etc/passwd", "C:/Windows/win.ini", "C:x", "\\\\server\\share\\x", "a.swift::$DATA",
            "a:b", "CON", "nul.txt", "Sources/aux", "com1.swift", "LPT9", "COM¹", "con.d", "x.", "x ", "a*?", "a\u{0}b",
        ] {
            assert!(!is_safe_relative(Path::new(bad)), "{bad:?} must be refused");
        }
    }

    #[test]
    fn join_inside_refuses_links_out_of_the_root() {
        let base = std::env::temp_dir().join(format!("swiftwin-join-inside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (root, outside) = (base.join("project"), base.join("outside"));
        std::fs::create_dir_all(root.join("Sources")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        assert_eq!(join_inside(&root, Path::new("Sources/New.swift")), Some(root.join("Sources/New.swift")));
        assert_eq!(join_inside(&root, Path::new("New/Deep/File.swift")), Some(root.join("New/Deep/File.swift")));
        assert!(join_inside(&root, Path::new("../outside/x")).is_none());
        // A link inside the project that points out of it (a junction needs no privilege on Windows).
        let link = root.join("escape");
        #[cfg(windows)]
        let made = std::process::Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(&link).arg(&outside).output().is_ok_and(|o| o.status.success());
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&outside, &link).is_ok();
        if made {
            assert!(join_inside(&root, Path::new("escape/secret.txt")).is_none(), "followed a link out of the project");
            assert!(join_inside(&root, Path::new("escape")).is_none());
        }
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn finds_private_copies_of_the_data_folder() {
        let local = std::env::temp_dir().join(format!("swiftwin-paths-sandbox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&local);
        let shadow = local.join("Packages").join("Claude_pzs8sxrjxfjjc").join("LocalCache").join("Local").join("SwiftWin");
        std::fs::create_dir_all(shadow.join("data")).unwrap();
        std::fs::create_dir_all(local.join("Packages").join("Other_123").join("LocalCache").join("Local").join("Something")).unwrap();
        let data = local.join("SwiftWin").join("data");
        assert_eq!(sandboxed_copies_in(&data, Some(&local)), vec![shadow]);
        assert!(sandboxed_copies_in(Path::new("/elsewhere/swiftwin"), Some(&local)).is_empty(), "SWIFTWIN_HOME outside AppData");
        assert!(sandboxed_copies_in(&data, None).is_empty());
        std::fs::remove_dir_all(&local).ok();
    }
}
