use serde::{Deserialize, Serialize};

/// How the phone is attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Connection {
    Usb,
    Network,
    Mock,
}

/// Logical screen size in points plus the pixel scale (3.0 on most iPhones).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScreenSpec {
    pub width_pts: f64,
    pub height_pts: f64,
    pub scale: f64,
}

impl ScreenSpec {
    /// Convert a pixel position in a screenshot into device points.
    pub fn pixel_to_point(&self, px: f64, py: f64) -> Point {
        Point { x: px / self.scale, y: py / self.scale }
    }

    /// Map a point inside a displayed image of size (`view_w`,`view_h`) to device points.
    pub fn view_to_point(&self, vx: f64, vy: f64, view_w: f64, view_h: f64) -> Point {
        Point {
            x: (vx / view_w * self.width_pts).clamp(0.0, self.width_pts),
            y: (vy / view_h * self.height_pts).clamp(0.0, self.height_pts),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub udid: String,
    pub name: String,
    /// Apple product type, e.g. `iPhone16,2`.
    pub model: String,
    pub ios_version: String,
    pub connection: Connection,
    /// True when this computer holds a pairing record ("Trust" was tapped).
    pub paired: bool,
    /// `None` when it could not be determined yet.
    pub developer_mode: Option<bool>,
    pub screen: Option<ScreenSpec>,
}

impl DeviceInfo {
    /// Parsed iOS major version (`17` for "17.4.1"), if known.
    pub fn ios_major(&self) -> Option<u32> {
        self.ios_version.split('.').next()?.parse().ok()
    }
}

/// A PNG screenshot plus its pixel dimensions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Screenshot {
    #[serde(with = "base64_bytes")]
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl Screenshot {
    /// Build from PNG bytes, reading width/height from the IHDR chunk.
    pub fn from_png(png: Vec<u8>) -> crate::Result<Self> {
        let (width, height) = png_dimensions(&png)
            .ok_or_else(|| crate::SwiftWinError::protocol("screenshot is not a valid PNG"))?;
        Ok(Screenshot { png, width, height })
    }
}

/// Read width/height from a PNG header without decoding.
pub fn png_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if data.len() < 24 || data[..8] != SIG || &data[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(data[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(data[20..24].try_into().ok()?);
    Some((w, h))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    #[default]
    Info,
    Notice,
    Error,
    Fault,
}

impl LogLevel {
    pub fn parse(s: &str) -> LogLevel {
        match s.to_ascii_lowercase().as_str() {
            "debug" => LogLevel::Debug,
            "notice" | "default" => LogLevel::Notice,
            "error" => LogLevel::Error,
            "fault" => LogLevel::Fault,
            _ => LogLevel::Info,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub process: String,
    pub pid: u32,
    pub level: LogLevel,
    pub message: String,
    /// os_log subsystem, e.g. the app's bundle id with `Logger(subsystem:category:)`, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsystem: Option<String>,
}

/// Filter for reading device logs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LogQuery {
    /// Only entries from this process name (exact, case-insensitive).
    pub process: Option<String>,
    /// Only entries whose message contains this text (case-insensitive).
    pub contains: Option<String>,
    pub min_level: Option<LogLevel>,
    /// Keep streaming new entries instead of returning after the backlog.
    pub follow: bool,
    /// Return at most this many backlog entries (newest).
    pub limit: Option<usize>,
    /// Only entries whose os_log subsystem starts with this (case-insensitive).
    #[serde(default)]
    pub subsystem: Option<String>,
    /// Only entries from this log sequence number on (`LogStatus::next` of an earlier read).
    #[serde(default)]
    pub since: Option<u64>,
}

impl LogQuery {
    pub fn matches(&self, e: &LogEntry) -> bool {
        if let Some(p) = &self.process {
            if !e.process.eq_ignore_ascii_case(p) {
                return false;
            }
        }
        if let Some(c) = &self.contains {
            if !e.message.to_lowercase().contains(&c.to_lowercase()) {
                return false;
            }
        }
        if let Some(l) = self.min_level {
            if e.level < l {
                return false;
            }
        }
        if let Some(sub) = &self.subsystem {
            let sub = sub.to_lowercase();
            if !e.subsystem.as_deref().is_some_and(|s| s.to_lowercase().starts_with(&sub)) {
                return false;
            }
        }
        true
    }
}

/// An alert on screen (system or app), as WebDriverAgent reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alert {
    pub text: String,
    pub buttons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Rect { x, y, width, height }
    }
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.x + self.width && p.y < self.y + self.height
    }
    pub fn center(&self) -> Point {
        Point { x: self.x + self.width / 2.0, y: self.y + self.height / 2.0 }
    }
}

/// One element of the on-screen accessibility tree (WebDriverAgent JSON shape,
/// simplified). `kind` keeps WDA's names, e.g. `XCUIElementTypeButton`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiNode {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub rect: Rect,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<UiNode>,
}

fn yes() -> bool {
    true
}

impl UiNode {
    pub fn new(kind: &str, rect: Rect) -> Self {
        UiNode {
            kind: kind.to_string(),
            label: None,
            name: None,
            value: None,
            rect,
            enabled: true,
            children: vec![],
        }
    }
    pub fn with_label(mut self, l: impl Into<String>) -> Self {
        let l = l.into();
        self.name = Some(l.clone());
        self.label = Some(l);
        self
    }
    pub fn with_value(mut self, v: impl Into<String>) -> Self {
        self.value = Some(v.into());
        self
    }
    pub fn with_children(mut self, c: Vec<UiNode>) -> Self {
        self.children = c;
        self
    }

    /// Depth-first iterator over this node and its descendants.
    pub fn walk(&self) -> Vec<&UiNode> {
        let mut out = vec![self];
        for c in &self.children {
            out.extend(c.walk());
        }
        out
    }

    /// The on-screen keyboard's frame, if a keyboard is showing.
    pub fn keyboard(&self, w: f64, h: f64) -> Option<Rect> {
        self.walk().into_iter().find(|n| n.kind.ends_with("Keyboard") && n.on_screen(w, h)).map(|n| n.rect)
    }

    /// On-screen nodes whose label or value contains `text` (case-insensitive).
    pub fn containing(&self, text: &str, w: f64, h: f64) -> Vec<&UiNode> {
        let t = text.to_lowercase();
        self.walk()
            .into_iter()
            .filter(|n| n.on_screen(w, h) && [&n.label, &n.value].iter().any(|v| v.as_deref().is_some_and(|s| s.to_lowercase().contains(&t))))
            .collect()
    }

    /// One `ui_tree`-style line for this node (no children).
    pub fn line(&self) -> String {
        let mut s = String::new();
        self.write_line(0, &mut s);
        s.trim_end().to_string()
    }

    /// First node whose label, name or value equals `text` (case-insensitive).
    pub fn find(&self, text: &str) -> Option<&UiNode> {
        self.walk().into_iter().find(|n| n.is_labelled(text))
    }

    fn is_labelled(&self, text: &str) -> bool {
        [&self.label, &self.name, &self.value].iter().any(|v| v.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(text)))
    }

    /// Is any part of this element inside a `w` x `h` point screen? Elements without a size are not.
    pub fn on_screen(&self, w: f64, h: f64) -> bool {
        let r = &self.rect;
        r.width > 0.0 && r.height > 0.0 && r.x < w && r.y < h && r.x + r.width > 0.0 && r.y + r.height > 0.0
    }

    /// Centre of the part of this element that is on a `w` x `h` screen.
    pub fn visible_center(&self, w: f64, h: f64) -> Point {
        let r = &self.rect;
        let (x0, y0) = (r.x.max(0.0), r.y.max(0.0));
        let (x1, y1) = ((r.x + r.width).min(w), (r.y + r.height).min(h));
        Point { x: (x0 + x1) / 2.0, y: (y0 + y1) / 2.0 }
    }

    /// The element to tap for `text` on a `w` x `h` screen: an on-screen element whose label,
    /// name or value equals `text`, preferring controls, and the front-most (last) one.
    /// `Err(Some(node))` when only off-screen or size-less elements match, `Err(None)` when none do.
    pub fn find_visible(&self, text: &str, w: f64, h: f64) -> std::result::Result<&UiNode, Option<&UiNode>> {
        let all: Vec<&UiNode> = self.walk().into_iter().filter(|n| n.is_labelled(text)).collect();
        let visible: Vec<&UiNode> = all.iter().copied().filter(|n| n.on_screen(w, h)).collect();
        if let Some(n) = visible.iter().rev().find(|n| n.is_control()).or_else(|| visible.last()) {
            return Ok(n);
        }
        Err(all.first().copied())
    }

    fn is_control(&self) -> bool {
        const CONTROLS: &[&str] = &[
            "Button", "Cell", "Switch", "Toggle", "TextField", "SecureTextField", "SearchField", "TextView", "Link",
            "Tab", "MenuItem", "Key", "Icon", "Slider", "Stepper", "SegmentedControl", "PickerWheel", "Image",
        ];
        let short = self.kind.strip_prefix("XCUIElementType").unwrap_or(&self.kind);
        CONTROLS.contains(&short)
    }

    /// `to_text` for a `w` x `h` screen, trimmed for reading: leaves out off-screen elements,
    /// unlabelled containers (their children move up a level) and text that repeats its parent.
    pub fn to_compact_text(&self, w: f64, h: f64) -> String {
        let mut s = String::new();
        self.write_compact(0, None, w, h, true, &mut s);
        s
    }

    /// `parent` is the nearest ancestor that was written.
    fn write_compact(&self, depth: usize, parent: Option<&UiNode>, w: f64, h: f64, root: bool, out: &mut String) {
        let sized = self.rect.width > 0.0 && self.rect.height > 0.0;
        if sized && !root && !self.on_screen(w, h) {
            return;
        }
        let short = self.kind.strip_prefix("XCUIElementType").unwrap_or(&self.kind);
        let unlabelled = self.label.is_none() && self.value.is_none();
        let container = matches!(short, "Other" | "Group" | "Window" | "Any" | "LayoutArea" | "LayoutItem");
        let parent_label = parent.and_then(|p| p.label.as_deref());
        let repeats_parent = short == "StaticText" && self.label.is_some() && self.label.as_deref() == parent_label;
        // SwiftUI nests an element in copies of itself (a ProgressView shows as 3 ProgressIndicators,
        // the inner ones unlabelled or labelled just "Progress"): same kind and value adds nothing.
        let same_as_parent = parent.is_some_and(|p| {
            p.kind == self.kind && p.value == self.value && (self.label.is_none() || self.label == p.label || short == "ProgressIndicator")
        });
        // A NavigationLink card is a Button holding a Button with the same frame: one tap target.
        let nested_button = short == "Button" && parent.is_some_and(|p| p.kind == self.kind && p.rect == self.rect);
        let skip_self = !root && (!sized || (unlabelled && container) || repeats_parent || same_as_parent || nested_button);
        let (next_depth, next_parent) = if skip_self {
            (depth, parent)
        } else {
            let mut line = String::new();
            self.write_line(depth, &mut line);
            out.push_str(&line);
            (depth + 1, Some(self))
        };
        for c in &self.children {
            c.write_compact(next_depth, next_parent, w, h, false, out);
        }
    }

    fn write_line(&self, depth: usize, out: &mut String) {
        use std::fmt::Write;
        let short = self.kind.strip_prefix("XCUIElementType").unwrap_or(&self.kind);
        let _ = write!(out, "{}{}", "  ".repeat(depth), short);
        if let Some(l) = &self.label {
            let _ = write!(out, " \"{l}\"");
        }
        if let Some(v) = &self.value {
            if Some(v) != self.label.as_ref() {
                let _ = write!(out, " value=\"{v}\"");
            }
        }
        let r = &self.rect;
        let _ = write!(out, " [{:.0},{:.0} {:.0}x{:.0}]", r.x, r.y, r.width, r.height);
        if !self.enabled {
            out.push_str(" disabled");
        }
        out.push('\n');
    }

    /// Compact indented text form, handy for terminals and for LLMs.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        self.write_text(0, &mut s);
        s
    }

    fn write_text(&self, depth: usize, out: &mut String) {
        use std::fmt::Write;
        let short = self.kind.strip_prefix("XCUIElementType").unwrap_or(&self.kind);
        let _ = write!(out, "{}{}", "  ".repeat(depth), short);
        if let Some(l) = &self.label {
            let _ = write!(out, " \"{l}\"");
        }
        if let Some(v) = &self.value {
            if Some(v) != self.label.as_ref() {
                let _ = write!(out, " value=\"{v}\"");
            }
        }
        let r = &self.rect;
        let _ = write!(out, " [{:.0},{:.0} {:.0}x{:.0}]", r.x, r.y, r.width, r.height);
        if !self.enabled {
            out.push_str(" disabled");
        }
        out.push('\n');
        for c in &self.children {
            c.write_text(depth + 1, out);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    Home,
    VolumeUp,
    VolumeDown,
}

impl Button {
    /// Name used by WebDriverAgent's `/wda/pressButton`.
    pub fn wda_name(&self) -> &'static str {
        match self {
            Button::Home => "home",
            Button::VolumeUp => "volumeUp",
            Button::VolumeDown => "volumeDown",
        }
    }
}

impl std::str::FromStr for Button {
    type Err = crate::SwiftWinError;
    fn from_str(s: &str) -> crate::Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "home" => Ok(Button::Home),
            "volumeup" | "volume-up" | "volume_up" => Ok(Button::VolumeUp),
            "volumedown" | "volume-down" | "volume_down" => Ok(Button::VolumeDown),
            o => Err(crate::SwiftWinError::invalid(format!("unknown button {o:?}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    pub bundle_id: String,
    pub name: String,
    pub version: Option<String>,
}

pub mod base64_bytes {
    //! Serialize `Vec<u8>` as standard base64 (no external crate).
    use serde::{Deserialize, Deserializer, Serializer};

    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            out.push(A[(n >> 18) as usize & 63] as char);
            out.push(A[(n >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 { A[(n >> 6) as usize & 63] as char } else { '=' });
            out.push(if chunk.len() > 2 { A[n as usize & 63] as char } else { '=' });
        }
        out
    }

    pub fn decode(s: &str) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(s.len() / 4 * 3);
        let mut buf = 0u32;
        let mut bits = 0;
        for c in s.bytes() {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' | b'\n' | b'\r' | b' ' => continue,
                _ => return None,
            };
            buf = (buf << 6) | u32::from(v);
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
                buf &= (1 << bits) - 1;
            }
        }
        Some(out)
    }

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        decode(&s).ok_or_else(|| serde::de::Error::custom("invalid base64"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            let e = base64_bytes::encode(s.as_bytes());
            assert_eq!(base64_bytes::decode(&e).unwrap(), s.as_bytes());
        }
        assert_eq!(base64_bytes::encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_bytes::encode(b"fo"), "Zm8=");
    }

    #[test]
    fn view_mapping_clamps() {
        let s = ScreenSpec { width_pts: 393.0, height_pts: 852.0, scale: 3.0 };
        let p = s.view_to_point(196.5, 426.0, 393.0, 852.0);
        assert_eq!(p, Point { x: 196.5, y: 426.0 });
        let p = s.view_to_point(1000.0, -5.0, 393.0, 852.0);
        assert_eq!(p, Point { x: 393.0, y: 0.0 });
        assert_eq!(s.pixel_to_point(300.0, 600.0), Point { x: 100.0, y: 200.0 });
    }

    #[test]
    fn ui_tree_find_and_text() {
        let t = UiNode::new("XCUIElementTypeApplication", Rect::new(0.0, 0.0, 10.0, 10.0))
            .with_label("Hello")
            .with_children(vec![UiNode::new("XCUIElementTypeButton", Rect::new(1.0, 2.0, 3.0, 4.0))
                .with_label("Tap me")]);
        assert_eq!(t.find("tap ME").unwrap().kind, "XCUIElementTypeButton");
        assert!(t.to_text().contains("  Button \"Tap me\" [1,2 3x4]"));
        let json = serde_json::to_string(&t).unwrap();
        let back: UiNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn log_query_filters() {
        let e = LogEntry {
            timestamp: "t".into(),
            process: "Hello".into(),
            pid: 1,
            level: LogLevel::Error,
            message: "Boom happened".into(),
            subsystem: Some("com.example.Hello".into()),
        };
        assert!(LogQuery { subsystem: Some("com.example.hello".into()), ..Default::default() }.matches(&e));
        assert!(!LogQuery { subsystem: Some("com.other".into()), ..Default::default() }.matches(&e));
        let system = LogEntry { subsystem: None, ..e.clone() };
        assert!(!LogQuery { subsystem: Some("com.example.hello".into()), ..Default::default() }.matches(&system), "system lines have no app subsystem");
        assert!(LogQuery { process: Some("hello".into()), ..Default::default() }.matches(&e));
        assert!(!LogQuery { process: Some("other".into()), ..Default::default() }.matches(&e));
        assert!(LogQuery { contains: Some("boom".into()), ..Default::default() }.matches(&e));
        assert!(!LogQuery { min_level: Some(LogLevel::Fault), ..Default::default() }.matches(&e));
    }

    #[test]
    fn ios_major() {
        let mut i = DeviceInfo {
            udid: "x".into(),
            name: "n".into(),
            model: "m".into(),
            ios_version: "17.4.1".into(),
            connection: Connection::Mock,
            paired: true,
            developer_mode: None,
            screen: None,
        };
        assert_eq!(i.ios_major(), Some(17));
        i.ios_version = "garbage".into();
        assert_eq!(i.ios_major(), None);
    }

    /// A label that only exists off-screen (or with no size) must not be tapped at (0, 0).
    #[test]
    fn find_visible_prefers_on_screen_controls() {
        let mut root = UiNode::new("XCUIElementTypeApplication", Rect::new(0.0, 0.0, 400.0, 800.0));
        let mut hidden = UiNode::new("XCUIElementTypeButton", Rect::new(0.0, 0.0, 0.0, 0.0));
        hidden.label = Some("Settings".into());
        let mut below = UiNode::new("XCUIElementTypeButton", Rect::new(10.0, 900.0, 100.0, 40.0));
        below.label = Some("Footer".into());
        let mut text = UiNode::new("XCUIElementTypeStaticText", Rect::new(10.0, 100.0, 100.0, 20.0));
        text.label = Some("Save".into());
        let mut button = UiNode::new("XCUIElementTypeButton", Rect::new(10.0, 90.0, 120.0, 40.0));
        button.label = Some("Save".into());
        let mut empty = UiNode::new("XCUIElementTypeOther", Rect::new(0.0, 0.0, 400.0, 800.0));
        button.children = vec![text];
        empty.children = vec![button];
        let mut cut = UiNode::new("XCUIElementTypeCell", Rect::new(0.0, 780.0, 400.0, 100.0));
        cut.label = Some("Row".into());
        root.children = vec![hidden, below, empty, cut];
        assert!(matches!(root.find_visible("Settings", 400.0, 800.0), Err(Some(_))), "size-less match is not tappable");
        assert!(matches!(root.find_visible("Footer", 400.0, 800.0), Err(Some(_))));
        assert!(matches!(root.find_visible("Nope", 400.0, 800.0), Err(None)));
        let save = root.find_visible("save", 400.0, 800.0).unwrap();
        assert_eq!(save.kind, "XCUIElementTypeButton", "a control wins over its text");
        let row = root.find_visible("Row", 400.0, 800.0).unwrap();
        assert_eq!(row.visible_center(400.0, 800.0), Point { x: 200.0, y: 790.0 }, "tap the part that is on screen");
        let compact = root.to_compact_text(400.0, 800.0);
        assert!(compact.contains("Button \"Save\"") && compact.contains("Cell \"Row\""), "{compact}");
        assert!(!compact.contains("Footer") && !compact.contains("Settings") && !compact.contains("Other"), "{compact}");
        assert_eq!(compact.matches("Save").count(), 1, "text repeating its button is dropped: {compact}");
        // As seen on iOS 27: a labelled bar, an inner "Progress" bar and an unlabelled one.
        let bar = |label: Option<&str>, y: f64, children| UiNode {
            label: label.map(String::from),
            value: Some("0%".into()),
            children,
            ..UiNode::new("XCUIElementTypeProgressIndicator", Rect::new(144.0, y, 186.0, 24.0))
        };
        let mut screen = UiNode::new("XCUIElementTypeApplication", Rect::new(0.0, 0.0, 400.0, 800.0));
        screen.children = vec![bar(Some("History today 0"), 583.0, vec![bar(Some("Progress"), 590.0, vec![bar(None, 580.0, vec![])])])];
        let text = screen.to_compact_text(400.0, 800.0);
        assert_eq!(text.matches("ProgressIndicator").count(), 1, "nested copies are shown once: {text}");
    }

    /// A NavigationLink card (Button in a same-frame Button) is one line; a real inner button
    /// (another frame) and the keyboard / text helpers still work.
    #[test]
    fn compact_collapses_a_button_in_a_same_frame_button() {
        let card = Rect::new(16.0, 100.0, 368.0, 80.0);
        let inner = UiNode::new("XCUIElementTypeButton", card).with_label("Run, 5 km");
        let star = UiNode::new("XCUIElementTypeButton", Rect::new(340.0, 120.0, 30.0, 30.0)).with_label("Favorite");
        let outer = UiNode::new("XCUIElementTypeButton", card).with_label("Run, 5 km").with_children(vec![inner, star]);
        let kb = UiNode::new("XCUIElementTypeKeyboard", Rect::new(0.0, 500.0, 400.0, 300.0));
        let screen = UiNode::new("XCUIElementTypeApplication", Rect::new(0.0, 0.0, 400.0, 800.0)).with_children(vec![outer, kb]);
        let text = screen.to_compact_text(400.0, 800.0);
        assert_eq!(text.matches("Button \"Run, 5 km\"").count(), 1, "{text}");
        assert!(text.contains("Button \"Favorite\""), "{text}");
        assert_eq!(screen.keyboard(400.0, 800.0).unwrap().y, 500.0);
        assert_eq!(screen.containing("5 KM", 400.0, 800.0).len(), 2);
    }
}
