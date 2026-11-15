//! iOS SDK management.
//!
//! SwiftWin never bundles or redistributes Apple's SDK. The user downloads
//! `Xcode_<ver>.xip` with their own Apple ID (in the desktop app's sign-in
//! window, or with a browser), and SwiftWin extracts **only** what it needs into
//! `<sdk dir>/iPhoneOS<ver>/`:
//!
//! * `SDKs/iPhoneOS*.sdk` (headers, .tbd stubs, Swift interfaces), from
//!   `Platforms/iPhoneOS.platform/Developer/SDKs`. Per Apple DTS, the textual SDKs of every
//!   platform are in the main .xip; the separate "Platform Support" download is the simulator
//!   and Xcode's build support.
//! * `Library/Frameworks`, `usr/lib` from the same `Developer/` folder (XCTest and its Swift
//!   support). WebDriverAgent does not need them: appium's runner zip leaves out `Frameworks/`
//!   on purpose so XCTest loads from the phone's developer disk image.
//! * `Toolchain/usr/lib/swift/iphoneos/*.{a,dylib}`, from `XcodeDefault.xctoolchain/usr/lib/swift/iphoneos`:
//!   Swift back-deployment libraries (none is linked for a deployment target of iOS 17 or later).
//! * `Toolchain/usr/lib/clang/darwin/libclang_rt.ios.a`, from
//!   `XcodeDefault.xctoolchain/usr/lib/clang/<version>/lib/darwin`: compiler-rt builtins such as
//!   `__isPlatformVersionAtLeast`. clang links it only if it exists, and the Windows toolchain has none.
//!
//! It then writes a Swift SDK artifact bundle (`swift sdk install`-compatible)
//! next to it. Pipeline: XAR (`xar.rs`) → pbzx/XZ (`pbzx.rs`) → cpio odc (`cpio.rs`).

pub mod cpio;
pub mod download;
pub mod pbzx;
pub mod swiftsdk;
pub mod xar;

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use swiftwin_core::{Result, SwiftWinError};

/// An extracted iOS SDK on disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SdkInfo {
    /// e.g. "iPhoneOS26.1"
    pub name: String,
    /// e.g. "26.1"
    pub version: String,
    /// Root folder holding `SDKs`, `Library/Frameworks`, `usr/lib` and `Toolchain`.
    pub root: PathBuf,
    /// Path to the `.sdk` directory (pass to `swiftc -sdk`).
    pub sdk_path: PathBuf,
    /// Path to the Swift SDK artifact bundle.
    pub artifact_bundle: PathBuf,
    /// Swift back-deployment libraries from Xcode's toolchain (a linker `-L` folder), if present.
    #[serde(default)]
    pub swift_lib_dir: Option<PathBuf>,
    /// `libclang_rt.ios.a` from Xcode's toolchain, if present: give it to the linker.
    #[serde(default)]
    pub clang_rt: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub phase: String,
    pub files: u64,
    pub bytes: u64,
}

const PLATFORM: &str = "Contents/Developer/Platforms/iPhoneOS.platform/Developer/";
const TOOLCHAIN: &str = "Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/";

/// `<root>/Toolchain/usr/lib/swift/iphoneos`
pub fn swift_lib_dir(root: &Path) -> PathBuf {
    root.join("Toolchain").join("usr").join("lib").join("swift").join("iphoneos")
}

/// `<root>/Toolchain/usr/lib/clang/darwin/libclang_rt.ios.a` (no clang version in the path; where
/// `swiftwin_builder::compile::xcode_toolchain_lib` looks for it).
pub fn clang_rt_path(root: &Path) -> PathBuf {
    root.join("Toolchain").join("usr").join("lib").join("clang").join("darwin").join("libclang_rt.ios.a")
}

/// Where an archive entry goes under the SDK root as a `/`-separated path, or `None` when
/// SwiftWin does not need it. Not yet checked for `..` or names Windows cannot store.
fn wanted(archive_name: &str) -> Option<String> {
    // "Xcode.app/" or "Xcode-beta.app/" or "Xcode_26.3.app/" prefix.
    let (_, rest) = archive_name.split_once(".app/")?;
    if let Some(p) = rest.strip_prefix(PLATFORM) {
        let keep = p.starts_with("SDKs/")
            || p.starts_with("Library/Frameworks/")
            || p.starts_with("usr/lib/")
            || p == "SDKs"
            || p == "Library/Frameworks";
        return keep.then(|| p.to_string());
    }
    let t = rest.strip_prefix(TOOLCHAIN)?;
    if let Some(f) = t.strip_prefix("swift/iphoneos/") {
        // The back-deployment libraries, not prebuilt-modules/ (made by Apple's compiler).
        let lib = !f.contains('/') && (f.ends_with(".a") || f.ends_with(".dylib"));
        return lib.then(|| format!("Toolchain/usr/lib/swift/iphoneos/{f}"));
    }
    // clang/<version>/lib/darwin/libclang_rt.ios.a, stored without the version.
    let (_, rest) = t.strip_prefix("clang/")?.split_once('/')?;
    (rest == "lib/darwin/libclang_rt.ios.a").then(|| "Toolchain/usr/lib/clang/darwin/libclang_rt.ios.a".to_string())
}

/// Map an archive path to where it goes under the SDK root, or `None` to skip:
/// `SDKs/...`, `Library/Frameworks/...`, `usr/lib/...`, `Toolchain/usr/lib/...`.
pub fn select_path(archive_name: &str) -> Option<PathBuf> {
    cpio::sanitize(&wanted(archive_name)?)
}

/// The cpio stream inside a `.xip`: XAR `Content` entry → pbzx.
fn open_content(xip: &Path) -> Result<pbzx::PbzxReader<Box<dyn Read>>> {
    let f = File::open(xip).map_err(|e| SwiftWinError::invalid(format!("cannot open {}: {e}", xip.display())))?;
    let x = xar::Xar::open(BufReader::with_capacity(1 << 20, f))?;
    let content = x
        .entry("Content")
        .cloned()
        .ok_or_else(|| SwiftWinError::invalid("this .xip has no Content entry"))?;
    let raw = x.into_raw_reader(&content)?;
    // Apple stores Content uncompressed; XAR's "application/x-gzip" encoding is zlib.
    let raw: Box<dyn Read> = if content.encoding.contains("gzip") {
        Box::new(flate2::read::ZlibDecoder::new(raw))
    } else {
        Box::new(raw)
    };
    pbzx::PbzxReader::new(raw)
}

/// Rename `from` to `to`, retrying briefly (on Windows a virus scanner may still hold the new
/// files open), else copy it.
fn move_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    for attempt in 0..5u32 {
        if std::fs::rename(from, to).is_ok() {
            return Ok(());
        }
        if attempt < 4 {
            std::thread::sleep(std::time::Duration::from_millis(200 << attempt));
        }
    }
    cpio::copy_any(from, to)?;
    let _ = std::fs::remove_dir_all(from); // a leftover .staging is removed by the next import
    Ok(())
}

/// Extract the iOS SDK from `xip` into `sdk_root/<name>` and package it.
pub fn import_xip(xip: &Path, sdk_root: &Path, progress: &mut dyn FnMut(&Progress)) -> Result<SdkInfo> {
    let content = open_content(xip)?;
    let staging = sdk_root.join(".staging");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    progress(&Progress { phase: "Extracting iOS SDK from Xcode archive".into(), ..Default::default() });
    let mut unsafe_names: Vec<String> = Vec::new();
    let mut ex = cpio::extract(
        content,
        |name| {
            let rel = wanted(name)?;
            let p = cpio::sanitize(&rel);
            if p.is_none() {
                unsafe_names.push(rel);
            }
            Some(staging.join(p?))
        },
        |s| progress(&Progress { phase: "Extracting".into(), files: s.files, bytes: s.bytes }),
    )?;
    if ex.stats.files == 0 {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(SwiftWinError::invalid("no iPhoneOS SDK found inside this .xip (is it really Xcode?)"));
    }
    if !ex.missing.is_empty() {
        progress(&Progress {
            phase: format!("Second pass: {} hard-linked files keep their data elsewhere in the archive", ex.missing.len()),
            ..Default::default()
        });
        cpio::fill_missing(open_content(xip)?, &mut ex.missing)?;
        if !ex.missing.is_empty() {
            progress(&Progress {
                phase: format!("warning: {} hard-linked files had no data in the archive and are empty", ex.missing.len()),
                ..Default::default()
            });
        }
    }
    if !unsafe_names.is_empty() {
        let shown: Vec<&str> = unsafe_names.iter().take(5).map(String::as_str).collect();
        progress(&Progress {
            phase: format!(
                "warning: skipped {} file(s) whose names are unsafe or invalid on Windows: {}",
                unsafe_names.len(),
                shown.join(", ")
            ),
            ..Default::default()
        });
    }
    // Links don't exist yet, so this finds the real folder: in Xcode that is `iPhoneOS.sdk`,
    // and `iPhoneOS<version>.sdk` is a symlink to it.
    let (sdk_dir_name, version) = find_sdk(&staging.join("SDKs"))?;
    let name = format!("iPhoneOS{version}");
    let root = sdk_root.join(&name);
    let _ = std::fs::remove_dir_all(&root);
    move_dir(&staging, &root)?;
    // Links are made in the final folder: an NTFS junction stores an absolute target.
    let links = ex
        .links
        .into_iter()
        .filter_map(|l| Some(cpio::PendingLink { at: root.join(l.at.strip_prefix(&staging).ok()?), target: l.target }))
        .collect();
    let unresolved = cpio::materialize_links(links, &root)?;
    if !unresolved.is_empty() {
        progress(&Progress { phase: format!("{} symlinks could not be resolved", unresolved.len()), ..Default::default() });
    }
    let sdk_path = root.join("SDKs").join(&sdk_dir_name);
    let lib_dir = Some(swift_lib_dir(&root)).filter(|p| p.is_dir());
    let clang_rt = Some(clang_rt_path(&root)).filter(|p| p.is_file());
    if clang_rt.is_none() {
        progress(&Progress {
            phase: "note: no libclang_rt.ios.a in this Xcode; code using `#available` may not link".into(),
            ..Default::default()
        });
    }
    let artifact_bundle = swiftsdk::write_bundle(&root, &sdk_path, &version)?;
    let info = SdkInfo { name, version, root, sdk_path, artifact_bundle, swift_lib_dir: lib_dir, clang_rt };
    std::fs::write(info.root.join("swiftwin-sdk.json"), serde_json::to_vec_pretty(&info)?)?;
    progress(&Progress { phase: "Done".into(), files: ex.stats.files, bytes: ex.stats.bytes });
    Ok(info)
}

/// Find the `iPhoneOS*.sdk` folder and its version from SDKSettings.json. Called before any
/// symlink exists, so only real folders are seen.
fn find_sdk(sdks: &Path) -> Result<(String, String)> {
    let mut best: Option<(String, String)> = None;
    for e in std::fs::read_dir(sdks).map_err(|_| SwiftWinError::invalid("no SDKs folder extracted"))? {
        let name = e?.file_name().to_string_lossy().into_owned();
        if !name.starts_with("iPhoneOS") || !name.ends_with(".sdk") {
            continue;
        }
        let settings = sdks.join(&name).join("SDKSettings.json");
        let version = std::fs::read(&settings)
            .ok()
            .and_then(|d| serde_json::from_slice::<serde_json::Value>(&d).ok())
            .and_then(|v| v["Version"].as_str().map(str::to_string))
            .or_else(|| name.strip_prefix("iPhoneOS")?.strip_suffix(".sdk").filter(|v| !v.is_empty()).map(str::to_string));
        if let Some(v) = version {
            // If both are folders, prefer the versioned name (iPhoneOS26.1.sdk) over iPhoneOS.sdk.
            if best.as_ref().is_none_or(|(n, _)| n == "iPhoneOS.sdk") {
                best = Some((name, v));
            }
        }
    }
    best.ok_or_else(|| SwiftWinError::invalid("could not find iPhoneOS*.sdk with SDKSettings.json"))
}

/// SDKs previously imported into `sdk_root`.
pub fn installed(sdk_root: &Path) -> Vec<SdkInfo> {
    let mut v: Vec<SdkInfo> = std::fs::read_dir(sdk_root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::read(e.path().join("swiftwin-sdk.json")).ok())
        .filter_map(|d| serde_json::from_slice(&d).ok())
        .collect();
    v.sort_by_key(|s: &SdkInfo| std::cmp::Reverse(version_key(&s.version)));
    v
}

fn version_key(v: &str) -> Vec<u32> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// The SDK to build with: the configured one, else the newest installed.
pub fn active(sdk_root: &Path, preferred: Option<&str>) -> Result<SdkInfo> {
    let all = installed(sdk_root);
    if let Some(p) = preferred {
        if let Some(s) = all.iter().find(|s| s.name == p || s.version == p) {
            return Ok(s.clone());
        }
    }
    all.into_iter().next().ok_or_else(|| {
        SwiftWinError::NotFound(
            "no iOS SDK imported yet: download Xcode with your Apple ID and run `swiftwin sdk import <Xcode.xip>`".into(),
        )
    })
}

#[cfg(test)]
pub(crate) mod testxip {
    //! Builds a tiny but structurally real `.xip` for tests.
    use super::cpio::build::{cpio, E};

    /// Laid out like Xcode: `iPhoneOS.sdk` is the folder and `iPhoneOS26.1.sdk` a symlink to
    /// it (the other way round with `versioned_folder`).
    pub fn xip_with(versioned_folder: bool) -> Vec<u8> {
        let p = "./Xcode.app/Contents/Developer/Platforms/iPhoneOS.platform/Developer";
        let t = "./Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib";
        let (folder, alias) = if versioned_folder { ("iPhoneOS26.1.sdk", "iPhoneOS.sdk") } else { ("iPhoneOS.sdk", "iPhoneOS26.1.sdk") };
        let sdk = format!("{p}/SDKs/{folder}");
        let n = [
            format!("{p}/SDKs"),
            sdk.clone(),
            format!("{sdk}/SDKSettings.json"),
            format!("{sdk}/System/Library/Frameworks/SwiftUI.framework/SwiftUI.tbd"),
            format!("{sdk}/usr/lib/swift/Swift.swiftmodule/arm64-apple-ios.swiftinterface"),
            format!("{p}/SDKs/{alias}"),
            format!("{p}/Library/Frameworks/XCTest.framework/XCTest"),
            format!("{p}/usr/lib/libXCTestSwiftSupport.dylib"),
            "./Xcode.app/Contents/MacOS/Xcode".to_string(),
            "./Xcode.app/Contents/Developer/Platforms/AppleTVOS.platform/Developer/SDKs/AppleTVOS.sdk/usr/include/shared.h".to_string(),
            format!("{sdk}/usr/include/shared.h"),
            format!("{sdk}/usr/include/aux.h"),
            format!("{sdk}/usr/lib/libz.1.tbd"),
            format!("{sdk}/usr/lib/libz.tbd"),
            format!("{t}/swift/iphoneos/libswiftCompatibility56.a"),
            format!("{t}/swift/iphoneos/prebuilt-modules/26.1/Foo.swiftmodule"),
            format!("{t}/swift/macosx/libswiftCompatibility56.a"),
            format!("{t}/clang/17/lib/darwin/libclang_rt.ios.a"),
            format!("{t}/clang/17/lib/darwin/libclang_rt.osx.a"),
        ];
        let big: Vec<u8> = (0..200_000u32).map(|i| (i % 7) as u8).collect();
        let arc = cpio(&[
            E::Dir(&n[0]),
            E::Dir(&n[1]),
            E::File(&n[2], br#"{"Version":"26.1","CanonicalName":"iphoneos26.1"}"#),
            E::File(&n[3], b"--- !tapi-tbd\ninstall-name: SwiftUI\n"),
            E::File(&n[4], &big),
            E::Link(&n[5], folder),
            E::File(&n[6], b"xctest-binary"),
            E::File(&n[7], b"dylib"),
            E::File(&n[8], b"mac app binary"),
            // A header shared with another platform: its data comes with the skipped copy.
            E::Hard(&n[9], 42, b"#define SHARED 1\n"),
            E::Hard(&n[10], 42, b""),
            E::File(&n[11], b"a device name on Windows"),
            E::File(&n[12], b"--- !tapi-tbd\ninstall-name: /usr/lib/libz.1.dylib\n"),
            E::Link(&n[13], "libz.1.tbd"),
            E::File(&n[14], b"!<arch>\n"),
            E::File(&n[15], b"prebuilt"),
            E::File(&n[16], b"!<arch>\n"),
            E::File(&n[17], b"!<arch>\nclang_rt"),
            E::File(&n[18], b"!<arch>\n"),
        ]);
        let content = super::pbzx::build::pbzx(&arc, 64 * 1024);
        super::xar::build::xar(&[("Metadata", b"<plist/>"), ("Content", &content)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_only_what_swiftwin_needs() {
        let p = "Xcode.app/Contents/Developer/Platforms/iPhoneOS.platform/Developer/";
        let t = "./Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/";
        assert_eq!(select_path(&format!("./{p}SDKs/iPhoneOS.sdk/x.h")), Some(PathBuf::from("SDKs/iPhoneOS.sdk/x.h")));
        assert!(select_path(&format!("{p}Library/Frameworks/XCTest.framework/XCTest")).is_some());
        assert!(select_path("./Xcode-beta.app/Contents/Developer/Platforms/iPhoneOS.platform/Developer/SDKs/a").is_some());
        assert!(select_path("./Xcode.app/Contents/MacOS/Xcode").is_none());
        assert!(select_path(&format!("{p}Library/Xcode/Templates/x")).is_none());
        assert!(select_path("./Xcode.app/Contents/Developer/Platforms/iPhoneOS.platform/Developer/../../../evil").is_none());
        assert!(select_path(&format!("./{p}SDKs/../../../../evil")).is_none());
        assert!(select_path(&format!("./{p}SDKs/iPhoneOS.sdk/usr/include/aux.h")).is_none());
        assert_eq!(
            select_path(&format!("{t}clang/17/lib/darwin/libclang_rt.ios.a")),
            Some(PathBuf::from("Toolchain/usr/lib/clang/darwin/libclang_rt.ios.a"))
        );
        assert!(select_path(&format!("{t}clang/17/lib/darwin/libclang_rt.osx.a")).is_none());
        assert_eq!(
            select_path(&format!("{t}swift/iphoneos/libswiftCompatibilityPacks.a")),
            Some(PathBuf::from("Toolchain/usr/lib/swift/iphoneos/libswiftCompatibilityPacks.a"))
        );
        assert!(select_path(&format!("{t}swift/iphoneos/prebuilt-modules/26.2/Swift.swiftmodule")).is_none());
        assert!(select_path(&format!("{t}swift/iphonesimulator/libswiftCompatibility56.a")).is_none());
    }

    fn import(tag: &str, versioned_folder: bool) -> (PathBuf, SdkInfo, Vec<String>) {
        let dir = std::env::temp_dir().join(format!("swiftwin-sdk-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let xip = dir.join("Xcode_26.1.xip");
        std::fs::write(&xip, testxip::xip_with(versioned_folder)).unwrap();
        let mut phases = vec![];
        let info = import_xip(&xip, &dir.join("sdk"), &mut |p| phases.push(p.phase.clone())).unwrap();
        (dir, info, phases)
    }

    #[test]
    fn imports_synthetic_xip() {
        let (dir, info, phases) = import("apple", false);
        assert_eq!(info.name, "iPhoneOS26.1");
        assert_eq!(info.version, "26.1");
        assert!(info.sdk_path.ends_with("iPhoneOS.sdk"), "the real folder, not the alias");
        let sdk = &info.sdk_path;
        assert!(sdk.join("System/Library/Frameworks/SwiftUI.framework/SwiftUI.tbd").exists());
        assert_eq!(
            std::fs::metadata(sdk.join("usr/lib/swift/Swift.swiftmodule/arm64-apple-ios.swiftinterface")).unwrap().len(),
            200_000
        );
        // The versioned alias and the libz.tbd symlink are materialised (as links or copies).
        assert!(info.root.join("SDKs/iPhoneOS26.1.sdk/SDKSettings.json").exists());
        assert!(std::fs::read_to_string(sdk.join("usr/lib/libz.tbd")).unwrap().contains("libz.1.dylib"));
        // A hard link whose data sat in a skipped platform is filled by the second pass.
        assert_eq!(std::fs::read_to_string(sdk.join("usr/include/shared.h")).unwrap(), "#define SHARED 1\n");
        assert!(phases.iter().any(|p| p.starts_with("Second pass")));
        // A name Windows cannot store is skipped with a warning instead of failing the import.
        let headers: Vec<String> = std::fs::read_dir(sdk.join("usr/include"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(headers, vec!["shared.h".to_string()]);
        assert!(phases.iter().any(|p| p.contains("aux.h")));
        assert!(info.root.join("Library/Frameworks/XCTest.framework/XCTest").exists());
        assert_eq!(info.clang_rt.as_deref(), Some(clang_rt_path(&info.root).as_path()));
        assert_eq!(std::fs::read(clang_rt_path(&info.root)).unwrap(), b"!<arch>\nclang_rt");
        let lib = info.swift_lib_dir.clone().unwrap();
        assert!(lib.join("libswiftCompatibility56.a").exists());
        assert!(!lib.join("prebuilt-modules").exists());
        assert!(!info.root.join("MacOS").exists());
        assert!(!info.root.join("Toolchain/usr/lib/swift/macosx").exists());
        assert!(info.artifact_bundle.join("info.json").exists());
        let root = dir.join("sdk");
        assert!(!root.join(".staging").exists());
        assert_eq!(installed(&root), vec![info.clone()]);
        assert_eq!(active(&root, None).unwrap(), info);
        assert!(phases.iter().any(|p| p == "Done"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn imports_sdk_whose_versioned_name_is_the_folder() {
        let (dir, info, _) = import("versioned", true);
        assert!(info.sdk_path.ends_with("iPhoneOS26.1.sdk"));
        assert!(info.root.join("SDKs/iPhoneOS.sdk/SDKSettings.json").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn old_sdk_json_without_toolchain_fields_still_loads() {
        let json = r#"{"name":"iPhoneOS26.1","version":"26.1","root":"r","sdk_path":"s","artifact_bundle":"a"}"#;
        let info: SdkInfo = serde_json::from_str(json).unwrap();
        assert!(info.swift_lib_dir.is_none() && info.clang_rt.is_none());
    }

    #[test]
    fn no_sdk_means_helpful_error() {
        let e = active(Path::new("/nonexistent-swiftwin"), None).unwrap_err().to_string();
        assert!(e.contains("swiftwin sdk import"));
    }
}
