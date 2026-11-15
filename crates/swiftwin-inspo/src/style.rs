//! Style guide extraction: palette from the screenshots, type scale and spacing from the UI
//! trees, corner sizes from both, component patterns from element kinds. Writes `inspo.json`,
//! `INSPO.md` and `InspoTheme.swift`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use swiftwin_builder::assets::Image;
use swiftwin_core::{AppInfo, Rect, Result, UiNode};

use crate::{Shot, GUARDRAIL, PRIVACY};

#[derive(Debug, Clone, Serialize)]
pub struct Guide {
    pub app: String,
    pub bundle_id: String,
    pub captured_at: String,
    pub screens: Vec<ScreenRef>,
    pub palette: Palette,
    /// Font sizes seen (points), largest first.
    pub type_scale: Vec<TypeSize>,
    pub spacing: Spacing,
    pub components: Vec<Component>,
    pub guardrail: String,
    pub privacy: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScreenRef {
    pub image: String,
    pub tree: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Palette {
    pub background: String,
    pub surface: String,
    pub text: String,
    pub accent: Option<String>,
    /// The most used colours with their share of the screen area.
    pub dominant: Vec<Swatch>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Swatch {
    pub hex: String,
    pub share: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TypeSize {
    pub size: f64,
    pub count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Spacing {
    /// Distance of content from the screen edge.
    pub screen_padding: f64,
    /// Most common vertical gap between stacked elements.
    pub stack: f64,
    pub row_height: Option<f64>,
    /// Estimated from the screenshots' button and cell corners.
    pub corner_radius: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Component {
    pub name: String,
    /// On how many captured screens it appears.
    pub screens: usize,
}

type Rgb = [f64; 3];

/// All screens in one image, in order (01 top-left, row by row), so a reader sees everything at once.
pub fn overview(images: &[Image]) -> Vec<u8> {
    let cols = (if images.len() > 16 { 8 } else { 4 }).min(images.len().max(1)) as u32;
    let (tw, gap) = (150u32, 6u32);
    let th = images.iter().map(|i| i.h * tw / i.w.max(1)).max().unwrap_or(1).max(1);
    let rows = (images.len() as u32).div_ceil(cols).max(1);
    let (w, h) = (cols * (tw + gap) + gap, rows * (th + gap) + gap);
    let mut rgb = vec![40u8; (w * h * 3) as usize];
    for (k, img) in images.iter().enumerate() {
        let small = img.resize(tw, (img.h * tw / img.w.max(1)).max(1));
        let (ox, oy) = (gap + (k as u32 % cols) * (tw + gap), gap + (k as u32 / cols) * (th + gap));
        for y in 0..small.h {
            for x in 0..small.w {
                let (s, d) = (((y * small.w + x) * 4) as usize, (((oy + y) * w + ox + x) * 3) as usize);
                rgb[d..d + 3].copy_from_slice(&small.rgba[s..s + 3]);
            }
        }
    }
    swiftwin_builder::assets::encode_png(w, h, &rgb, png::ColorType::Rgb)
}

/// Analyse the shots and write everything into `dir` (replacing an older capture there).
pub fn write(dir: &Path, app: &AppInfo, shots: &[Shot], screen_w: f64) -> Result<Guide> {
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::create_dir_all(dir)?;
    let mut screens = Vec::new();
    let mut images = Vec::new();
    for (i, s) in shots.iter().enumerate() {
        let (image, tree) = (format!("{:02}.png", i + 1), format!("{:02}.json", i + 1));
        std::fs::write(dir.join(&image), &s.png)?;
        std::fs::write(dir.join(&tree), serde_json::to_vec_pretty(&s.tree)?)?;
        screens.push(ScreenRef { image, tree });
        images.push(Image::decode(&s.png)?);
    }
    let trees: Vec<&UiNode> = shots.iter().map(|s| &s.tree).collect();
    let mut spacing = spacing(&trees);
    spacing.corner_radius = corner_radius(&images, &trees, screen_w);
    let guide = Guide {
        app: app.name.clone(),
        bundle_id: app.bundle_id.clone(),
        captured_at: swiftwin_core::time::now_rfc3339(),
        screens,
        palette: palette(&images),
        type_scale: type_scale(&trees),
        spacing,
        components: components(&trees, screen_w),
        guardrail: GUARDRAIL.into(),
        privacy: PRIVACY.into(),
    };
    std::fs::write(dir.join("overview.png"), overview(&images))?;
    std::fs::write(dir.join("inspo.json"), serde_json::to_vec_pretty(&guide)?)?;
    std::fs::write(dir.join("INSPO.md"), guide.markdown())?;
    std::fs::write(dir.join("InspoTheme.swift"), guide.swift())?;
    Ok(guide)
}

// ---- palette ----

fn hex(c: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0].round() as u8, c[1].round() as u8, c[2].round() as u8)
}

fn from_hex(h: &str) -> Rgb {
    let v = u32::from_str_radix(h.trim_start_matches('#'), 16).unwrap_or(0);
    [(v >> 16 & 255) as f64, (v >> 8 & 255) as f64, (v & 255) as f64]
}

fn dist(a: Rgb, b: Rgb) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn luminance(c: Rgb) -> f64 {
    let ch = |v: f64| {
        let v = v / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2])
}

/// WCAG contrast ratio (1..21).
fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn saturation(c: Rgb) -> f64 {
    let (mx, mn) = (c.iter().cloned().fold(0.0, f64::max), c.iter().cloned().fold(255.0, f64::min));
    if mx == 0.0 { 0.0 } else { (mx - mn) / mx }
}

/// Colour clusters over all screenshots (status bar skipped), biggest first: (colour, share).
fn clusters(images: &[Image]) -> Vec<(Rgb, f64)> {
    let mut buckets: BTreeMap<u32, (f64, Rgb)> = BTreeMap::new();
    let mut total = 0.0_f64;
    for img in images {
        let step = ((img.w as u64 * img.h as u64 / 150_000) as f64).sqrt().max(1.0) as u32;
        for y in (img.h * 6 / 100..img.h).step_by(step as usize) {
            for x in (0..img.w).step_by(step as usize) {
                let i = ((y * img.w + x) * 4) as usize;
                let p = [img.rgba[i] as f64, img.rgba[i + 1] as f64, img.rgba[i + 2] as f64];
                let key = (img.rgba[i] as u32 >> 3) << 10 | (img.rgba[i + 1] as u32 >> 3) << 5 | img.rgba[i + 2] as u32 >> 3;
                let b = buckets.entry(key).or_insert((0.0, [0.0; 3]));
                b.0 += 1.0;
                (0..3).for_each(|k| b.1[k] += p[k]);
                total += 1.0;
            }
        }
    }
    let mut sorted: Vec<(f64, Rgb)> = buckets.into_values().map(|(n, s)| (n, [s[0] / n, s[1] / n, s[2] / n])).collect();
    sorted.sort_by(|a, b| b.0.total_cmp(&a.0));
    // Merge near colours (anti-aliasing, gradients) into the bigger cluster.
    let mut out: Vec<(Rgb, f64)> = Vec::new();
    for (n, c) in sorted.into_iter().take(600) {
        match out.iter_mut().find(|(o, _)| dist(*o, c) < 10.0) {
            Some(o) => o.1 += n,
            None => out.push((c, n)),
        }
    }
    out.iter_mut().for_each(|o| o.1 /= total.max(1.0));
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    out
}

/// Most common colour in the left and right 2% of the screens (below the status bar, above the tab bar).
fn edge_colour(images: &[Image]) -> Option<Rgb> {
    let mut counts: BTreeMap<u32, (usize, Rgb)> = BTreeMap::new();
    for img in images {
        let edge = (img.w / 50).max(1);
        for y in (img.h * 6 / 100..img.h * 88 / 100).step_by(3) {
            for x in (0..edge).chain(img.w - edge..img.w) {
                let i = ((y * img.w + x) * 4) as usize;
                let key = (img.rgba[i] as u32 >> 3) << 10 | (img.rgba[i + 1] as u32 >> 3) << 5 | img.rgba[i + 2] as u32 >> 3;
                counts.entry(key).or_insert((0, [img.rgba[i] as f64, img.rgba[i + 1] as f64, img.rgba[i + 2] as f64])).0 += 1;
            }
        }
    }
    counts.into_values().max_by_key(|c| c.0).map(|c| c.1)
}

pub fn palette(images: &[Image]) -> Palette {
    let cl = clusters(images);
    let Some(&(biggest, _)) = cl.first() else { return Palette::default() };
    // The background shows at the screen's side edges (cards and cells may cover more area).
    let edge = edge_colour(images).unwrap_or(biggest);
    let bg = cl.iter().map(|c| c.0).min_by(|a, b| dist(*a, edge).total_cmp(&dist(*b, edge))).unwrap_or(biggest);
    let visible: Vec<(Rgb, f64)> = cl.iter().cloned().filter(|c| c.1 >= 0.001 && c.0 != bg).collect();
    let text = visible
        .iter()
        .find(|c| contrast(c.0, bg) >= 4.5 && saturation(c.0) < 0.25)
        .or_else(|| visible.iter().find(|c| contrast(c.0, bg) >= 4.5))
        .or_else(|| visible.iter().max_by(|a, b| contrast(a.0, bg).total_cmp(&contrast(b.0, bg))))
        .map(|c| c.0)
        .unwrap_or(bg);
    let surface = visible
        .iter()
        .find(|c| contrast(c.0, bg) < 1.6 && saturation(c.0) < 0.35 && c.1 >= 0.01)
        .map(|c| c.0)
        .unwrap_or_else(|| std::array::from_fn(|k| bg[k] * 0.94 + text[k] * 0.06));
    // The accent is the vivid colour seen on the most screens (artwork and promo cards change from
    // screen to screen; the brand colour of chips, icons and progress bars does not), then the most used.
    let per_screen: Vec<Vec<(Rgb, f64)>> = images.iter().map(|i| clusters(std::slice::from_ref(i))).collect();
    let screens_with = |c: Rgb| per_screen.iter().filter(|cl| cl.iter().any(|o| o.1 >= 0.0005 && dist(o.0, c) < 30.0)).count();
    let accent = visible
        .iter()
        .filter(|c| saturation(c.0) >= 0.45 && c.0.iter().cloned().fold(0.0_f64, f64::max) >= 110.0)
        .take(12)
        .max_by(|a, b| (screens_with(a.0), a.1).partial_cmp(&(screens_with(b.0), b.1)).unwrap_or(std::cmp::Ordering::Equal))
        .map(|c| hex(c.0));
    Palette {
        background: hex(bg),
        surface: hex(surface),
        text: hex(text),
        accent,
        dominant: cl.iter().filter(|c| c.1 >= 0.005).take(8).map(|c| Swatch { hex: hex(c.0), share: (c.1 * 1000.0).round() / 1000.0 }).collect(),
    }
}

// ---- type scale, spacing, corners, components (from the UI trees) ----

fn is_text(n: &UiNode) -> bool {
    n.kind == "XCUIElementTypeStaticText" && n.label.as_deref().is_some_and(|l| !l.trim().is_empty())
}

/// A text element's frame is about 1.2 x its font size (single line); taller ones are multi-line.
pub fn type_scale(trees: &[&UiNode]) -> Vec<TypeSize> {
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for n in trees.iter().flat_map(|t| t.walk()).filter(|n| is_text(n) && (8.0..=60.0).contains(&n.rect.height)) {
        *counts.entry((n.rect.height / 1.2).round() as u32).or_default() += 1;
    }
    let mut sizes: Vec<TypeSize> = counts.into_iter().map(|(s, count)| TypeSize { size: s as f64, count }).collect();
    // The six most used sizes, plus the largest one seen at least twice (titles are rare but matter).
    let largest = sizes.iter().filter(|s| s.count >= 2).max_by(|a, b| a.size.total_cmp(&b.size)).cloned();
    sizes.sort_by_key(|s| std::cmp::Reverse(s.count));
    sizes.truncate(6);
    if let Some(l) = largest.filter(|l| !sizes.contains(l)) {
        sizes.pop();
        sizes.push(l);
    }
    sizes.sort_by(|a, b| b.size.total_cmp(&a.size));
    sizes
}

fn mode(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut counts: BTreeMap<i64, usize> = BTreeMap::new();
    values.for_each(|v| *counts.entry(v.round() as i64).or_default() += 1);
    counts.into_iter().max_by_key(|&(v, n)| (n, -v)).map(|(v, _)| v as f64)
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    v.sort_by(|a, b| a.total_cmp(b));
    v.get(v.len() / 2).copied()
}

fn content(n: &UiNode) -> bool {
    is_text(n) || matches!(n.kind.as_str(), "XCUIElementTypeButton" | "XCUIElementTypeCell" | "XCUIElementTypeTextField" | "XCUIElementTypeSearchField")
}

pub fn spacing(trees: &[&UiNode]) -> Spacing {
    let nodes: Vec<Vec<&UiNode>> = trees.iter().map(|t| t.walk().into_iter().filter(|n| content(n)).collect()).collect();
    let all = || nodes.iter().flatten();
    let padding = mode(all().map(|n| n.rect.x).filter(|x| (4.0..=40.0).contains(x))).unwrap_or(16.0);
    let mut gaps = Vec::new();
    for screen in &nodes {
        let mut s: Vec<&Rect> = screen.iter().map(|n| &n.rect).collect();
        s.sort_by(|a, b| a.y.total_cmp(&b.y));
        for (a, b) in s.iter().zip(s.iter().skip(1)) {
            let overlap = a.x < b.x + b.width && b.x < a.x + a.width;
            let gap = b.y - (a.y + a.height);
            if overlap && (2.0..=40.0).contains(&gap) {
                gaps.push(gap);
            }
        }
    }
    let rows = all().filter(|n| n.kind == "XCUIElementTypeCell" && n.rect.height < 200.0).map(|n| n.rect.height.round()).collect();
    Spacing { screen_padding: padding, stack: mode(gaps.into_iter()).unwrap_or(8.0), row_height: median(rows), corner_radius: None }
}

/// Measure corner radii of filled buttons and cells: walk the top-left diagonal until the
/// fill colour starts; a circle corner of radius r reaches the diagonal at r·(1 − 1/√2).
pub fn corner_radius(images: &[Image], trees: &[&UiNode], screen_w: f64) -> Option<f64> {
    let mut radii = Vec::new();
    for (img, tree) in images.iter().zip(trees) {
        let scale = if screen_w > 0.0 { img.w as f64 / screen_w } else { 1.0 };
        let px = |x: f64, y: f64| -> Option<Rgb> {
            let (x, y) = ((x * scale) as i64, (y * scale) as i64);
            (x >= 0 && y >= 0 && (x as u32) < img.w && (y as u32) < img.h).then(|| {
                let i = ((y as u32 * img.w + x as u32) * 4) as usize;
                [img.rgba[i] as f64, img.rgba[i + 1] as f64, img.rgba[i + 2] as f64]
            })
        };
        for n in tree.walk() {
            let r = n.rect;
            if !matches!(n.kind.as_str(), "XCUIElementTypeButton" | "XCUIElementTypeCell") || !(28.0..=200.0).contains(&r.height) {
                continue;
            }
            let (Some(fill), Some(outside)) = (px(r.x + r.width / 2.0, r.y + 2.0), px(r.x + r.width / 2.0, r.y - 3.0)) else { continue };
            if dist(fill, outside) < 20.0 {
                continue; // no visible shape (plain text button)
            }
            let steps = (r.height / 2.0 * scale) as usize;
            if let Some(d) = (0..steps).find(|&i| px(r.x + i as f64 / scale, r.y + i as f64 / scale).is_some_and(|p| dist(p, fill) < dist(p, outside))) {
                radii.push((d as f64 / scale / (1.0 - std::f64::consts::FRAC_1_SQRT_2)).min(r.height / 2.0));
            }
        }
    }
    median(radii).map(f64::round)
}

pub fn components(trees: &[&UiNode], screen_w: f64) -> Vec<Component> {
    type Is<'a> = dyn Fn(&UiNode) -> bool + 'a;
    let kinds: [(&str, &Is<'_>); 10] = [
        ("Tab bar", &|n| n.kind == "XCUIElementTypeTabBar"),
        ("Navigation bar", &|n| n.kind == "XCUIElementTypeNavigationBar"),
        ("Search field", &|n| n.kind == "XCUIElementTypeSearchField"),
        ("List rows / cells", &|n| n.kind == "XCUIElementTypeCell"),
        ("Cards / tiles", &|n| {
            matches!(n.kind.as_str(), "XCUIElementTypeCell" | "XCUIElementTypeOther") && n.rect.height >= 80.0 && n.rect.width >= 80.0 && n.rect.width < screen_w * 0.7
        }),
        ("Grid / carousel", &|n| n.kind == "XCUIElementTypeCollectionView"),
        ("Buttons", &|n| n.kind == "XCUIElementTypeButton"),
        ("Text fields", &|n| matches!(n.kind.as_str(), "XCUIElementTypeTextField" | "XCUIElementTypeSecureTextField")),
        ("Toggles", &|n| n.kind == "XCUIElementTypeSwitch"),
        ("Segmented control", &|n| n.kind == "XCUIElementTypeSegmentedControl"),
    ];
    kinds
        .iter()
        .map(|(name, f)| Component { name: name.to_string(), screens: trees.iter().filter(|t| t.walk().iter().any(|n| f(n))).count() })
        .filter(|c| c.screens > 0)
        .collect()
}

// ---- outputs ----

impl Guide {
    /// Largest size, if it looks like a title (else 28).
    pub fn title_size(&self) -> f64 {
        self.type_scale.first().map(|t| t.size).filter(|&s| s >= 20.0).unwrap_or(28.0)
    }
    /// Most used size below the title (else 17).
    pub fn body_size(&self) -> f64 {
        let t = self.title_size();
        self.type_scale.iter().filter(|s| s.size < t).max_by_key(|s| (s.count, -(s.size as i64))).map(|s| s.size).unwrap_or(17.0)
    }
    /// Smallest size below the body (else body − 4).
    pub fn caption_size(&self) -> f64 {
        let b = self.body_size();
        self.type_scale.iter().rfind(|s| s.size < b).map(|s| s.size).unwrap_or((b - 4.0).max(11.0))
    }

    /// A short summary for chat, the MCP tool and the dialog.
    pub fn summary(&self) -> String {
        let p = &self.palette;
        format!(
            "{} screens of {}. Palette: background {}, surface {}, text {}, accent {}. Type: title {}pt, body {}pt, caption {}pt. \
             Spacing: padding {}, stack {}, rows {}, corners {}. Patterns: {}.",
            self.screens.len(),
            self.app,
            p.background,
            p.surface,
            p.text,
            p.accent.as_deref().unwrap_or("none"),
            self.title_size(),
            self.body_size(),
            self.caption_size(),
            self.spacing.screen_padding,
            self.spacing.stack,
            self.spacing.row_height.map(|v| v.to_string()).unwrap_or("-".into()),
            self.spacing.corner_radius.map(|v| v.to_string()).unwrap_or("-".into()),
            self.components.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")
        )
    }

    /// Pre-filled AI-mode message. `rel_dir` is the capture folder relative to the project.
    pub fn claude_prompt(&self, rel_dir: &str) -> String {
        format!(
            "Restyle my app using the Inspo style guide in {rel_dir}/. Be thorough, this is the whole point:\n\
             1. Read INSPO.md and inspo.json.\n\
             2. Look at {rel_dir}/overview.png first: it shows all {n} captured screens in one grid (01 top-left, row by \
             row). Then open every screenshot, {rel_dir}/01.png to {rel_dir}/{n:02}.png, not just a couple, and note how \
             headers, lists, cards, carousels, buttons, search, spacing and typography are designed. The NN.json files \
             have each screen's exact element frames.\n\
             3. Add {rel_dir}/InspoTheme.swift to Sources/ if it is not there yet, and use its colours, font sizes, \
             spacing and corner radius in every view of my app; use the layout patterns you saw ({}) where they fit my \
             app's content.\n\
             4. {GUARDRAIL} Keep my app's own content and features.\n\
             5. Call install_run, then screenshot (scroll and tap through every screen of my app) and compare it with \
             the inspiration screens; fix what looks off and run again until it holds up next to them.",
            self.components.iter().map(|c| c.name.to_lowercase()).collect::<Vec<_>>().join(", "),
            n = self.screens.len()
        )
    }

    pub fn markdown(&self) -> String {
        let p = &self.palette;
        let mut m = format!(
            "# Inspo style guide\n\nCaptured from **{}** ({}) on {}, {} screens.\n\n> {}\n>\n> {}\n\n## Palette\n\n| Role | Colour |\n|---|---|\n\
             | Background | `{}` |\n| Surface | `{}` |\n| Text | `{}` |\n| Accent | `{}` |\n\nMost used: {}\n\n## Type scale\n\n",
            self.app,
            self.bundle_id,
            self.captured_at,
            self.screens.len(),
            GUARDRAIL,
            PRIVACY,
            p.background,
            p.surface,
            p.text,
            p.accent.as_deref().unwrap_or("none"),
            p.dominant.iter().map(|s| format!("`{}` ({:.0}%)", s.hex, s.share * 100.0)).collect::<Vec<_>>().join(", ")
        );
        for t in &self.type_scale {
            m.push_str(&format!("- {}pt ({} texts)\n", t.size, t.count));
        }
        let s = &self.spacing;
        m.push_str(&format!(
            "\n## Spacing\n\n- Screen padding: {}pt\n- Stack spacing: {}pt\n- Row height: {}\n- Corner radius (estimate): {}\n\n## Patterns\n\n",
            s.screen_padding,
            s.stack,
            s.row_height.map(|v| format!("{v}pt")).unwrap_or("-".into()),
            s.corner_radius.map(|v| format!("{v}pt")).unwrap_or("-".into())
        ));
        for c in &self.components {
            m.push_str(&format!("- {} (on {} of {} screens)\n", c.name, c.screens, self.screens.len()));
        }
        m.push_str("\n## Screens\n\n");
        for s in &self.screens {
            m.push_str(&format!("- `{}` (UI tree: `{}`)\n", s.image, s.tree));
        }
        m
    }

    /// `InspoTheme.swift`: plain SwiftUI values (no macros, which the Windows toolchain cannot compile).
    pub fn swift(&self) -> String {
        let color = |h: &str| {
            let c = from_hex(h);
            format!("Color(red: {:.3}, green: {:.3}, blue: {:.3})", c[0] / 255.0, c[1] / 255.0, c[2] / 255.0)
        };
        let p = &self.palette;
        let s = &self.spacing;
        let (t, b, c) = (self.title_size(), self.body_size(), self.caption_size());
        format!(
            "import SwiftUI\n\n\
             // Generated by SwiftWin Inspo from another app's look: palette, type scale and spacing only.\n\
             // {GUARDRAIL}\n\
             enum InspoTheme {{\n\
             \x20   // Colours\n\
             \x20   static let background = {}\n\
             \x20   static let surface = {}\n\
             \x20   static let text = {}\n\
             \x20   static let accent = {}\n\n\
             \x20   // Type scale (points)\n\
             \x20   static let titleSize: CGFloat = {t}\n\
             \x20   static let bodySize: CGFloat = {b}\n\
             \x20   static let captionSize: CGFloat = {c}\n\
             \x20   static let title = Font.system(size: {t}, weight: .bold)\n\
             \x20   static let body = Font.system(size: {b})\n\
             \x20   static let caption = Font.system(size: {c})\n\n\
             \x20   // Spacing (points)\n\
             \x20   static let padding: CGFloat = {}\n\
             \x20   static let spacing: CGFloat = {}\n\
             \x20   static let rowHeight: CGFloat = {}\n\
             \x20   static let cornerRadius: CGFloat = {}\n\
             }}\n",
            color(&p.background),
            color(&p.surface),
            color(&p.text),
            p.accent.as_deref().map(color).unwrap_or_else(|| "Color.accentColor".into()),
            s.screen_padding,
            s.stack,
            s.row_height.unwrap_or(44.0),
            s.corner_radius.unwrap_or(12.0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixture "screenshot": white page, light-grey cards with 12pt corners, black text, blue accent bar.
    fn fixture() -> (Image, UiNode) {
        let (w, h) = (390u32, 844u32);
        let mut rgba = vec![255u8; (w * h * 4) as usize];
        let mut fill = |x0: u32, y0: u32, x1: u32, y1: u32, r: f64, c: [u8; 3]| {
            for y in y0..y1 {
                for x in x0..x1 {
                    // Round the corners: skip pixels outside the corner circles.
                    let (cx, cy) = ((x as f64 + 0.5).clamp(x0 as f64 + r, x1 as f64 - r), (y as f64 + 0.5).clamp(y0 as f64 + r, y1 as f64 - r));
                    if ((x as f64 + 0.5 - cx).powi(2) + (y as f64 + 0.5 - cy).powi(2)).sqrt() > r {
                        continue;
                    }
                    let i = ((y * w + x) * 4) as usize;
                    rgba[i..i + 3].copy_from_slice(&c);
                }
            }
        };
        for k in 0..4 {
            fill(16, 200 + k * 120, 374, 300 + k * 120, 12.0, [242, 242, 247]);
            fill(32, 230 + k * 120, 200, 250 + k * 120, 0.0, [0, 0, 0]);
        }
        fill(16, 120, 374, 170, 12.0, [0, 122, 255]);
        let text = |l: &str, y: f64, h: f64| UiNode::new("XCUIElementTypeStaticText", Rect::new(16.0, y, 300.0, h)).with_label(l);
        let mut kids = vec![
            text("Title", 60.0, 41.0),
            UiNode::new("XCUIElementTypeButton", Rect::new(16.0, 120.0, 358.0, 50.0)).with_label("Go"),
            text("Body", 176.0, 20.33),
        ];
        for k in 0..4 {
            let y = 200.0 + k as f64 * 120.0;
            kids.push(UiNode::new("XCUIElementTypeCell", Rect::new(16.0, y, 358.0, 100.0)).with_label(format!("Row {k}")));
            kids.push(text(&format!("Row text {k}"), y + 20.0, 20.33));
            kids.push(text(&format!("Caption {k}"), y + 50.0, 15.67));
        }
        let tree = UiNode::new("XCUIElementTypeApplication", Rect::new(0.0, 0.0, 390.0, 844.0)).with_label("Fixture").with_children(kids);
        (Image { w, h, rgba }, tree)
    }

    #[test]
    fn palette_from_fixture() {
        let (img, _) = fixture();
        let p = palette(&[img]);
        assert_eq!(p.background, "#FFFFFF");
        assert_eq!(p.surface, "#F2F2F7");
        assert_eq!(p.text, "#000000");
        assert_eq!(p.accent.as_deref(), Some("#007AFF"));
        assert!(p.dominant[0].share > 0.3);
    }

    /// Run on a real capture: `INSPO_DIR=<project>/.inspo/<app> cargo test -p swiftwin-inspo real_capture -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_capture() {
        let dir = std::path::PathBuf::from(std::env::var("INSPO_DIR").unwrap());
        let mut images = Vec::new();
        for i in 1..=crate::MAX_SCREENS_DEEP {
            if let Ok(png) = std::fs::read(dir.join(format!("{i:02}.png"))) {
                images.push(Image::decode(&png).unwrap());
            }
        }
        println!("{:#?}", palette(&images));
        std::fs::write(dir.join("overview.png"), overview(&images)).unwrap();
    }

    #[test]
    fn type_scale_spacing_and_corners_from_fixture() {
        let (img, tree) = fixture();
        let trees = [&tree];
        assert_eq!(type_scale(&trees), vec![
            TypeSize { size: 34.0, count: 1 },
            TypeSize { size: 17.0, count: 5 },
            TypeSize { size: 13.0, count: 4 },
        ]);
        let s = spacing(&trees);
        assert_eq!((s.screen_padding, s.row_height), (16.0, Some(100.0)));
        let r = corner_radius(&[img], &trees, 390.0).unwrap();
        assert!((10.0..=14.0).contains(&r), "radius {r}");
        let names: Vec<String> = components(&trees, 390.0).into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["List rows / cells", "Buttons"]);
    }

    #[test]
    fn swift_theme_has_values_and_the_guardrail() {
        let (img, tree) = fixture();
        let trees = [&tree];
        let g = Guide {
            app: "Fixture".into(),
            bundle_id: "com.x".into(),
            captured_at: "now".into(),
            screens: vec![],
            palette: palette(&[img]),
            type_scale: type_scale(&trees),
            spacing: spacing(&trees),
            components: components(&trees, 390.0),
            guardrail: GUARDRAIL.into(),
            privacy: PRIVACY.into(),
        };
        let swift = g.swift();
        assert!(swift.contains("static let accent = Color(red: 0.000, green: 0.478, blue: 1.000)"), "{swift}");
        assert!(swift.contains("static let body = Font.system(size: 17)"));
        assert!(swift.contains("Do not copy") && !swift.contains('@') && !swift.contains("Fixture"));
        assert_eq!((g.title_size(), g.body_size(), g.caption_size()), (34.0, 17.0, 13.0));
        let one = Guide { type_scale: vec![TypeSize { size: 34.0, count: 3 }], ..g.clone() };
        assert_eq!((one.title_size(), one.body_size(), one.caption_size()), (34.0, 17.0, 13.0), "one size: defaults below it");
        assert!(g.claude_prompt(".inspo/Fixture").contains("Do not copy the other app's logo"));
        assert!(g.markdown().contains("personal data"));
    }
}
