//! Builds SwiftUI apps into `.app` bundles and `.ipa` files without a Mac.
//!
//! Steps: load `swiftwin.toml` → compile the asset catalog → run swiftc for
//! `arm64-apple-ios<target>` against the extracted iOS SDK → write
//! Info.plist/PkgInfo/resources/icons → (sign, see `swiftwin-signer`) → zip to `.ipa`.

pub mod assets;
pub mod compile;
pub mod infoplist;
pub mod macho;
pub mod project;
pub mod scaffold;

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use swiftwin_core::{Result, SwiftWinError};

pub use compile::{Compiler, FakeCompiler, Swiftc};
pub use project::Project;

#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub release: bool,
    /// The `.sdk` directory.
    pub sdk_path: PathBuf,
    /// SDK version string, e.g. "26.1".
    pub sdk_version: String,
    /// Clang/Swift module cache; `None` = `<project>/build/ModuleCache`. Real builds share
    /// [`shared_module_cache`]: a fresh cache costs minutes (SwiftUI, UIKit… are compiled into it).
    pub module_cache: Option<PathBuf>,
}

/// One module cache per SDK for all projects. Per-project caches made every new project's
/// first build take ~5 minutes instead of ~40 s (seen with the Tempo app, 2026-11-11).
pub fn shared_module_cache(sdk_version: &str) -> PathBuf {
    swiftwin_core::paths::data_dir().join("ModuleCache").join(sdk_version)
}

/// True when `dir` holds no compiled modules yet, so the next build will be slow.
pub fn module_cache_is_cold(dir: &Path) -> bool {
    std::fs::read_dir(dir).map(|mut d| d.next().is_none()).unwrap_or(true)
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildOutput {
    pub app: PathBuf,
    pub bundle_id: String,
    pub executable: String,
    pub warnings: Vec<String>,
    pub compiler_log: String,
}

/// Compile and assemble `<project>/build/<config>/<Name>.app` (unsigned).
pub fn build(project: &Project, opts: &BuildOptions, compiler: &dyn Compiler) -> Result<BuildOutput> {
    let config = if opts.release { "release" } else { "debug" };
    let out_dir = project.build_dir().join(config);
    let exe = project.executable();
    let app = out_dir.join(format!("{exe}.app"));
    let _ = std::fs::remove_dir_all(&app);
    std::fs::create_dir_all(&app)?;

    let compiled = assets::compile(&project.assets_dir())?;
    let mut sources = project.swift_sources()?;
    if let Some(swift) = &compiled.swift {
        let gen = out_dir.join("generated").join("SwiftWinAssets.swift");
        std::fs::create_dir_all(gen.parent().unwrap())?;
        std::fs::write(&gen, swift)?;
        sources.push(gen);
    }

    let job = compile::CompileJob {
        module_name: exe.clone(),
        sources,
        sdk: opts.sdk_path.clone(),
        sdk_version: opts.sdk_version.clone(),
        target: format!("arm64-apple-ios{}", project.app().deployment_target),
        output: app.join(&exe),
        work_dir: out_dir.join("obj"),
        module_cache: opts.module_cache.clone().unwrap_or_else(|| project.build_dir().join("ModuleCache")),
        release: opts.release,
        extra_args: vec![],
    };
    let compiler_log = compiler.compile(&job)?;
    if !job.output.exists() {
        return Err(SwiftWinError::tool("swiftc", "the compiler reported success but produced no executable"));
    }

    let info = infoplist::generate(&infoplist::InfoInputs {
        project,
        sdk_version: &opts.sdk_version,
        icon_files: compiled.icon_files.clone(),
    })?;
    plist::Value::Dictionary(info)
        .to_file_xml(app.join("Info.plist"))
        .map_err(|e| SwiftWinError::other(format!("writing Info.plist: {e}")))?;
    std::fs::write(app.join("PkgInfo"), b"APPL????")?;
    for (src, name) in &compiled.copies {
        std::fs::copy(src, app.join(name))?;
    }
    for (data, name) in &compiled.generated {
        std::fs::write(app.join(name), data)?;
    }
    let res = project.resources_dir();
    if res.exists() {
        for e in std::fs::read_dir(&res)? {
            let e = e?;
            copy_tree(&e.path(), &app.join(e.file_name()))?;
        }
    }
    Ok(BuildOutput {
        app,
        bundle_id: project.app().bundle_id.clone(),
        executable: exe,
        warnings: compiled.warnings,
        compiler_log,
    })
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            copy_tree(&e.path(), &to.join(e.file_name()))?;
        }
    } else {
        std::fs::copy(from, to)?;
    }
    Ok(())
}

/// Zip `X.app` into `Payload/X.app` → `.ipa`. Mach-O files keep mode 0755.
pub fn package_ipa(app: &Path, ipa: &Path) -> Result<PathBuf> {
    zip_app(app, ipa, |_| false)
}

/// Like [`package_ipa`], for an `.ipa` another tool signs (e.g. Sideloadly): leaves
/// out any earlier bundle signature, so only the re-signer's ends up in the bundle.
pub fn package_unsigned_ipa(app: &Path, ipa: &Path) -> Result<PathBuf> {
    zip_app(app, ipa, is_signing_artifact)
}

/// Files a code signature adds to a bundle (nested bundles included), by path
/// relative to the `.app`.
pub fn is_signing_artifact(rel: &str) -> bool {
    rel.split('/').any(|c| c == "_CodeSignature") || rel.rsplit('/').next() == Some("embedded.mobileprovision")
}

fn zip_app(app: &Path, ipa: &Path, skip: fn(&str) -> bool) -> Result<PathBuf> {
    let name = app.file_name().ok_or_else(|| SwiftWinError::invalid("bad .app path"))?.to_string_lossy().into_owned();
    if let Some(p) = ipa.parent() {
        std::fs::create_dir_all(p)?;
    }
    let mut z = zip::ZipWriter::new(std::fs::File::create(ipa)?);
    let mut files = Vec::new();
    project::walk(app, &mut |p| files.push(p.to_path_buf()))?;
    let zerr = |e: zip::result::ZipError| SwiftWinError::other(format!("zip: {e}"));
    let rels: Vec<(PathBuf, String)> = files
        .into_iter()
        .map(|f| {
            let rel = f.strip_prefix(app).unwrap().to_string_lossy().replace('\\', "/");
            (f, rel)
        })
        .filter(|(_, rel)| !skip(rel))
        .collect();
    // Directory entries first, like `zip -r Payload` writes them.
    let mut dirs = std::collections::BTreeSet::from(["Payload/".to_string(), format!("Payload/{name}/")]);
    for (_, rel) in &rels {
        let mut acc = format!("Payload/{name}/");
        for part in rel.split('/').rev().skip(1).collect::<Vec<_>>().into_iter().rev() {
            acc.push_str(part);
            acc.push('/');
            dirs.insert(acc.clone());
        }
    }
    for d in dirs {
        // zip 9 records the current OS as the entry's system unless told otherwise, and readers
        // (iOS included) ignore Unix modes on DOS entries: the app's executable bit would be lost.
        z.add_directory(d, zip::write::SimpleFileOptions::default().system(zip::System::Unix).unix_permissions(0o755)).map_err(zerr)?;
    }
    for (f, rel) in rels {
        let data = std::fs::read(&f)?;
        let mode = if is_macho(&data) { 0o755 } else { 0o644 };
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .system(zip::System::Unix)
            .unix_permissions(mode);
        z.start_file(format!("Payload/{name}/{rel}"), opts).map_err(zerr)?;
        z.write_all(&data)?;
    }
    z.finish().map_err(zerr)?;
    Ok(ipa.to_path_buf())
}

pub fn is_macho(data: &[u8]) -> bool {
    data.len() >= 4
        && matches!(
            u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            0xfeedfacf | 0xfeedface | 0xcafebabe | 0xbebafeca
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("swiftwin-build-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn opts() -> BuildOptions {
        BuildOptions { release: false, sdk_path: "mock.sdk".into(), sdk_version: "26.1".into(), module_cache: None }
    }

    #[test]
    fn new_build_and_package() {
        let d = tmp("hello");
        let dir = scaffold::new_project(&d, "Hello", Some("com.example.hello")).unwrap();
        std::fs::create_dir_all(dir.join("Resources")).unwrap();
        std::fs::write(dir.join("Resources/data.json"), b"{}").unwrap();
        let p = Project::load(&dir.join("Sources")).unwrap(); // found from a subfolder
        let out = build(&p, &opts(), &FakeCompiler).unwrap();
        assert!(out.app.ends_with("build/debug/Hello.app"));
        let exe = std::fs::read(out.app.join("Hello")).unwrap();
        assert!(is_macho(&exe));
        let info: plist::Dictionary = plist::from_file(out.app.join("Info.plist")).unwrap();
        assert_eq!(info["CFBundleIdentifier"].as_string(), Some("com.example.hello"));
        assert_eq!(info["CFBundleExecutable"].as_string(), Some("Hello"));
        assert_eq!(info["MinimumOSVersion"].as_string(), Some("17.0"));
        assert_eq!(info["DTSDKName"].as_string(), Some("iphoneos26.1"));
        let icons = info["CFBundleIcons"].as_dictionary().unwrap()["CFBundlePrimaryIcon"].as_dictionary().unwrap();
        assert_eq!(icons["CFBundleIconFiles"].as_array().unwrap()[0].as_string(), Some("AppIcon60x60"));
        assert!(icons.get("CFBundleIconName").is_none(), "no Assets.car to name an icon in");
        let icon = std::fs::read(out.app.join("AppIcon60x60@3x.png")).unwrap();
        assert_eq!(swiftwin_core::png_dimensions(&icon), Some((180, 180)));
        assert!(out.app.join("data.json").exists());
        let gen = std::fs::read_to_string(p.build_dir().join("debug/generated/SwiftWinAssets.swift")).unwrap();
        assert!(gen.contains("static let brand = Color(.sRGB, red: 0.0000, green: 0.4780, blue: 1.0000"));

        let ipa = package_ipa(&out.app, &d.join("Hello.ipa")).unwrap();
        let mut z = zip::ZipArchive::new(std::fs::File::open(&ipa).unwrap()).unwrap();
        assert_eq!(z.by_name("Payload/Hello.app/Hello").unwrap().unix_mode().unwrap() & 0o777, 0o755);
        assert!(z.by_name("Payload/Hello.app/Info.plist").is_ok());
        // The .ipa is installable on the mock (bundle id readable).
        let f = z.by_name("Payload/Hello.app/Info.plist").unwrap();
        drop(f);
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn unsigned_ipa_for_other_signers() {
        let d = tmp("unsigned");
        let dir = scaffold::new_project(&d, "Hello", Some("com.example.hello")).unwrap();
        let p = Project::load(&dir).unwrap();
        let out = build(&p, &opts(), &FakeCompiler).unwrap();
        // As if `swiftwin run` had signed this .app earlier.
        std::fs::create_dir_all(out.app.join("_CodeSignature")).unwrap();
        std::fs::write(out.app.join("_CodeSignature/CodeResources"), b"x").unwrap();
        std::fs::write(out.app.join("embedded.mobileprovision"), b"x").unwrap();

        let signed = package_ipa(&out.app, &d.join("s.ipa")).unwrap();
        let z = zip::ZipArchive::new(std::fs::File::open(&signed).unwrap()).unwrap();
        assert!(z.file_names().flatten().any(|n| n == "Payload/Hello.app/_CodeSignature/CodeResources"));
        assert!(z.file_names().flatten().any(|n| n == "Payload/Hello.app/_CodeSignature/"));

        let ipa = package_unsigned_ipa(&out.app, &d.join("u.ipa")).unwrap();
        let mut z = zip::ZipArchive::new(std::fs::File::open(&ipa).unwrap()).unwrap();
        let names: Vec<String> = z.file_names().flatten().map(|n| n.to_string()).collect();
        assert!(names.contains(&"Payload/".to_string()) && names.contains(&"Payload/Hello.app/".to_string()));
        assert!(!names.iter().any(|n| n.contains("_CodeSignature") || n.ends_with("embedded.mobileprovision")), "{names:?}");
        assert_eq!(z.by_name("Payload/Hello.app/Hello").unwrap().unix_mode().unwrap() & 0o777, 0o755);
        assert!(is_signing_artifact("PlugIns/X.xctest/_CodeSignature/CodeResources"));
        assert!(!is_signing_artifact("Info.plist"));

        // A rebuild starts from an empty .app: no stale signature survives.
        let out = build(&p, &opts(), &FakeCompiler).unwrap();
        assert!(!out.app.join("_CodeSignature").exists() && !out.app.join("embedded.mobileprovision").exists());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn info_keys_and_errors() {
        let d = tmp("info");
        let dir = scaffold::new_project(&d, "My-App", None).unwrap();
        let toml = std::fs::read_to_string(dir.join("swiftwin.toml")).unwrap()
            + "NSCameraUsageDescription = \"Scan\"\nUIBackgroundModes = [\"audio\"]\n";
        std::fs::write(dir.join("swiftwin.toml"), toml).unwrap();
        let p = Project::load(&dir).unwrap();
        assert_eq!(p.app().bundle_id, "com.example.myapp");
        assert_eq!(p.executable(), "MyApp");
        let out = build(&p, &opts(), &FakeCompiler).unwrap();
        let info: plist::Dictionary = plist::from_file(out.app.join("Info.plist")).unwrap();
        assert_eq!(info["NSCameraUsageDescription"].as_string(), Some("Scan"));
        assert_eq!(info["UIBackgroundModes"].as_array().unwrap().len(), 1);

        std::fs::write(dir.join("Sources/Bad.swift"), "#error(\"nope\")").unwrap();
        assert!(build(&p, &opts(), &FakeCompiler).unwrap_err().to_string().contains("#error"));
        assert!(scaffold::new_project(&d, "My-App", None).is_err());
        assert!(scaffold::new_project(&d, "bad name", None).is_err());
        // Inside a folder already named like the project: in place, not Tempo/Tempo.
        let here = d.join("Tempo");
        std::fs::create_dir_all(&here).unwrap();
        let made = scaffold::new_project(&here, "Tempo", None).unwrap();
        assert_eq!(made, here);
        assert!(here.join("swiftwin.toml").is_file() && here.join(".gitignore").is_file());
        assert!(!here.join("Tempo").exists());
        assert!(scaffold::new_project(&here, "Tempo", None).is_err(), "not twice");
        assert!(Project::load(&d.join("nowhere")).is_err());
        std::fs::write(dir.join("swiftwin.toml"), "[app]\nname=\"x\"\nbundle_id=\"nodot\"\n").unwrap();
        assert!(Project::load(&dir).unwrap_err().to_string().contains("bundle_id"));
        std::fs::remove_dir_all(d).ok();
    }
}
