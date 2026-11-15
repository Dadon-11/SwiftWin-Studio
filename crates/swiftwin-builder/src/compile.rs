//! Driving the Swift compiler and the linker.
//!
//! Real builds use the open-source swift.org toolchain for Windows in two steps:
//!
//! 1. `swiftc -c -wmo` compiles every source into one `arm64-apple-ios` object.
//! 2. `ld64.lld` links it against the iOS SDK's `.tbd` stubs ([`find_ld64`]).
//!
//! SwiftWin runs the linker itself instead of letting swiftc link (`-use-ld=lld` /
//! `-ld-path=`): on Windows the driver adds a mangled `-rpath \usr\lib\swift`, links
//! through clang with `--sysroot` (so the binary records SDK 17.0 instead of the real
//! SDK version) and runs `dsymutil`, which would put `Hello.dSYM` inside the `.app`.
//! The swift.org toolchain's own lld refuses Mach-O inputs that carry a platform load
//! command (see `macho.rs`); SwiftWin strips those from its inputs, so that lld works too,
//! but an upstream LLVM lld is preferred. Tests and `--mock` use `FakeCompiler`.

use std::path::{Path, PathBuf};
use std::process::Command;

use swiftwin_core::{Result, SwiftWinError};

use crate::macho;

#[derive(Debug, Clone)]
pub struct CompileJob {
    pub module_name: String,
    pub sources: Vec<PathBuf>,
    pub sdk: PathBuf,
    /// SDK version, e.g. "26.1" (the `sdk` field of the executable's LC_BUILD_VERSION).
    pub sdk_version: String,
    /// e.g. "arm64-apple-ios17.0"
    pub target: String,
    pub output: PathBuf,
    /// Scratch folder (outside the `.app`) for the object file and patched libraries.
    pub work_dir: PathBuf,
    /// Module cache; the SDK's Swift interfaces are compiled into it on the first build.
    pub module_cache: PathBuf,
    pub release: bool,
    pub extra_args: Vec<String>,
}

impl CompileJob {
    /// "17.0" from "arm64-apple-ios17.0".
    pub fn deployment_target(&self) -> &str {
        self.target.rsplit_once("ios").map(|(_, v)| v).unwrap_or("")
    }

    pub fn object(&self) -> PathBuf {
        self.work_dir.join(format!("{}.o", self.module_name))
    }
}

pub trait Compiler: Send + Sync {
    fn describe(&self) -> String;
    /// Compile and link; returns the compiler's output (warnings).
    fn compile(&self, job: &CompileJob) -> Result<String>;
}

pub struct Swiftc {
    pub path: PathBuf,
}

/// Where Xcode's toolchain libraries go next to an imported SDK
/// (`<sdk root>/SDKs/iPhoneOS26.1.sdk` → `<sdk root>/Toolchain/usr/lib`):
///
/// * `swift/iphoneos/libswiftCompatibility*.a`, from
///   `XcodeDefault.xctoolchain/usr/lib/swift/iphoneos/` (back-deployment to iOS 15–16)
/// * `clang/darwin/libclang_rt.ios.a`, from
///   `XcodeDefault.xctoolchain/usr/lib/clang/<N>/lib/darwin/` (C `@available` checks)
///
/// Neither is in the SDK itself, and the swift.org toolchain has no Darwin runtimes.
pub fn xcode_toolchain_lib(sdk: &Path) -> Option<PathBuf> {
    Some(sdk.parent()?.parent()?.join("Toolchain").join("usr").join("lib"))
}

/// Swift back-deployment libraries the frontend autolinks below iOS 17, as reported by
/// `swiftc -print-target-info -target arm64-apple-ios<N>` (Swift 6.2).
pub fn compatibility_libraries(deployment_target: &str) -> &'static [&'static str] {
    match deployment_target.split('.').next().and_then(|m| m.parse::<u32>().ok()) {
        Some(m) if m <= 15 => &["swiftCompatibility56", "swiftCompatibilityPacks"],
        Some(16) => &["swiftCompatibilityPacks"],
        _ => &[],
    }
}

impl Swiftc {
    /// `swiftc` arguments: compile (no link) into `job.object()`.
    pub fn args(job: &CompileJob) -> Vec<String> {
        let mut a: Vec<String> = vec![
            "-target".into(),
            job.target.clone(),
            "-sdk".into(),
            job.sdk.to_string_lossy().into_owned(),
            "-module-name".into(),
            job.module_name.clone(),
            "-c".into(),
            "-wmo".into(),
            "-o".into(),
            job.object().to_string_lossy().into_owned(),
            "-module-cache-path".into(),
            job.module_cache.to_string_lossy().into_owned(),
        ];
        // `@main` needs -parse-as-library unless there is a top-level main.swift.
        if !job.sources.iter().any(|s| s.file_name().is_some_and(|n| n == "main.swift")) {
            a.push("-parse-as-library".into());
        }
        if job.release {
            a.push("-O".into());
        } else {
            a.extend(["-Onone".into(), "-g".into(), "-DDEBUG".into()]);
        }
        a.extend(job.extra_args.iter().cloned());
        a.extend(job.sources.iter().map(|s| s.to_string_lossy().into_owned()));
        a
    }

    /// `ld64.lld` arguments for the app executable. `inputs` are objects and static
    /// archives (already stripped of platform load commands).
    pub fn link_args(job: &CompileJob, inputs: &[PathBuf], lib_dirs: &[PathBuf]) -> Vec<String> {
        let min = job.deployment_target().to_string();
        let sdk_version = if job.sdk_version.is_empty() { min.clone() } else { job.sdk_version.clone() };
        let path = |p: &Path| p.to_string_lossy().into_owned();
        let mut a: Vec<String> = vec![
            "-arch".into(),
            "arm64".into(),
            "-platform_version".into(),
            "ios".into(),
            min,
            sdk_version,
            "-syslibroot".into(),
            path(&job.sdk),
            "-o".into(),
            path(&job.output),
            "-L".into(),
            path(&job.sdk.join("usr").join("lib").join("swift")),
        ];
        for d in lib_dirs {
            a.extend(["-L".into(), path(d)]);
        }
        a.extend(inputs.iter().map(|p| path(p)));
        a.extend(
            [
                "-lSystem",
                "-rpath",
                "/usr/lib/swift",
                "-rpath",
                "@executable_path/Frameworks",
                // lld doesn't sign iOS device binaries by default; an ad-hoc signature
                // (as Apple's ld64 writes for arm64) gives re-signers an LC_CODE_SIGNATURE
                // to replace, and the header padding leaves room for more load commands.
                "-adhoc_codesign",
                "-headerpad_max_install_names",
            ]
            .map(String::from),
        );
        if job.release {
            a.push("-dead_strip".into());
        }
        a
    }

    pub fn linker(&self) -> Option<PathBuf> {
        find_ld64(&self.path)
    }
}

const LD64: &str = if cfg!(windows) { "ld64.lld.exe" } else { "ld64.lld" };
const SWIFTC: &str = if cfg!(windows) { "swiftc.exe" } else { "swiftc" };

/// The Mach-O linker: `SWIFTWIN_LD64`, upstream LLVM (`%ProgramFiles%\LLVM\bin`,
/// `winget install LLVM.LLVM`), an `ld64.lld` on PATH outside a Swift toolchain, then
/// the one next to `swiftc` (Swift's lld fork, usable because SwiftWin strips platform
/// load commands).
pub fn find_ld64(swiftc: &Path) -> Option<PathBuf> {
    let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from).or_else(|| cfg!(windows).then(|| "C:\\Program Files".into()));
    let path_dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).filter(|d| d.is_absolute()).collect()).unwrap_or_default();
    pick_ld64(std::env::var_os("SWIFTWIN_LD64").map(PathBuf::from), program_files, &path_dirs, swiftc, |p| p.exists())
}

fn pick_ld64(
    env: Option<PathBuf>,
    program_files: Option<PathBuf>,
    path_dirs: &[PathBuf],
    swiftc: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if env.is_some() {
        return env;
    }
    let upstream = program_files.map(|p| p.join("LLVM").join("bin").join(LD64)).filter(|p| exists(p));
    upstream
        .or_else(|| path_dirs.iter().filter(|d| !exists(&d.join(SWIFTC))).map(|d| d.join(LD64)).find(|p| exists(p)))
        .or_else(|| Some(swiftc.with_file_name(LD64)).filter(|p| exists(p)))
}

/// Copy `src` into `dir`, stripping platform load commands so lld accepts it.
fn stage_library(src: &Path, dir: &Path) -> Result<PathBuf> {
    let mut data = std::fs::read(src)?;
    macho::strip_platform_load_commands(&mut data)?;
    std::fs::create_dir_all(dir)?;
    let dst = dir.join(src.file_name().ok_or_else(|| SwiftWinError::invalid("bad library path"))?);
    std::fs::write(&dst, data)?;
    Ok(dst)
}

/// Back-deployment libraries and `libclang_rt.ios.a` from Xcode's toolchain, if the
/// SDK import kept them: (extra archives to link, extra `-L` dirs).
fn toolchain_libraries(job: &CompileJob) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let lib = xcode_toolchain_lib(&job.sdk);
    let staged = job.work_dir.join("lib");
    let (mut archives, mut dirs) = (Vec::new(), Vec::new());
    let compat = compatibility_libraries(job.deployment_target());
    if !compat.is_empty() {
        let src = lib.as_ref().map(|l| l.join("swift").join("iphoneos"));
        let missing: Vec<&str> =
            compat.iter().copied().filter(|n| !src.as_ref().is_some_and(|s| s.join(format!("lib{n}.a")).exists())).collect();
        if !missing.is_empty() {
            return Err(SwiftWinError::invalid(format!(
                "deployment_target {} needs Swift's back-deployment libraries ({}), which only Xcode's toolchain has and \
                 this SDK import doesn't include. Set deployment_target = \"17.0\" (or newer) in swiftwin.toml.",
                job.deployment_target(),
                missing.join(", ")
            )));
        }
        let src = src.unwrap();
        for n in compat {
            stage_library(&src.join(format!("lib{n}.a")), &staged)?;
        }
        dirs.push(staged.clone());
    }
    if let Some(rt) = lib.map(|l| l.join("clang").join("darwin").join("libclang_rt.ios.a")).filter(|p| p.exists()) {
        archives.push(stage_library(&rt, &staged)?);
    }
    Ok((archives, dirs))
}

/// swift.org layout: `<Swift>\Toolchains\6.2.4+Asserts\usr\bin\swiftc.exe` only starts
/// with `<Swift>\Runtimes\6.2.4\usr\bin` (swiftCore.dll, …) on PATH.
pub fn runtime_bin_for(exe: &Path) -> Option<PathBuf> {
    let toolchain = exe.parent()?.parent()?.parent()?;
    let version = toolchain.file_name()?.to_str()?.split('+').next()?;
    let rt = toolchain.parent()?.parent()?.join("Runtimes").join(version).join("usr").join("bin");
    rt.is_dir().then_some(rt)
}

/// A `Command` for a Swift toolchain executable, with its runtime DLLs on PATH even
/// when swiftc was found without PATH (e.g. the app started before Swift was installed).
pub fn swift_command(exe: &Path) -> Command {
    let mut c = Command::new(exe);
    if let Some(rt) = runtime_bin_for(exe) {
        let mut dirs = vec![rt];
        if let Some(p) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&p));
        }
        if let Ok(joined) = std::env::join_paths(dirs) {
            c.env("PATH", joined);
        }
    }
    c
}

fn run(mut cmd: Command, args: &[String], tool: &str, exe: &Path) -> Result<String> {
    let out = cmd.args(args).output().map_err(|e| SwiftWinError::ToolMissing {
        tool: tool.into(),
        hint: format!("{} could not be started ({e}); install Swift from https://www.swift.org/install/windows/", exe.display()),
    })?;
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        let mut msg = log.trim().to_string();
        if msg.contains("isPlatformVersionAtLeast") || msg.contains("isOSVersionAtLeast") {
            msg.push_str("\n(hint: `@available` checks need libclang_rt.ios.a from Xcode's toolchain; re-import the SDK with a SwiftWin that extracts it)");
        }
        if msg.is_empty() {
            msg = format!("exit status {}", out.status);
        }
        return Err(SwiftWinError::tool(tool, msg));
    }
    Ok(log)
}

impl Compiler for Swiftc {
    fn describe(&self) -> String {
        format!("swiftc ({})", self.path.display())
    }

    // A SwiftUI app built this way (iOS 26.2 SDK, Swift 6.2.4 on Windows) runs on an iPhone with iOS 27.0.1.
    // Verified without an SDK: swiftc 6.2.4 emits arm64 iOS objects, and ld64.lld links stripped
    // objects against .tbd stubs into an ad-hoc-signed MH_EXECUTE with LC_BUILD_VERSION ios.
    fn compile(&self, job: &CompileJob) -> Result<String> {
        std::fs::create_dir_all(&job.work_dir)?;
        std::fs::create_dir_all(&job.module_cache)?;
        let obj = job.object();
        let _ = std::fs::remove_file(&obj);
        let mut log = run(swift_command(&self.path), &Self::args(job), "swiftc", &self.path)?;
        let mut data = std::fs::read(&obj)
            .map_err(|e| SwiftWinError::tool("swiftc", format!("no object file at {} ({e})", obj.display())))?;
        macho::strip_platform_load_commands(&mut data)?;
        std::fs::write(&obj, data)?;

        let (archives, lib_dirs) = toolchain_libraries(job)?;
        let mut inputs = vec![obj];
        inputs.extend(archives);
        let linker = self.linker().ok_or_else(|| SwiftWinError::ToolMissing {
            tool: "ld64.lld (the Mach-O linker)".into(),
            hint: "install LLVM with `winget install LLVM.LLVM`, or set SWIFTWIN_LD64 to an ld64.lld.exe".into(),
        })?;
        log.push_str(&run(swift_command(&linker), &Self::link_args(job, &inputs, &lib_dirs), "ld64.lld", &linker)?);
        Ok(log)
    }
}

/// Stands in for swiftc in tests and `--mock`: validates inputs and writes a
/// tiny arm64 Mach-O header so the rest of the pipeline can run.
pub struct FakeCompiler;

pub const MACHO_ARM64_HEADER: [u8; 32] = [
    0xcf, 0xfa, 0xed, 0xfe, // MH_MAGIC_64
    0x0c, 0x00, 0x00, 0x01, // CPU_TYPE_ARM64
    0x00, 0x00, 0x00, 0x00, // CPU_SUBTYPE_ARM64_ALL
    0x02, 0x00, 0x00, 0x00, // MH_EXECUTE
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

impl Compiler for FakeCompiler {
    fn describe(&self) -> String {
        "mock compiler (no real code is generated)".into()
    }
    fn compile(&self, job: &CompileJob) -> Result<String> {
        for s in &job.sources {
            let text = std::fs::read_to_string(s)?;
            if text.contains("#error") {
                return Err(SwiftWinError::tool("swiftc", format!("{}: error: #error directive", s.display())));
            }
        }
        std::fs::write(&job.output, MACHO_ARM64_HEADER)?;
        Ok(format!("mock-compiled {} file(s) for {}", job.sources.len(), job.target))
    }
}

/// Find `swiftc`: explicit dir, `SWIFTWIN_SWIFTC`, PATH, then the default Windows install location.
pub fn find_swiftc(configured_bin: Option<&str>) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "swiftc.exe" } else { "swiftc" };
    if let Some(b) = configured_bin {
        let p = Path::new(b).join(exe);
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(p) = std::env::var("SWIFTWIN_SWIFTC") {
        return Some(PathBuf::from(p));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path).filter(|d| d.is_absolute()) {
            let p = d.join(exe);
            if p.exists() {
                return Some(p);
            }
        }
    }
    // %LOCALAPPDATA%\Programs\Swift\Toolchains\<version>\usr\bin\swiftc.exe (swift.org installer default)
    let base = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Programs").join("Swift").join("Toolchains");
    let mut tcs: Vec<PathBuf> = std::fs::read_dir(base).ok()?.flatten().map(|e| e.path()).collect();
    tcs.sort();
    tcs.into_iter().rev().map(|t| t.join("usr").join("bin").join(exe)).find(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> CompileJob {
        CompileJob {
            module_name: "Hello".into(),
            sources: vec!["a/HelloApp.swift".into(), "a/ContentView.swift".into()],
            sdk: "C:/sdk/iPhoneOS26.1/SDKs/iPhoneOS26.1.sdk".into(),
            sdk_version: "26.1".into(),
            target: "arm64-apple-ios17.0".into(),
            output: "out/Hello.app/Hello".into(),
            work_dir: "out/obj".into(),
            module_cache: "build/ModuleCache".into(),
            release: false,
            extra_args: vec![],
        }
    }

    #[test]
    fn swiftc_args() {
        let job = job();
        let a = Swiftc::args(&job);
        let s = a.join(" ");
        assert!(s.starts_with("-target arm64-apple-ios17.0 -sdk C:/sdk/iPhoneOS26.1/SDKs/iPhoneOS26.1.sdk -module-name Hello -c -wmo -o"));
        assert!(s.contains(&format!("-o {}", Path::new("out/obj").join("Hello.o").display())));
        assert!(s.contains("-module-cache-path build/ModuleCache"));
        assert!(a.contains(&"-parse-as-library".to_string()));
        assert!(!a.iter().any(|x| x.starts_with("-use-ld") || x == "-emit-executable"), "swiftc must not link");
        assert!(s.ends_with("a/HelloApp.swift a/ContentView.swift"));
        let mut job2 = job.clone();
        job2.sources = vec!["main.swift".into()];
        job2.release = true;
        let a2 = Swiftc::args(&job2);
        assert!(!a2.contains(&"-parse-as-library".to_string()));
        assert!(a2.contains(&"-O".to_string()));
        assert!(!a2.contains(&"-g".to_string()));
    }

    #[test]
    fn linker_args() {
        let job = job();
        assert_eq!(job.deployment_target(), "17.0");
        let a = Swiftc::link_args(&job, &[job.object()], &[]);
        let s = a.join(" ");
        assert!(s.starts_with("-arch arm64 -platform_version ios 17.0 26.1 -syslibroot C:/sdk/iPhoneOS26.1/SDKs/iPhoneOS26.1.sdk -o out/Hello.app/Hello -L"));
        assert!(s.contains("-lSystem -rpath /usr/lib/swift -rpath @executable_path/Frameworks -adhoc_codesign"));
        assert!(a.contains(&job.object().to_string_lossy().into_owned()));
        assert!(!a.contains(&"-dead_strip".to_string()));
        let mut rel = job.clone();
        rel.release = true;
        rel.sdk_version = String::new();
        let a = Swiftc::link_args(&rel, &[], &["x/lib".into()]);
        assert!(a.join(" ").contains("-platform_version ios 17.0 17.0"), "falls back to the deployment target");
        assert!(a.contains(&"-dead_strip".to_string()));
        assert!(a.windows(2).any(|w| w[0] == "-L" && w[1] == "x/lib"));
    }

    #[test]
    fn toolchain_libs_and_compat() {
        assert!(compatibility_libraries("17.0").is_empty());
        assert!(compatibility_libraries("26.1").is_empty());
        assert_eq!(compatibility_libraries("16.4"), &["swiftCompatibilityPacks"]);
        assert_eq!(compatibility_libraries("15.0").len(), 2);
        let sdk = Path::new("C:/t/iPhoneOS26.1/SDKs/iPhoneOS26.1.sdk");
        assert_eq!(xcode_toolchain_lib(sdk).unwrap(), Path::new("C:/t/iPhoneOS26.1/Toolchain/usr/lib"));

        let d = std::env::temp_dir().join(format!("swiftwin-compile-tc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let sdk = d.join("iPhoneOS26.1/SDKs/iPhoneOS26.1.sdk");
        std::fs::create_dir_all(&sdk).unwrap();
        let mut j = job();
        j.sdk = sdk.clone();
        j.work_dir = d.join("obj");
        // iOS 17: nothing needed, nothing staged.
        assert_eq!(toolchain_libraries(&j).unwrap(), (vec![], vec![]));
        // iOS 16 without the libraries: a clear error.
        j.target = "arm64-apple-ios16.0".into();
        let e = toolchain_libraries(&j).unwrap_err().to_string();
        assert!(e.contains("swiftCompatibilityPacks") && e.contains("17.0"), "{e}");
        // With Xcode's libraries next to the SDK: staged (stripped) copies.
        let lib = xcode_toolchain_lib(&sdk).unwrap();
        std::fs::create_dir_all(lib.join("swift/iphoneos")).unwrap();
        std::fs::create_dir_all(lib.join("clang/darwin")).unwrap();
        std::fs::write(lib.join("swift/iphoneos/libswiftCompatibilityPacks.a"), crate::macho::tests::object()).unwrap();
        std::fs::write(lib.join("clang/darwin/libclang_rt.ios.a"), crate::macho::tests::object()).unwrap();
        let (archives, dirs) = toolchain_libraries(&j).unwrap();
        assert_eq!(dirs, vec![d.join("obj/lib")]);
        assert_eq!(archives, vec![d.join("obj/lib/libclang_rt.ios.a")]);
        let staged = std::fs::read(d.join("obj/lib/libswiftCompatibilityPacks.a")).unwrap();
        assert!(crate::macho::platform_commands(&staged).is_empty());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn linker_discovery_order() {
        let swiftc = PathBuf::from("S/Toolchains/6.2.4+Asserts/usr/bin").join(SWIFTC);
        let swift_ld = swiftc.with_file_name(LD64);
        let upstream = PathBuf::from("PF/LLVM/bin").join(LD64);
        let other = PathBuf::from("tools").join(LD64);
        let dirs = vec![swiftc.parent().unwrap().to_path_buf(), "tools".into()];
        let all = [swiftc.clone(), swift_ld.clone(), upstream.clone(), other.clone()];
        let has = |set: &[PathBuf]| {
            let set = set.to_vec();
            move |p: &Path| set.iter().any(|s| s == p)
        };
        let pick = |env: Option<PathBuf>, set: &[PathBuf]| pick_ld64(env, Some("PF".into()), &dirs, &swiftc, has(set));
        assert_eq!(pick(Some("x/ld.exe".into()), &all), Some("x/ld.exe".into()), "SWIFTWIN_LD64 wins");
        assert_eq!(pick(None, &all), Some(upstream), "upstream LLVM next");
        // PATH: the Swift toolchain's bin dir (it has swiftc) is skipped.
        assert_eq!(pick(None, &[swiftc.clone(), swift_ld.clone(), other.clone()]), Some(other));
        assert_eq!(pick(None, &[swiftc.clone(), swift_ld.clone()]), Some(swift_ld), "Swift's lld last");
        assert_eq!(pick(None, &[swiftc.clone()]), None);
    }

    #[test]
    fn runtime_dir_from_toolchain_layout() {
        let d = std::env::temp_dir().join(format!("swiftwin-compile-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let bin = d.join("Swift/Toolchains/6.2.4+Asserts/usr/bin");
        let rt = d.join("Swift/Runtimes/6.2.4/usr/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let swiftc = bin.join("swiftc.exe");
        assert_eq!(runtime_bin_for(&swiftc), None);
        std::fs::create_dir_all(&rt).unwrap();
        assert_eq!(runtime_bin_for(&swiftc), Some(rt.clone()));
        let c = swift_command(&swiftc);
        let path = c.get_envs().find(|(k, _)| *k == "PATH").and_then(|(_, v)| v).unwrap();
        assert_eq!(std::env::split_paths(path).next(), Some(rt));
        std::fs::remove_dir_all(d).ok();
    }
}
