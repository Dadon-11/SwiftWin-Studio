//! `swiftwin.toml` – the project file.
//!
//! ```toml
//! [app]
//! name = "Hello"
//! bundle_id = "com.example.hello"
//! version = "1.0"
//! build = "1"
//! deployment_target = "17.0"
//!
//! [info]            # extra Info.plist keys
//! NSCameraUsageDescription = "Scan things"
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use swiftwin_core::{Result, SwiftWinError};

pub const PROJECT_FILE: &str = "swiftwin.toml";

/// Values of the `[info]` table.
pub type TomlValue = toml::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSection {
    pub name: String,
    pub bundle_id: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default = "default_build")]
    pub build: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default = "default_target")]
    pub deployment_target: String,
    #[serde(default = "default_sources")]
    pub sources: String,
    #[serde(default = "default_resources")]
    pub resources: String,
    #[serde(default = "default_assets")]
    pub assets: String,
    /// "portrait", "landscape-left", "landscape-right", "upside-down".
    #[serde(default = "default_orientations")]
    pub orientations: Vec<String>,
}

fn default_version() -> String {
    "1.0".into()
}
fn default_build() -> String {
    "1".into()
}
fn default_target() -> String {
    "17.0".into()
}
fn default_sources() -> String {
    "Sources".into()
}
fn default_resources() -> String {
    "Resources".into()
}
fn default_assets() -> String {
    "Assets.xcassets".into()
}
fn default_orientations() -> Vec<String> {
    vec!["portrait".into()]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectFile {
    pub app: AppSection,
    #[serde(default)]
    pub info: toml::Table,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub dir: PathBuf,
    pub file: ProjectFile,
}

impl Project {
    /// Load `swiftwin.toml` from `dir` (or from a parent directory).
    pub fn load(dir: &Path) -> Result<Project> {
        let mut d = Some(dir);
        while let Some(cur) = d {
            let f = cur.join(PROJECT_FILE);
            if f.exists() {
                let text = std::fs::read_to_string(&f)?;
                let file: ProjectFile =
                    toml::from_str(&text).map_err(|e| SwiftWinError::invalid(format!("{}: {e}", f.display())))?;
                let p = Project { dir: cur.to_path_buf(), file };
                p.validate()?;
                return Ok(p);
            }
            d = cur.parent();
        }
        Err(SwiftWinError::NotFound(format!(
            "no {PROJECT_FILE} in {} or its parents (create one with `swiftwin new <Name>`)",
            dir.display()
        )))
    }

    fn validate(&self) -> Result<()> {
        let a = &self.file.app;
        if a.name.trim().is_empty() {
            return Err(SwiftWinError::invalid("app.name is empty"));
        }
        let ok_id = !a.bundle_id.is_empty()
            && a.bundle_id.contains('.')
            && a.bundle_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
        if !ok_id {
            return Err(SwiftWinError::invalid(format!(
                "app.bundle_id {:?} must look like com.yourname.app (letters, digits, '.', '-')",
                a.bundle_id
            )));
        }
        if a.deployment_target.split('.').next().and_then(|m| m.parse::<u32>().ok()).is_none_or(|m| m < 15) {
            return Err(SwiftWinError::invalid("app.deployment_target must be iOS 15.0 or newer"));
        }
        if !a.deployment_target.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return Err(SwiftWinError::invalid("app.deployment_target must look like 17.0"));
        }
        // These folders are read by the build and uploaded by a cloud release: keep them inside the project.
        for (key, v) in [("sources", &a.sources), ("resources", &a.resources), ("assets", &a.assets)] {
            if !swiftwin_core::paths::is_safe_relative(Path::new(v)) {
                return Err(SwiftWinError::invalid(format!("app.{key} must be a folder inside the project, got {v:?}")));
            }
        }
        // `[info]` keys go into Info.plist and the cloud build's XcodeGen spec (YAML).
        if let Some(k) = self.file.info.keys().find(|k| k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))) {
            return Err(SwiftWinError::invalid(format!("[info] key {k:?} is not a valid Info.plist key")));
        }
        Ok(())
    }

    pub fn app(&self) -> &AppSection {
        &self.file.app
    }

    /// Executable / module name: the app name without characters Swift dislikes.
    pub fn executable(&self) -> String {
        let s: String = self.file.app.name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if s.is_empty() || s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            format!("App{s}")
        } else {
            s
        }
    }

    pub fn sources_dir(&self) -> PathBuf {
        self.dir.join(&self.file.app.sources)
    }
    pub fn resources_dir(&self) -> PathBuf {
        self.dir.join(&self.file.app.resources)
    }
    pub fn assets_dir(&self) -> PathBuf {
        self.dir.join(&self.file.app.assets)
    }
    pub fn build_dir(&self) -> PathBuf {
        self.dir.join("build")
    }

    /// All `.swift` files under the sources folder, sorted.
    pub fn swift_sources(&self) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        walk(&self.sources_dir(), &mut |p| {
            if p.extension().is_some_and(|e| e == "swift") {
                out.push(p.to_path_buf());
            }
        })?;
        out.sort();
        if out.is_empty() {
            return Err(SwiftWinError::invalid(format!("no .swift files in {}", self.sources_dir().display())));
        }
        Ok(out)
    }
}

/// Every regular file under `dir`, sorted. Symlinks and junctions are skipped, not followed: a
/// project (say, one downloaded from the internet) could otherwise point the file tree, the
/// build or the cloud release upload at files outside it, or loop forever.
pub fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.flatten().filter_map(|e| Some((e.path(), e.file_type().ok()?))).collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (p, t) in entries {
        if t.is_symlink() {
            continue;
        }
        if t.is_dir() {
            walk(&p, f)?;
        } else if t.is_file() {
            f(&p);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with(tag: &str, toml: &str) -> (PathBuf, Result<Project>) {
        let d = std::env::temp_dir().join(format!("swiftwin-project-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Sources")).unwrap();
        std::fs::write(d.join(PROJECT_FILE), toml).unwrap();
        let p = Project::load(&d);
        (d, p)
    }

    #[test]
    fn project_folders_and_keys_stay_inside() {
        let base = "[app]\nname = \"A\"\nbundle_id = \"com.example.a\"\n";
        let (d, p) = project_with("ok", &format!("{base}[info]\nNSCameraUsageDescription = \"x\"\n"));
        assert!(p.is_ok(), "{p:?}");
        std::fs::remove_dir_all(d).ok();
        for (tag, extra) in [
            ("src", "sources = \"../../Users\"\n"),
            ("abs", "resources = \"C:/Users/me/.ssh\"\n"),
            ("assets", "assets = \"/etc\"\n"),
            ("target", "deployment_target = \"17.0\\\"\\nevil: 1\"\n"),
            ("info", "[info]\n\"evil: yes\\n  x\" = \"1\"\n"),
            ("info-space", "[info]\n\"a b\" = \"1\"\n"),
        ] {
            let (d, p) = project_with(tag, &format!("{base}{extra}"));
            assert!(p.is_err(), "{tag} must be refused");
            std::fs::remove_dir_all(d).ok();
        }
    }

    #[test]
    fn walk_skips_links() {
        let d = std::env::temp_dir().join(format!("swiftwin-walk-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (root, outside) = (d.join("p"), d.join("outside"));
        std::fs::create_dir_all(root.join("Sources")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("Sources/a.swift"), "").unwrap();
        std::fs::write(outside.join("secret.txt"), "").unwrap();
        #[cfg(windows)]
        let made = std::process::Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(root.join("link")).arg(&outside).output().is_ok_and(|o| o.status.success());
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&outside, root.join("link")).is_ok();
        assert!(made, "could not create a test link");
        let mut seen = Vec::new();
        walk(&root, &mut |p| seen.push(p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"))).unwrap();
        assert_eq!(seen, ["Sources/a.swift"]);
        std::fs::remove_dir_all(&d).ok();
    }
}
