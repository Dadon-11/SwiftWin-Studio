//! Reading app identity (bundle id, name, version) from `.app` folders and `.ipa` files.

use std::io::Read;
use std::path::Path;

use swiftwin_core::{AppInfo, Result, SwiftWinError};

/// Read `AppInfo` from an `.app` directory or an `.ipa` (zip with `Payload/X.app/Info.plist`).
pub fn read_app_info(package: &Path) -> Result<AppInfo> {
    if package.is_dir() {
        let plist = package.join("Info.plist");
        let data = std::fs::read(&plist)
            .map_err(|e| SwiftWinError::invalid(format!("{} has no readable Info.plist: {e}", package.display())))?;
        return app_info_from_plist(&data);
    }
    let f = std::fs::File::open(package)
        .map_err(|e| SwiftWinError::invalid(format!("cannot open {}: {e}", package.display())))?;
    let mut zip = zip::ZipArchive::new(f)
        .map_err(|e| SwiftWinError::invalid(format!("{} is not an .ipa/.zip: {e}", package.display())))?;
    let name = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().and_then(|f| f.name().ok().map(|n| n.to_string())))
        .find(|n| is_top_level_info_plist(n))
        .ok_or_else(|| SwiftWinError::invalid("no Payload/<App>.app/Info.plist inside the .ipa"))?;
    let mut data = Vec::new();
    zip.by_name(&name)
        .map_err(|e| SwiftWinError::invalid(e.to_string()))?
        .read_to_end(&mut data)?;
    app_info_from_plist(&data)
}

fn is_top_level_info_plist(name: &str) -> bool {
    let parts: Vec<&str> = name.split('/').collect();
    parts.len() == 3 && parts[0] == "Payload" && parts[1].ends_with(".app") && parts[2] == "Info.plist"
}

/// Parse an Info.plist (XML or binary).
pub fn app_info_from_plist(data: &[u8]) -> Result<AppInfo> {
    let v: plist::Value = plist::from_bytes(data).map_err(|e| SwiftWinError::invalid(format!("bad Info.plist: {e}")))?;
    let d = v.as_dictionary().ok_or_else(|| SwiftWinError::invalid("Info.plist is not a dictionary"))?;
    let s = |k: &str| d.get(k).and_then(|v| v.as_string()).map(str::to_string);
    let bundle_id = s("CFBundleIdentifier").ok_or_else(|| SwiftWinError::invalid("Info.plist lacks CFBundleIdentifier"))?;
    let name = s("CFBundleDisplayName").or_else(|| s("CFBundleName")).unwrap_or_else(|| bundle_id.clone());
    Ok(AppInfo { bundle_id, name, version: s("CFBundleShortVersionString") })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    pub(crate) fn info_plist(id: &str, name: &str) -> Vec<u8> {
        let mut d = plist::Dictionary::new();
        d.insert("CFBundleIdentifier".into(), id.into());
        d.insert("CFBundleName".into(), name.into());
        d.insert("CFBundleShortVersionString".into(), "1.2".into());
        let mut out = Vec::new();
        plist::Value::Dictionary(d).to_writer_xml(&mut out).unwrap();
        out
    }

    #[test]
    fn reads_app_dir_and_ipa() {
        let dir = std::env::temp_dir().join(format!("swiftwin-bundle-{}", std::process::id()));
        let app = dir.join("Hello.app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("Info.plist"), info_plist("com.example.hello", "Hello")).unwrap();
        let i = read_app_info(&app).unwrap();
        assert_eq!(i.bundle_id, "com.example.hello");
        assert_eq!(i.version.as_deref(), Some("1.2"));

        let ipa = dir.join("Hello.ipa");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&ipa).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("Payload/Hello.app/Frameworks/X.framework/Info.plist", o).unwrap();
        z.write_all(&info_plist("com.example.framework", "X")).unwrap();
        z.start_file("Payload/Hello.app/Info.plist", o).unwrap();
        z.write_all(&info_plist("com.example.hello", "Hello")).unwrap();
        z.finish().unwrap();
        assert_eq!(read_app_info(&ipa).unwrap().bundle_id, "com.example.hello");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_garbage() {
        assert!(app_info_from_plist(b"not a plist").is_err());
        assert!(read_app_info(Path::new("/definitely/not/here.ipa")).is_err());
    }
}
