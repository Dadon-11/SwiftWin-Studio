//! Inspo: open another app on the phone, capture its screens safely, and turn them into a style
//! guide for the user's own app (palette, type scale, spacing, component patterns).
//!
//! Safe exploration: SwiftWin taps tab-bar buttons and scrolls; in deep mode it also opens every
//! button whose label doesn't suggest a change (buy, send, delete, sign out, like, add... are never
//! tapped, see [`is_risky`]) and comes back after each. Captures stay on this PC, in
//! `<project>/.inspo/<app>/` (git-ignored), because they may show the user's personal data.

pub mod style;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use swiftwin_control::{controller::find_runner, Phone};
use swiftwin_core::{AppInfo, Button, Point, Result, SwiftWinError, UiNode};
use swiftwin_device::LaunchOptions;

pub use style::Guide;

/// At most this many distinct screens are captured.
pub const MAX_SCREENS: usize = 12;
/// Scrolls per tab (stops early at the bottom).
const SCROLLS: usize = 2;

/// Said wherever the captures or the guide are used.
pub const GUARDRAIL: &str = "Use this as design inspiration only (palette, spacing, type scale, patterns). \
Do not copy the other app's logo, icons, artwork, brand name or exact screens.";
pub const PRIVACY: &str = "The captures may show your personal data (account names, playlists, messages). \
They stay on this PC in the project's .inspo folder, which is git-ignored.";

/// One captured screen.
pub struct Shot {
    pub png: Vec<u8>,
    pub tree: UiNode,
}

/// The phone's apps worth taking inspiration from: user apps, minus SwiftWin's own WebDriverAgent runner.
pub fn user_apps(phone: &Phone) -> Result<Vec<AppInfo>> {
    let wda = phone.wda.as_ref().map(|w| w.bundle_id());
    let mut apps: Vec<AppInfo> = phone
        .device
        .list_apps()?
        .into_iter()
        .filter(|a| {
            !a.bundle_id.starts_with("com.apple.")
                && Some(&a.bundle_id) != wda.as_ref()
                && find_runner(std::slice::from_ref(a)).is_none()
        })
        .collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    Ok(apps)
}

/// Capture `bundle_id`, extract the style guide and write it to `<project>/.inspo/<app>/`.
/// Afterwards the phone goes back to `return_to` (the user's app) if given, else Home.
/// `progress(message, screens_so_far)`; setting `cancel` stops exploring and keeps what was captured.
pub fn capture(
    phone: &Phone,
    bundle_id: &str,
    project: &Path,
    return_to: Option<&str>,
    deep: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&str, usize),
) -> Result<(PathBuf, Guide)> {
    let app = phone
        .device
        .list_apps()?
        .into_iter()
        .find(|a| a.bundle_id == bundle_id)
        .ok_or_else(|| SwiftWinError::NotFound(format!("{bundle_id} is not installed on the phone")))?;
    let shots = explore(phone, bundle_id, deep, cancel, progress);
    // Leave the phone where the user expects it, even if exploring failed.
    let back = return_to.and_then(|id| phone.device.launch(id, &LaunchOptions::default()).ok());
    if back.is_none() {
        let _ = phone.control.press_button(Button::Home);
    }
    let shots = shots?;
    if shots.is_empty() {
        return Err(SwiftWinError::Other("no screens were captured".into()));
    }
    progress("Extracting the style guide", shots.len());
    let screen_w = phone.control.window_size().map(|s| s.0).unwrap_or(0.0);
    let rel = std::path::Path::new(".inspo").join(folder_name(&app));
    let dir = swiftwin_core::paths::join_inside(project, &rel)
        .ok_or_else(|| SwiftWinError::invalid("the project's .inspo folder points outside the project"))?;
    let guide = style::write(&dir, &app, &shots, screen_w)?;
    std::fs::write(dir.parent().unwrap_or(&dir).join(".gitignore"), "*\n")?;
    Ok((dir, guide))
}

/// A safe folder name for the app: letters, digits and dashes only.
fn folder_name(app: &AppInfo) -> String {
    let clean = |s: &str| -> String {
        let s: String = s.chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
        s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
    };
    match clean(&app.name) {
        n if !n.is_empty() => n,
        _ => clean(&app.bundle_id),
    }
}

/// Launch the app and visit each tab (tap) and a couple of scroll positions per tab (swipe up),
/// keeping each distinct screen. Normal mode taps only tab-bar buttons. `deep` also opens every
/// safe button and row on those screens (see [`is_risky`]) and comes back after each.
// UNTESTED-ON-DEVICE: settle times, WDA trees of third-party apps, tab bars hidden on scroll, going back.
pub fn explore(phone: &Phone, bundle_id: &str, deep: bool, cancel: &AtomicBool, progress: &mut dyn FnMut(&str, usize)) -> Result<Vec<Shot>> {
    let settle = if phone.is_mock { Duration::ZERO } else { Duration::from_millis(1500) };
    progress("Opening the app", 0);
    phone.device.launch(bundle_id, &LaunchOptions::default())?;
    std::thread::sleep(settle * 2);
    let first = phone.control.ui_tree()?;
    let mut x = Explorer {
        phone,
        bundle_id,
        app_label: first.label.clone(),
        settle,
        cancel,
        max: if deep { MAX_SCREENS_DEEP } else { MAX_SCREENS },
        shots: Vec::new(),
        seen: HashSet::new(),
        seen_labels: Vec::new(),
        tab_limit: usize::MAX,
        taps: 0,
        progress,
    };
    let tab_labels: Vec<Option<String>> = match tab_buttons(&first) {
        t if t.is_empty() => vec![None],
        t => t.into_iter().map(|n| n.label.clone()).collect(),
    };
    x.snap()?;
    x.say("Captured the first screen");
    let tabs = tab_labels.len();
    for (i, tab) in tab_labels.into_iter().enumerate() {
        // Share what is left of the screen budget between this tab and the ones after it.
        let left = x.max.saturating_sub(x.shots.len());
        x.tab_limit = x.shots.len() + left.div_ceil(tabs - i);
        if x.done() {
            break;
        }
        if let Some(label) = &tab {
            x.say(&format!("Opening the “{label}” tab"));
            if !x.tap_tab(label)? {
                continue;
            }
        }
        let (mut tree, _) = x.snap()?;
        for scroll in 0..=SCROLLS {
            if deep {
                x.open_buttons(&tree, tab.as_deref())?;
            }
            if scroll == SCROLLS || x.done() {
                break;
            }
            x.say("Scrolling");
            let (w, h) = phone.control.window_size()?;
            phone.control.swipe(Point { x: w / 2.0, y: h * 0.7 }, Point { x: w / 2.0, y: h * 0.3 }, 0.3)?;
            std::thread::sleep(settle);
            let (t, new) = x.snap()?;
            if !new {
                break; // bottom reached
            }
            tree = t;
        }
    }
    Ok(x.shots)
}

/// At most this many screens in deep mode, and this many taps on buttons.
pub const MAX_SCREENS_DEEP: usize = 40;
const MAX_TAPS_DEEP: usize = 120;
/// Buttons per screen that deep mode opens.
const BUTTONS_PER_SCREEN: usize = 15;

/// Words of buttons that would change something (buy, send, delete, sign out, like, add, settings…):
/// never tapped, in deep mode either.
const RISKY: &[&str] = &[
    "buy", "purchase", "subscribe", "subscription", "premium", "upgrade", "pay", "payment", "checkout", "cart", "order",
    "send", "post", "share", "publish", "upload", "delete", "remove", "clear", "erase", "reset", "logout", "signout",
    "login", "signin", "signup", "register", "follow", "unfollow", "block", "report", "download", "add", "create",
    "new", "invite", "call", "record", "camera", "like", "unlike", "save", "edit", "rename", "allow", "install", "rate",
    "review", "donate", "tip", "gift", "redeem", "confirm", "submit", "accept", "join", "leave", "mute", "hide",
    "compose", "sell", "transfer", "book", "unlock",
];

/// Would tapping a button with this label change something? Matches whole words, so "Liked Songs" is fine
/// but "Like" is not; "Log out" / "Sign in" match with or without the space.
pub fn is_risky(label: &str) -> bool {
    let l = label.to_lowercase();
    let words: Vec<String> = l.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect();
    let pairs = words.windows(2).map(|p| p.concat());
    words.iter().cloned().chain(pairs).any(|w| RISKY.contains(&w.as_str()))
}

struct Explorer<'a> {
    phone: &'a Phone,
    bundle_id: &'a str,
    app_label: Option<String>,
    settle: Duration,
    cancel: &'a AtomicBool,
    max: usize,
    shots: Vec<Shot>,
    seen: HashSet<String>,
    /// Labels of each captured screen, to skip near-copies (a timer or a "now playing" line changed).
    seen_labels: Vec<HashSet<String>>,
    /// This tab's share of the screen budget (`shots.len()` to stop at).
    tab_limit: usize,
    taps: usize,
    progress: &'a mut dyn FnMut(&str, usize),
}

impl Explorer<'_> {
    fn done(&self) -> bool {
        self.shots.len() >= self.max.min(self.tab_limit) || self.cancel.load(Ordering::SeqCst)
    }

    fn say(&mut self, m: &str) {
        (self.progress)(m, self.shots.len());
    }

    fn in_app(&self, t: &UiNode) -> bool {
        t.label == self.app_label
    }

    /// Screenshot + tree; the flag is true if it is a new screen of this app (never captures another app).
    fn snap(&mut self) -> Result<(UiNode, bool)> {
        let tree = self.phone.control.ui_tree()?;
        let l = labels(&tree);
        if !self.in_app(&tree) || !self.seen.insert(signature(&tree)) || self.seen_labels.iter().any(|s| jaccard(s, &l) >= 0.85) {
            return Ok((tree, false));
        }
        self.seen_labels.push(l);
        self.shots.push(Shot { png: self.phone.screenshot()?.png, tree: tree.clone() });
        Ok((tree, true))
    }

    fn relaunch(&self, kill: bool) -> Result<()> {
        self.phone.device.launch(self.bundle_id, &LaunchOptions { kill_existing: kill, ..Default::default() })?;
        std::thread::sleep(self.settle * 2);
        Ok(())
    }

    /// Tap the tab with this label, if it is on screen now (re-read so we never tap something else).
    fn tap_tab(&mut self, label: &str) -> Result<bool> {
        let mut tree = self.phone.control.ui_tree()?;
        if !self.in_app(&tree) {
            self.relaunch(false)?;
            tree = self.phone.control.ui_tree()?;
        }
        let Some(at) = tab_buttons(&tree).into_iter().find(|n| n.label.as_deref() == Some(label)).map(|n| n.rect.center()) else {
            return Ok(false);
        };
        self.phone.control.tap(at)?;
        std::thread::sleep(self.settle);
        Ok(true)
    }

    /// Deep mode: open each safe button of `base`, capture what it shows, and come back.
    fn open_buttons(&mut self, base: &UiNode, tab: Option<&str>) -> Result<()> {
        let base_labels = labels(base);
        let todo: Vec<String> = safe_buttons(base).into_iter().filter_map(|n| n.label.clone()).collect();
        for label in todo {
            if self.done() || self.taps >= MAX_TAPS_DEEP {
                break;
            }
            let tree = self.phone.control.ui_tree()?;
            if !(self.in_app(&tree) && similar(&labels(&tree), &base_labels)) && !self.back(&base_labels, tab)? {
                return Ok(()); // can't get back to this screen: move on to the next tab
            }
            let tree = self.phone.control.ui_tree()?;
            let Some(at) = safe_buttons(&tree).into_iter().find(|n| n.label.as_deref() == Some(label.as_str())).map(|n| n.rect.center()) else {
                continue;
            };
            self.say(&format!("Opening “{label}”"));
            self.phone.control.tap(at)?;
            self.taps += 1;
            std::thread::sleep(self.settle);
            self.snap()?;
            if !self.back(&base_labels, tab)? {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Return to the screen whose labels are `base`: Back/Close button, edge swipe, sheet swipe down,
    /// re-tapping the tab, and finally restarting the app. False if none of it worked.
    fn back(&mut self, base: &HashSet<String>, tab: Option<&str>) -> Result<bool> {
        let (w, h) = self.phone.control.window_size()?;
        for step in 0..6 {
            let tree = self.phone.control.ui_tree()?;
            if !self.in_app(&tree) {
                self.relaunch(false)?;
                continue;
            }
            if similar(&labels(&tree), base) {
                return Ok(true);
            }
            let ctl = &self.phone.control;
            match step {
                0 => match back_button(&tree) {
                    Some(at) => ctl.tap(at)?,
                    None => continue,
                },
                1 => ctl.swipe(Point { x: 2.0, y: h * 0.5 }, Point { x: w * 0.8, y: h * 0.5 }, 0.3)?,
                2 => ctl.swipe(Point { x: w / 2.0, y: h * 0.12 }, Point { x: w / 2.0, y: h * 0.9 }, 0.3)?,
                3 | 4 => match tab {
                    Some(t) => {
                        self.tap_tab(t)?;
                    }
                    None => continue,
                },
                _ => {
                    self.relaunch(true)?;
                    if let Some(t) = tab {
                        self.tap_tab(t)?;
                    }
                }
            }
            std::thread::sleep(self.settle);
        }
        let tree = self.phone.control.ui_tree()?;
        Ok(self.in_app(&tree) && similar(&labels(&tree), base))
    }
}

fn labels(t: &UiNode) -> HashSet<String> {
    t.walk().iter().filter_map(|n| n.label.clone()).collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    let union = a.union(b).count();
    if union == 0 { 1.0 } else { a.intersection(b).count() as f64 / union as f64 }
}

/// Mostly the same labels (live parts like a playback timer may change).
fn similar(a: &HashSet<String>, b: &HashSet<String>) -> bool {
    jaccard(a, b) >= 0.75
}

fn is_back(n: &UiNode) -> bool {
    let id = format!("{} {}", n.label.as_deref().unwrap_or(""), n.name.as_deref().unwrap_or("")).to_lowercase();
    ["back", "close", "cancel", "dismiss"].iter().any(|k| id.split(|c: char| !c.is_alphanumeric()).any(|w| w == *k))
}

/// A Back / Close / Cancel button (navigation bars, sheets).
fn back_button(tree: &UiNode) -> Option<Point> {
    tree.walk().into_iter().find(|n| n.kind == "XCUIElementTypeButton" && is_back(n)).map(|n| n.rect.center())
}

/// Buttons and rows on screen that deep mode may open: labelled, enabled, fully visible between the status
/// bar and the tab bar, not a tab, not Back, not risky, one per label.
fn safe_buttons(tree: &UiNode) -> Vec<&UiNode> {
    let (w, h) = (tree.rect.width, tree.rect.height);
    let bars = tab_bars(tree);
    let bottom = bars.iter().map(|b| b.y).fold(h * 0.92, f64::min);
    let mut seen = HashSet::new();
    tree.walk()
        .into_iter()
        .filter(|n| matches!(n.kind.as_str(), "XCUIElementTypeButton" | "XCUIElementTypeCell" | "XCUIElementTypeLink") && n.enabled)
        .filter(|n| n.label.as_deref().is_some_and(|l| !l.trim().is_empty() && !is_risky(l)) && !is_back(n))
        .filter(|n| n.rect.x >= 0.0 && n.rect.x + n.rect.width <= w + 1.0 && n.rect.y >= 50.0 && n.rect.y + n.rect.height <= bottom + 1.0)
        .filter(|n| !bars.iter().any(|b| b.contains(n.rect.center())))
        .filter(|n| seen.insert(n.label.clone()))
        .take(BUTTONS_PER_SCREEN)
        .collect()
}

/// Tab bars: `XCUIElementTypeTabBar`, or a wide strip in the bottom quarter named like one (e.g. `TabBarContainer`).
fn tab_bars(tree: &UiNode) -> Vec<swiftwin_core::Rect> {
    let (w, h) = (tree.rect.width, tree.rect.height);
    let custom = |n: &UiNode| {
        let id = format!("{} {}", n.name.as_deref().unwrap_or(""), n.label.as_deref().unwrap_or("")).to_lowercase().replace([' ', '-', '_'], "");
        (id.contains("tabbar") || id.contains("tabscontainer")) && n.rect.y >= h * 0.75 && n.rect.height <= 120.0 && n.rect.width >= w * 0.6
    };
    tree.walk().into_iter().filter(|n| n.kind == "XCUIElementTypeTabBar" || custom(n)).map(|n| n.rect).collect()
}

/// Buttons inside a tab bar, one per label (by geometry, so flat and nested trees both work).
/// Tabs that start something ("Create", "Buy"…) are left out.
fn tab_buttons(tree: &UiNode) -> Vec<&UiNode> {
    let bars = tab_bars(tree);
    let mut seen = HashSet::new();
    let tabs: Vec<&UiNode> = tree
        .walk()
        .into_iter()
        .filter(|n| n.kind == "XCUIElementTypeButton" && bars.iter().any(|b| b.contains(n.rect.center())))
        .filter(|n| n.label.as_deref().is_some_and(|l| !l.trim().is_empty() && !is_risky(l)))
        .filter(|n| seen.insert(n.label.clone()))
        .collect();
    if tabs.len() >= 2 { tabs } else { Vec::new() }
}

/// What makes a screen distinct: its visible texts and their positions.
fn signature(tree: &UiNode) -> String {
    tree.walk()
        .iter()
        .filter_map(|n| n.label.as_ref().map(|l| format!("{}@{:.0},{:.0}", l, n.rect.x, n.rect.y)))
        .collect::<Vec<_>>()
        .join("|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use swiftwin_core::{LogQuery, Rect};
    use swiftwin_device::mock::{DEMO_APP, TUNES_APP};

    #[test]
    fn lists_user_apps_only() {
        let apps = user_apps(&Phone::mock()).unwrap();
        let ids: Vec<_> = apps.iter().map(|a| a.bundle_id.as_str()).collect();
        assert_eq!(ids, [DEMO_APP, TUNES_APP]);
    }

    #[test]
    fn explores_tabs_and_scrolls_without_tapping_other_buttons() {
        let phone = Phone::mock();
        let dir = std::env::temp_dir().join(format!("swiftwin-inspo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap(); // a project folder exists before Inspo writes into it
        let mut steps = Vec::new();
        let (out, guide) =
            capture(&phone, TUNES_APP, &dir, Some(DEMO_APP), false, &AtomicBool::new(false), &mut |m, n| steps.push((m.to_string(), n))).unwrap();
        // Home: top + 2 scrolls; Search and Library: tab + 2 scrolls each.
        assert_eq!(guide.screens.len(), 9, "{steps:?}");
        assert!(steps.iter().any(|(m, _)| m.contains("Library")));
        let logs = swiftwin_device::collect_logs(phone.device.as_ref(), &LogQuery::default()).unwrap();
        assert!(!logs.iter().any(|l| l.message.contains("Purchase")), "never taps Buy Premium");
        assert!(logs.iter().any(|l| l.message.contains(&format!("Launching {DEMO_APP}"))), "returns to the user's app");
        assert!(out.ends_with(".inspo/Tunes") || out.ends_with(".inspo\\Tunes"));
        for f in ["inspo.json", "INSPO.md", "InspoTheme.swift", "overview.png", "01.png", "01.json"] {
            assert!(out.join(f).is_file(), "{f}");
        }
        assert_eq!(std::fs::read_to_string(dir.join(".inspo/.gitignore")).unwrap(), "*\n");
        let p = &guide.palette;
        assert_eq!((p.background.as_str(), p.surface.as_str(), p.text.as_str()), ("#121218", "#282834", "#FFFFFF"));
        assert_eq!(p.accent.as_deref(), Some("#FF5A5F"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn deep_opens_every_safe_row_comes_back_and_never_deletes_or_buys() {
        let phone = Phone::mock();
        let mut steps = Vec::new();
        let shots = explore(&phone, TUNES_APP, true, &AtomicBool::new(false), &mut |m, _| steps.push(m.to_string())).unwrap();
        assert_eq!(shots.len(), MAX_SCREENS_DEEP, "{steps:?}");
        let titles: Vec<String> = shots.iter().filter_map(|s| s.tree.find("Back").map(|_| s.tree.walk()[2].label.clone().unwrap())).collect();
        for t in ["Song 1", "Genre 1", "Playlist 1"] {
            assert!(titles.contains(&t.to_string()), "every tab gets its share: {titles:?}");
        }
        assert!(steps.iter().all(|m| !m.contains("Delete") && !m.contains("Buy")), "{steps:?}");
        let logs = swiftwin_device::collect_logs(phone.device.as_ref(), &LogQuery::default()).unwrap();
        assert!(!logs.iter().any(|l| l.message.contains("Purchase") || l.message.contains("Deleted")));
    }

    #[test]
    fn risky_labels() {
        for l in ["Buy Premium", "Delete Playlist", "Log out", "Sign In", "Like", "Add to playlist", "Share", "Create"] {
            assert!(is_risky(l), "{l}");
        }
        for l in ["Liked Songs", "Home", "Your Library", "Settings", "Song 3", "Playlists", "Profile"] {
            assert!(!is_risky(l), "{l}");
        }
    }

    #[test]
    fn cancel_stops_after_the_first_screen() {
        let phone = Phone::mock();
        let cancel = AtomicBool::new(true);
        let shots = explore(&phone, TUNES_APP, false, &cancel, &mut |_, _| {}).unwrap();
        assert_eq!(shots.len(), 1);
    }

    #[test]
    fn app_without_tab_bar_just_scrolls() {
        let shots = explore(&Phone::mock(), DEMO_APP, false, &AtomicBool::new(false), &mut |_, _| {}).unwrap();
        assert_eq!(shots.len(), 3);
    }

    #[test]
    fn finds_custom_tab_bars_and_skips_action_tabs() {
        // Like Spotify: no XCUIElementTypeTabBar, nested buttons repeating each label, a "Create" tab.
        let b = |l: &str, x: f64| UiNode::new("XCUIElementTypeButton", Rect::new(x, 791.0, 101.0, 49.0)).with_label(l);
        let mut bar = UiNode::new("XCUIElementTypeOther", Rect::new(0.0, 791.0, 402.0, 83.0));
        bar.name = Some("TabBarContainer.stackView".into());
        bar.children = vec![b("Home", 0.0).with_children(vec![b("Home", 0.0)]), b("Search", 100.0), b("Your Library", 201.0), b("Create", 301.0)];
        let mut top = UiNode::new("XCUIElementTypeOther", Rect::new(0.0, 0.0, 402.0, 80.0));
        top.name = Some("TabBarHeader".into()); // not at the bottom: not a tab bar; its Buy is an action anyway
        top.children = vec![b("Buy", 0.0)];
        let tree = UiNode::new("XCUIElementTypeApplication", Rect::new(0.0, 0.0, 402.0, 874.0)).with_children(vec![top, bar]);
        let labels: Vec<_> = tab_buttons(&tree).iter().map(|n| n.label.clone().unwrap()).collect();
        assert_eq!(labels, ["Home", "Search", "Your Library"]);
    }

    /// A project from someone else could ship `.inspo` as a link to another folder.
    #[cfg(windows)]
    #[test]
    fn a_linked_inspo_folder_is_refused() {
        let base = std::env::temp_dir().join(format!("swiftwin-inspo-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (proj, outside) = (base.join("proj"), base.join("outside"));
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(proj.join(".inspo"))
            .arg(&outside)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
        let r = capture(&Phone::mock(), TUNES_APP, &proj, None, false, &AtomicBool::new(false), &mut |_, _| {});
        assert!(r.unwrap_err().to_string().contains("outside the project"));
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0, "nothing was written through the link");
        std::fs::remove_dir(proj.join(".inspo")).ok();
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn folder_names_are_safe() {
        let a = |n: &str| AppInfo { bundle_id: "com.x.y".into(), name: n.into(), version: None };
        assert_eq!(folder_name(&a("My Cool/App..")), "My-Cool-App");
        assert_eq!(folder_name(&a("..\\..")), "com-x-y");
    }
}
