//! Bakes the account-service IDs into the build, from (first found) the environment variables
//! `SWIFTWIN_BUILD_SUPABASE_URL`, `SWIFTWIN_BUILD_SUPABASE_KEY`, `SWIFTWIN_BUILD_GITHUB_CLIENT_ID`,
//! or the git-ignored `swiftwin-build.env` at the repository root (same names, `KEY=value` lines).
//! Neither is in the public source; without them accounts are simply not set up in that build.

use std::path::Path;

const KEYS: [&str; 3] = ["SWIFTWIN_BUILD_SUPABASE_URL", "SWIFTWIN_BUILD_SUPABASE_KEY", "SWIFTWIN_BUILD_GITHUB_CLIENT_ID"];

fn main() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../swiftwin-build.env");
    println!("cargo:rerun-if-changed={}", file.display());
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    for key in KEYS {
        println!("cargo:rerun-if-env-changed={key}");
        let from_file = text.lines().filter_map(|l| l.trim().split_once('=')).find(|(k, _)| k.trim() == key).map(|(_, v)| v.trim().to_string());
        let value = std::env::var(key).ok().filter(|v| !v.trim().is_empty()).or(from_file).unwrap_or_default();
        println!("cargo:rustc-env={key}={value}");
    }
}
