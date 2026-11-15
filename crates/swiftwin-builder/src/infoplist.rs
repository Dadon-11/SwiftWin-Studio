//! Info.plist generation (what Xcode would produce for a SwiftUI app).

use plist::{Dictionary, Value};
use swiftwin_core::Result;

use crate::project::Project;

pub struct InfoInputs<'a> {
    pub project: &'a Project,
    pub sdk_version: &'a str,
    /// From the asset catalog: CFBundleIconFiles base names.
    pub icon_files: Vec<String>,
}

fn arr(v: &[&str]) -> Value {
    Value::Array(v.iter().map(|s| Value::String(s.to_string())).collect())
}

pub fn orientation_key(o: &str) -> Option<&'static str> {
    Some(match o {
        "portrait" => "UIInterfaceOrientationPortrait",
        "upside-down" => "UIInterfaceOrientationPortraitUpsideDown",
        "landscape-left" => "UIInterfaceOrientationLandscapeLeft",
        "landscape-right" => "UIInterfaceOrientationLandscapeRight",
        _ => return None,
    })
}

pub fn generate(i: &InfoInputs) -> Result<Dictionary> {
    let a = i.project.app();
    let mut d = Dictionary::new();
    let s = |v: &str| Value::String(v.to_string());
    d.insert("CFBundleDevelopmentRegion".into(), s("en"));
    d.insert("CFBundleExecutable".into(), s(&i.project.executable()));
    d.insert("CFBundleIdentifier".into(), s(&a.bundle_id));
    d.insert("CFBundleInfoDictionaryVersion".into(), s("6.0"));
    d.insert("CFBundleName".into(), s(&a.name));
    d.insert("CFBundleDisplayName".into(), s(a.display_name.as_deref().unwrap_or(&a.name)));
    d.insert("CFBundlePackageType".into(), s("APPL"));
    d.insert("CFBundleShortVersionString".into(), s(&a.version));
    d.insert("CFBundleVersion".into(), s(&a.build));
    d.insert("CFBundleSupportedPlatforms".into(), arr(&["iPhoneOS"]));
    d.insert("LSRequiresIPhoneOS".into(), Value::Boolean(true));
    d.insert("MinimumOSVersion".into(), s(&a.deployment_target));
    d.insert("UIDeviceFamily".into(), Value::Array(vec![Value::Integer(1.into())]));
    d.insert("UIRequiredDeviceCapabilities".into(), arr(&["arm64"]));
    d.insert("UILaunchScreen".into(), Value::Dictionary(Dictionary::new()));
    let mut scenes = Dictionary::new();
    scenes.insert("UIApplicationSupportsMultipleScenes".into(), Value::Boolean(true));
    d.insert("UIApplicationSceneManifest".into(), Value::Dictionary(scenes));
    let orients: Vec<Value> =
        a.orientations.iter().filter_map(|o| orientation_key(o)).map(|k| Value::String(k.into())).collect();
    d.insert("UISupportedInterfaceOrientations".into(), Value::Array(orients));
    d.insert("DTPlatformName".into(), s("iphoneos"));
    d.insert("DTPlatformVersion".into(), s(i.sdk_version));
    d.insert("DTSDKName".into(), s(&format!("iphoneos{}", i.sdk_version)));
    if !i.icon_files.is_empty() {
        // Loose PNGs only. CFBundleIconName would point iOS at an Assets.car entry,
        // and SwiftWin can't build Assets.car (no actool), so it's left out.
        let mut primary = Dictionary::new();
        primary.insert("CFBundleIconFiles".into(), Value::Array(i.icon_files.iter().map(|f| s(f)).collect()));
        let mut icons = Dictionary::new();
        icons.insert("CFBundlePrimaryIcon".into(), Value::Dictionary(primary));
        d.insert("CFBundleIcons".into(), Value::Dictionary(icons));
    }
    for (k, v) in &i.project.file.info {
        d.insert(k.clone(), toml_to_plist(v));
    }
    Ok(d)
}

pub fn toml_to_plist(v: &toml::Value) -> Value {
    match v {
        toml::Value::String(s) => Value::String(s.clone()),
        toml::Value::Integer(i) => Value::Integer((*i).into()),
        toml::Value::Float(f) => Value::Real(*f),
        toml::Value::Boolean(b) => Value::Boolean(*b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.iter().map(toml_to_plist).collect()),
        toml::Value::Table(t) => {
            let mut d = Dictionary::new();
            for (k, v) in t {
                d.insert(k.clone(), toml_to_plist(v));
            }
            Value::Dictionary(d)
        }
    }
}
