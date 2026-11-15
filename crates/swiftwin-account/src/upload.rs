//! Upload a project folder to GitHub: the repository's default branch is made to match the
//! folder in one new commit (earlier versions stay in its history). The first upload creates
//! the repository (private unless asked otherwise). Only runs when the user clicks Upload.
//!
//! What is never uploaded: anything a `.gitignore` excludes, build output, the iOS SDK,
//! signing material and other secrets (by name and by content), and files over 50 MB.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use swiftwin_core::{Result, SwiftWinError};
use swiftwin_release::GitHub;

/// GitHub rejects files over 100 MB; stay well under.
pub const MAX_FILE: u64 = 50 * 1024 * 1024;
pub const MAX_TOTAL: u64 = 500 * 1024 * 1024;

/// Folders never uploaded (build output, dependencies, tools, SDKs).
const SKIP_DIRS: &[&str] = &[".git", "build", ".build", "DerivedData", ".swiftwin", "node_modules", ".swiftpm", "xcuserdata", "Pods", "target"];
/// Folder suffixes never uploaded (bundles and SDKs).
const SKIP_DIR_SUFFIX: &[&str] = &[".app", ".xcarchive", ".dSYM", ".sdk", ".artifactbundle", ".framework", ".xcframework"];
/// File names / suffixes never uploaded: secrets, signing material, packages.
const SECRET_SUFFIX: &[&str] = &[".p8", ".p12", ".pfx", ".pem", ".key", ".mobileprovision", ".provisionprofile", ".cer", ".keystore", ".jks", ".der"];
const PACKAGE_SUFFIX: &[&str] = &[".ipa", ".xip", ".dmg", ".zip", ".tar", ".gz", ".7z"];
const SECRET_NAMES: &[&str] = &[".env", ".netrc", ".npmrc", "credentials.json", "id_rsa", "id_ed25519", "id_ecdsa", "AuthKey.p8", ".DS_Store", "Thumbs.db"];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct Collected {
    /// (repo path with `/`, file on disk), sorted.
    pub files: Vec<(String, PathBuf)>,
    pub skipped: Vec<Skipped>,
    pub total_bytes: u64,
}

fn lower_ends_with(name: &str, suffixes: &[&str]) -> bool {
    let n = name.to_ascii_lowercase();
    suffixes.iter().any(|s| n.ends_with(&s.to_ascii_lowercase()))
}

fn looks_secret(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    SECRET_NAMES.iter().any(|s| n == s.to_ascii_lowercase())
        || n.starts_with(".env.")
        || n.starts_with("id_rsa")
        || lower_ends_with(name, SECRET_SUFFIX)
}

/// True when the bytes contain a PEM private key (any kind).
fn has_private_key(data: &[u8]) -> bool {
    let hay = String::from_utf8_lossy(&data[..data.len().min(4 * 1024 * 1024)]);
    hay.contains("-----BEGIN") && hay.contains("PRIVATE KEY-----")
}

/// The files to upload from `dir`, honouring `.gitignore` files (and `.swiftwinignore`).
pub fn collect(dir: &Path) -> Result<Collected> {
    if !dir.is_dir() {
        return Err(SwiftWinError::NotFound(format!("project folder {}", dir.display())));
    }
    let mut out = Collected::default();
    let walker = ignore::WalkBuilder::new(dir)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(false)
        .git_global(false)
        .require_git(false)
        .parents(false)
        .follow_links(false)
        .add_custom_ignore_filename(".swiftwinignore")
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            let is_dir = e.file_type().is_some_and(|t| t.is_dir());
            !(is_dir && e.depth() > 0 && (SKIP_DIRS.iter().any(|d| d.eq_ignore_ascii_case(&name)) || lower_ends_with(&name, SKIP_DIR_SUFFIX)))
        })
        .build();
    for entry in walker {
        let entry = entry.map_err(|e| SwiftWinError::other(format!("reading the project: {e}")))?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue; // folders, and symlinks (never followed)
        }
        let p = entry.path();
        let rel = p.strip_prefix(dir).unwrap_or(p);
        let Some(rel) = rel.to_str().map(|r| r.replace('\\', "/")) else {
            out.skipped.push(Skipped { path: rel.to_string_lossy().into(), reason: "name is not valid Unicode".into() });
            continue;
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let skip = |reason: &str, out: &mut Collected| out.skipped.push(Skipped { path: rel.clone(), reason: reason.into() });
        if looks_secret(&name) {
            skip("secret or signing file", &mut out);
            continue;
        }
        if lower_ends_with(&name, PACKAGE_SUFFIX) {
            skip("package or archive", &mut out);
            continue;
        }
        let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if len > MAX_FILE {
            skip("larger than 50 MB", &mut out);
            continue;
        }
        out.total_bytes += len;
        out.files.push((rel, p.to_path_buf()));
    }
    out.files.sort();
    if out.total_bytes > MAX_TOTAL {
        return Err(SwiftWinError::invalid(format!(
            "the project is {} MB; SwiftWin uploads at most 500 MB (add big folders to .gitignore)",
            out.total_bytes / (1024 * 1024)
        )));
    }
    Ok(out)
}

/// Git's blob id: `sha1("blob <len>\0" + data)`.
pub fn blob_sha(data: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(format!("blob {}\0", data.len()).as_bytes());
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// A valid GitHub repository name from a project name.
pub fn repo_name(project: &str) -> String {
    let mut s: String = project.trim().chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '-' }).collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches(|c| c == '-' || c == '.').chars().take(100).collect::<String>();
    if s.is_empty() {
        "swiftwin-app".into()
    } else {
        s
    }
}

/// Which repository each project was uploaded to (`<data dir>/uploads.json`: folder → owner/name).
#[derive(Clone)]
pub struct Links {
    file: PathBuf,
}

impl Links {
    pub fn new(file: PathBuf) -> Links {
        Links { file }
    }
    fn key(dir: &Path) -> String {
        std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()).to_string_lossy().to_lowercase()
    }
    fn all(&self) -> BTreeMap<String, String> {
        std::fs::read(&self.file).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default()
    }
    pub fn get(&self, dir: &Path) -> Option<String> {
        self.all().remove(&Self::key(dir))
    }
    pub fn set(&self, dir: &Path, repo: Option<&str>) -> Result<()> {
        let mut all = self.all();
        match repo {
            Some(r) => all.insert(Self::key(dir), r.into()),
            None => all.remove(&Self::key(dir)),
        };
        swiftwin_core::paths::write_atomic(&self.file, &serde_json::to_vec_pretty(&all)?)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Repository name for the first upload (default: the folder name).
    pub name: Option<String>,
    /// Make a new repository public (default private).
    pub public: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Outcome {
    pub repo: String,
    pub url: String,
    pub created: bool,
    /// None when nothing changed.
    pub commit: Option<String>,
    pub files: usize,
    pub uploaded: usize,
    pub skipped: Vec<Skipped>,
}

/// Progress: (what is happening, done, total).
pub type Progress<'a> = &'a (dyn Fn(&str, usize, usize) + Sync);

// UNTESTED-ON-DEVICE: only run against the fake GitHub API so far.
/// Upload `dir` with `token` to the GitHub API at `api`.
pub fn upload(api: &str, token: &str, dir: &Path, opts: &Options, links: &Links, progress: Progress) -> Result<Outcome> {
    progress("Reading the project", 0, 0);
    let col = collect(dir)?;
    if col.files.is_empty() {
        return Err(SwiftWinError::invalid("there are no files to upload in this project"));
    }
    let account = GitHub::account(api, token);

    // 1. The repository: the one this folder went to before, or a new one.
    let mut created = false;
    let repo_info = match links.get(dir) {
        Some(full) => match account.send("GET", &format!("/repos/{full}"), None) {
            Ok(v) => v,
            Err(e) if GitHub::status_of(&e) == Some(404) => {
                links.set(dir, None)?;
                return Err(SwiftWinError::NotFound(format!(
                    "the repository {full} is gone (deleted, renamed, or no longer yours). Click Upload again to create a new one"
                )));
            }
            Err(e) => return Err(e),
        },
        None => {
            let default = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let name = repo_name(opts.name.as_deref().unwrap_or(&default));
            progress(&format!("Creating the repository {name}"), 0, 0);
            let v = account
                .send(
                    "POST",
                    "/user/repos",
                    Some(json!({"name": name, "private": !opts.public, "auto_init": true, "description": "Made with SwiftWin"})),
                )
                .map_err(|e| match &e {
                    SwiftWinError::InvalidInput(m) if m.contains("already exists") => SwiftWinError::invalid(format!(
                        "you already have a repository named {name} on GitHub. Pick another name"
                    )),
                    _ => e,
                })?;
            created = true;
            v
        }
    };
    let full = repo_info["full_name"].as_str().ok_or_else(|| SwiftWinError::protocol("GitHub repo has no full_name"))?.to_string();
    let url = repo_info["html_url"].as_str().unwrap_or("").to_string();
    let branch = repo_info["default_branch"].as_str().unwrap_or("main").to_string();
    if created {
        links.set(dir, Some(&full))?;
    }
    let gh = GitHub::with_base(api, &full, token)?;

    // 2. Where the branch is now (a brand-new repo can take a moment to show its first commit).
    let mut head = None;
    for attempt in 0..10 {
        match gh.call("GET", &format!("/git/ref/heads/{branch}"), None) {
            Ok(v) => {
                head = v["object"]["sha"].as_str().map(str::to_string);
                break;
            }
            Err(e) if created && attempt < 9 => {
                let _ = e;
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Err(e) if matches!(GitHub::status_of(&e), Some(404 | 409)) => break, // empty repository
            Err(e) => return Err(e),
        }
    }
    let mut remote: HashMap<String, String> = HashMap::new();
    if let Some(h) = &head {
        let c = gh.call("GET", &format!("/git/commits/{h}"), None)?;
        if let Some(t) = c["tree"]["sha"].as_str() {
            let tree = gh.call("GET", &format!("/git/trees/{t}?recursive=1"), None)?;
            for e in tree["tree"].as_array().into_iter().flatten() {
                if e["type"] == "blob" {
                    remote.insert(e["path"].as_str().unwrap_or("").into(), e["sha"].as_str().unwrap_or("").into());
                }
            }
        }
    }
    if head.is_none() {
        return Err(SwiftWinError::invalid(format!(
            "{full} has no {branch} branch yet. Add a README on GitHub, then upload again"
        )));
    }

    // 3. Upload what GitHub doesn't have yet.
    let known: HashSet<String> = remote.values().cloned().collect();
    let total = col.files.len();
    let mut entries = Vec::new();
    let mut uploaded = 0;
    let mut same = remote.len() == total;
    for (i, (path, file)) in col.files.iter().enumerate() {
        let data = std::fs::read(file)?;
        if has_private_key(&data) {
            return Err(SwiftWinError::invalid(format!(
                "{path} contains a private key. SwiftWin won't upload it: delete it or add it to .gitignore"
            )));
        }
        let sha = blob_sha(&data);
        if remote.get(path) != Some(&sha) {
            same = false;
        }
        if !known.contains(&sha) {
            progress(&format!("Uploading {path}"), i, total);
            let b = gh.call("POST", "/git/blobs", Some(json!({"content": swiftwin_core::base64_bytes::encode(&data), "encoding": "base64"})))?;
            if b["sha"].as_str() != Some(sha.as_str()) {
                return Err(SwiftWinError::protocol(format!("GitHub stored {path} with an unexpected id")));
            }
            uploaded += 1;
        }
        entries.push(json!({"path": path, "mode": "100644", "type": "blob", "sha": sha}));
    }
    let outcome = |commit| Outcome { repo: full.clone(), url: url.clone(), created, commit, files: total, uploaded, skipped: col.skipped.clone() };
    if same {
        progress("Already up to date", total, total);
        return Ok(outcome(None));
    }

    // 4. One commit with the whole folder, then move the branch (never forced).
    progress("Saving the commit", total, total);
    let t = gh.call("POST", "/git/trees", Some(json!({"tree": entries})))?;
    let tree = t["sha"].as_str().ok_or_else(|| SwiftWinError::protocol("tree has no sha"))?;
    let message = opts.message.clone().filter(|m| !m.trim().is_empty()).unwrap_or_else(|| "Upload from SwiftWin".into());
    let c = gh.call("POST", "/git/commits", Some(json!({"message": message, "tree": tree, "parents": [head]})))?;
    let sha = c["sha"].as_str().ok_or_else(|| SwiftWinError::protocol("commit has no sha"))?.to_string();
    gh.call("PATCH", &format!("/git/refs/heads/{branch}"), Some(json!({"sha": sha, "force": false}))).map_err(|e| match &e {
        SwiftWinError::InvalidInput(m) if m.to_lowercase().contains("fast forward") => SwiftWinError::invalid(format!(
            "{full} changed on GitHub while uploading. Upload again"
        )),
        _ => e,
    })?;
    progress("Uploaded", total, total);
    Ok(outcome(Some(sha)))
}

/// For the UI: what an upload would include, without any network call.
pub fn preview(dir: &Path, links: &Links) -> Result<Value> {
    let col = collect(dir)?;
    Ok(json!({
        "repo": links.get(dir),
        "default_name": repo_name(&dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
        "files": col.files.len(),
        "bytes": col.total_bytes,
        "skipped": col.skipped,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_blob_ids_and_names() {
        // `git hash-object` of "hello\n"
        assert_eq!(blob_sha(b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a");
        assert_eq!(repo_name("My Cool App!"), "My-Cool-App");
        assert_eq!(repo_name("  "), "swiftwin-app");
        assert_eq!(repo_name("a/b\\c"), "a-b-c");
    }

    #[test]
    fn collect_skips_build_secrets_and_gitignored() {
        let d = std::env::temp_dir().join(format!("swiftwin-upload-{}", rand::random::<u32>()));
        let w = |p: &str, s: &str| {
            let f = d.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, s).unwrap();
        };
        w("Sources/App.swift", "import SwiftUI");
        w("swiftwin.toml", "name = 'x'");
        w("build/Hello.app/Hello", "bin");
        w("AuthKey_ABC.p8", "-----BEGIN PRIVATE KEY-----");
        w(".env", "TOKEN=1");
        w("notes/secret.txt", "x");
        w(".gitignore", "notes/\n");
        w("Foo.xcarchive/Info.plist", "x");
        let c = collect(&d).unwrap();
        let names: Vec<&str> = c.files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, vec![".gitignore", "Sources/App.swift", "swiftwin.toml"]);
        assert!(c.skipped.iter().any(|s| s.path == "AuthKey_ABC.p8"));
        assert!(c.skipped.iter().any(|s| s.path == ".env"));
        assert!(has_private_key(b"x\n-----BEGIN EC PRIVATE KEY-----\n"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
