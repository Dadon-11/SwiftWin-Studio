//! `swiftwin new <Name>` – a Hello SwiftUI project.

use std::path::{Path, PathBuf};

use swiftwin_core::{Result, SwiftWinError};

use crate::assets::encode_png;

pub fn new_project(parent: &Path, name: &str, bundle_id: Option<&str>) -> Result<PathBuf> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(SwiftWinError::invalid("project name: use letters, digits, '-' or '_'"));
    }
    let dir = project_dir(parent, name);
    if dir.join(crate::project::PROJECT_FILE).exists() {
        return Err(SwiftWinError::invalid(format!("{} already contains a SwiftWin project", dir.display())));
    }
    if dir.join("Sources").exists() {
        return Err(SwiftWinError::invalid(format!("{} already has a Sources folder: use an empty folder", dir.display())));
    }
    let ident: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let bundle = bundle_id.map(str::to_string).unwrap_or_else(|| format!("com.example.{}", ident.to_lowercase()));
    let w = |rel: &str, data: &[u8]| -> Result<()> {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(p, data)?;
        Ok(())
    };
    w(
        "swiftwin.toml",
        format!(
            "[app]\nname = \"{name}\"\nbundle_id = \"{bundle}\"\nversion = \"1.0\"\nbuild = \"1\"\ndeployment_target = \"17.0\"\n\n# Extra Info.plist keys:\n[info]\n"
        )
        .as_bytes(),
    )?;
    w(
        &format!("Sources/{ident}App.swift"),
        format!(
            "import SwiftUI\n\n@main\nstruct {ident}App: App {{\n    var body: some Scene {{\n        WindowGroup {{\n            ContentView()\n        }}\n    }}\n}}\n"
        )
        .as_bytes(),
    )?;
    w("Sources/ContentView.swift", content_view(&bundle).as_bytes())?;
    w("Assets.xcassets/Contents.json", br#"{"info":{"author":"swiftwin","version":1}}"#)?;
    w(
        "Assets.xcassets/AppIcon.appiconset/Contents.json",
        br#"{"images":[{"filename":"icon-1024.png","idiom":"universal","platform":"ios","size":"1024x1024"}],"info":{"author":"swiftwin","version":1}}"#,
    )?;
    w("Assets.xcassets/AppIcon.appiconset/icon-1024.png", &icon_png(1024, name))?;
    w(
        "Assets.xcassets/Brand.colorset/Contents.json",
        br#"{"colors":[{"idiom":"universal","color":{"color-space":"srgb","components":{"red":"0.000","green":"0.478","blue":"1.000","alpha":"1.000"}}}],"info":{"author":"swiftwin","version":1}}"#,
    )?;
    w(".gitignore", b"build/\n")?;
    Ok(dir)
}

/// `<parent>/<name>`, or `parent` itself when it is already named `name` (`swiftwin new Tempo
/// --dir .` inside a `Tempo` folder used to create `Tempo/Tempo`).
pub fn project_dir(parent: &Path, name: &str) -> PathBuf {
    let named = std::fs::canonicalize(parent)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case(name)))
        .unwrap_or(false);
    if named {
        parent.to_path_buf()
    } else {
        parent.join(name)
    }
}

fn content_view(bundle: &str) -> String {
    format!(
        r#"import SwiftUI
import os

private let log = Logger(subsystem: "{bundle}", category: "ui")

struct ContentView: View {{
    @State private var count = 0
    @State private var name = ""

    var body: some View {{
        VStack(alignment: .leading, spacing: 16) {{
            Text("Hello").font(.largeTitle).bold()
            Text("Count: \(count)")
            Button("Tap me") {{
                count += 1
                log.notice("Button tapped, count = \(count, privacy: .public)")
            }}
            .buttonStyle(.borderedProminent)
            .tint(.brand)
            TextField("Your name", text: $name)
                .textFieldStyle(.roundedBorder)
            Text(name.isEmpty ? "Hello, SwiftWin!" : "Hello, \(name)!")
            List(1...40, id: \.self) {{ i in
                Text("Item \(i)")
            }}
            .listStyle(.plain)
        }}
        .padding()
    }}
}}
"#
    )
}

/// A starter icon: a two-tone gradient whose colour comes from the app's name (so apps look
/// different on the home screen) with a soft white ring. Meant to be replaced by the user's own.
pub fn icon_png(size: u32, name: &str) -> Vec<u8> {
    // FNV-1a of the name picks the hue: stable across runs and platforms.
    let hash = name.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    let hue = (hash % 360) as f64;
    let (top, bottom) = (hsl(hue, 0.78, 0.58), hsl((hue + 32.0) % 360.0, 0.82, 0.42));
    let mut px = Vec::with_capacity((size * size * 3) as usize);
    for y in 0..size {
        for x in 0..size {
            let t = (x + y) as f64 / (2 * size) as f64;
            let (dx, dy) = (x as f64 / size as f64 - 0.5, y as f64 / size as f64 - 0.5);
            let r = (dx * dx + dy * dy).sqrt();
            // Ring between radius 0.20 and 0.29, with a one-pixel soft edge.
            let edge = 1.5 / size as f64;
            let ring = ((r - 0.20) / edge).clamp(0.0, 1.0) * ((0.29 - r) / edge).clamp(0.0, 1.0);
            for c in 0..3 {
                let base = top[c] + (bottom[c] - top[c]) * t;
                px.push((base + (255.0 - base) * ring * 0.95) as u8);
            }
        }
    }
    encode_png(size, size, &px, png::ColorType::Rgb)
}

/// HSL (hue in degrees, saturation and lightness 0..1) to RGB 0..255.
fn hsl(h: f64, s: f64, l: f64) -> [f64; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [(r + m) * 255.0, (g + m) * 255.0, (b + m) * 255.0]
}

#[cfg(test)]
mod icon_tests {
    use super::icon_png;

    #[test]
    fn each_app_gets_its_own_icon_colour() {
        let (a, b) = (icon_png(64, "Pulse"), icon_png(64, "Notes"));
        assert_eq!(swiftwin_core::png_dimensions(&a), Some((64, 64)));
        assert_eq!(a, icon_png(64, "Pulse"), "the same name always gives the same icon");
        assert_ne!(a, b);
        if let Ok(dir) = std::env::var("SWIFTWIN_ICON_SAMPLES") {
            std::fs::write(format!("{dir}/pulse.png"), icon_png(256, "Pulse")).unwrap();
            std::fs::write(format!("{dir}/notes.png"), icon_png(256, "Notes")).unwrap();
        }
    }
}
