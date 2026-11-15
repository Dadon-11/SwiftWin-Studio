//! Writes a Swift SDK artifact bundle (SE-0387 / `swift sdk install` format)
//! that points at the extracted iOS SDK, so SwiftPM can also cross-compile:
//! `swift build --swift-sdk swiftwin-ios --triple arm64-apple-ios`.

use std::path::{Path, PathBuf};

use serde_json::json;
use swiftwin_core::Result;

pub const ARTIFACT_ID: &str = "swiftwin-ios";

// UNTESTED-ON-DEVICE: the bundle is only validated structurally; `swift build --swift-sdk` with it
// has never been run against a real iOS SDK on Windows.
pub fn write_bundle(root: &Path, sdk_path: &Path, version: &str) -> Result<PathBuf> {
    let bundle = root.join(format!("{ARTIFACT_ID}.artifactbundle"));
    let variant = bundle.join(ARTIFACT_ID);
    std::fs::create_dir_all(&variant)?;
    let info = json!({
        "schemaVersion": "1.0",
        "artifacts": {
            ARTIFACT_ID: { "type": "swiftSDK", "version": version, "variants": [ { "path": ARTIFACT_ID } ] }
        }
    });
    std::fs::write(bundle.join("info.json"), serde_json::to_vec_pretty(&info)?)?;
    let rel_sdk = format!(
        "../../SDKs/{}",
        sdk_path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    );
    let sdk = json!({
        "schemaVersion": "4.0",
        "targetTriples": {
            "arm64-apple-ios": { "sdkRootPath": rel_sdk, "toolsetPaths": ["toolset.json"] }
        }
    });
    std::fs::write(variant.join("swift-sdk.json"), serde_json::to_vec_pretty(&sdk)?)?;
    let toolset = json!({
        "schemaVersion": "1.0",
        "swiftCompiler": { "extraCLIOptions": ["-use-ld=lld"] },
        "linker": { "path": "ld64.lld" }
    });
    std::fs::write(variant.join("toolset.json"), serde_json::to_vec_pretty(&toolset)?)?;
    Ok(bundle)
}
