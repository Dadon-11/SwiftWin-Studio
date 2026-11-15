//! HTTP client for WebDriverAgent (appium/WebDriverAgent).
//!
//! Routes and JSON shapes verified against WebDriverAgentLib (Commands/*.m, Routing/*.m) at
//! appium/WebDriverAgent tag v16.15.3, the version SwiftWin installs:
//! `GET /status`, `POST /session {capabilities}` (kills any other session; WDA keeps one),
//! `GET /session/:id/window/size`, `POST /session/:id/wda/tap {x,y}` (points, relative to
//! the active app), `POST /session/:id/wda/dragfromtoforduration {fromX,fromY,toX,toY,duration}`
//! (duration = press time before dragging), `POST /session/:id/wda/keys {value:[..]}`,
//! `POST /session/:id/wda/pressButton {name}` (matched lowercased: home, volumeup, volumedown),
//! `POST /wda/homescreen`, `GET /source?format=json`, `GET /screenshot` (always PNG, in pixels),
//! `POST /session/:id/appium/settings {settings}` (answers with all settings),
//! `GET /alert/text`, `GET /session/:id/wda/alert/buttons`, `POST /alert/accept {name}` (404
//! `no such alert` when none is open; alerts of SpringBoard and of the app both count).
//! Every reply is `{"value": .., "sessionId": ..}`; errors are `{"value": {"error", "message",
//! "traceback"}}` with an HTTP error status, e.g. 404 `invalid session id` for a stale session.

use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};
use swiftwin_core::{Alert, Button, Point, Rect, Result, Screenshot, SwiftWinError, UiNode};
use swiftwin_device::Controller;

/// Press time before a swipe starts moving. Short = a flick/scroll, long = drag-and-drop.
pub const SWIPE_PRESS_S: f64 = 0.05;

/// `/status` must answer fast; a WDA that takes longer is treated as not ready.
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);

/// Source attributes SwiftWin doesn't use. WDA computes them for every element and the
/// visibility/accessibility ones are slow on big screens; `enabled` is kept.
pub const SOURCE_EXCLUDED: &str =
    "visible,accessible,nativeAccessibilityElement,focused,traits,placeholderValue,nativeFrame,frame,minValue,maxValue";

pub struct WdaClient {
    base: String,
    agent: ureq::Agent,
    session: Mutex<Option<String>>,
}

impl WdaClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout(Duration::from_secs(60))
            .build();
        WdaClient { base: base_url.into().trim_end_matches('/').to_string(), agent, session: Mutex::new(None) }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        self.request_within(method, path, body, None)
    }

    fn request_within(&self, method: &str, path: &str, body: Option<Value>, limit: Option<Duration>) -> Result<Value> {
        let url = format!("{}{}", self.base, path);
        let mut req = self.agent.request(method, &url);
        if let Some(t) = limit {
            req = req.timeout(t);
        }
        let res = match body {
            Some(b) => req.send_json(b),
            None => req.call(),
        };
        match res {
            Ok(r) => r.into_json::<Value>().map_err(|e| SwiftWinError::Http(format!("{method} {path}: bad JSON: {e}"))),
            Err(ureq::Error::Status(code, r)) => {
                let v: Value = r.into_json().unwrap_or(Value::Null);
                let err = v["value"]["error"].as_str().unwrap_or("error").to_string();
                let msg = v["value"]["message"].as_str().unwrap_or("").to_string();
                Err(WdaError::Status { code, error: err, message: msg }.into())
            }
            Err(e) => Err(SwiftWinError::Http(format!(
                "WebDriverAgent is not reachable at {} ({e})",
                self.base
            ))),
        }
    }

    /// True when WDA answers `/status` with `ready`.
    pub fn is_ready(&self) -> bool {
        self.request_within("GET", "/status", None, Some(STATUS_TIMEOUT))
            .map(|v| v["value"]["ready"].as_bool().unwrap_or(true))
            .unwrap_or(false)
    }

    fn session_id(&self) -> Result<String> {
        let mut g = self.session.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(s) = g.as_ref() {
            return Ok(s.clone());
        }
        let v = self.request("POST", "/session", Some(json!({"capabilities": {"alwaysMatch": {}, "firstMatch": [{}]}})))?;
        let id = v["sessionId"]
            .as_str()
            .or_else(|| v["value"]["sessionId"].as_str())
            .ok_or_else(|| SwiftWinError::protocol("WebDriverAgent did not return a sessionId"))?
            .to_string();
        *g = Some(id.clone());
        Ok(id)
    }

    /// Call a session route, re-creating the session once if WDA forgot it.
    fn session_call(&self, method: &str, sub: &str, body: Option<Value>) -> Result<Value> {
        for attempt in 0..2 {
            let id = self.session_id()?;
            match self.request(method, &format!("/session/{id}{sub}"), body.clone()) {
                Err(SwiftWinError::Protocol(m)) if attempt == 0 && m.contains("invalid session id") => {
                    *self.session.lock().unwrap_or_else(|p| p.into_inner()) = None;
                }
                other => return other,
            }
        }
        unreachable!()
    }

    /// Settings of WDA's MJPEG screen stream (device port 9100): frames per second, size in
    /// percent of the screen, JPEG quality. Confirmed on iOS 27.0.1: the stream runs at ~9 fps
    /// with WDA's default of 10, and ~23 fps at full size (1206x2622, ~150 KB a frame at
    /// quality 25) with 30 or more; the phone's capture speed is the limit, not USB.
    pub fn set_mjpeg(&self, fps: u32, scale_percent: u32, quality: u32) -> Result<()> {
        let settings = json!({"settings": {
            "mjpegServerFramerate": fps,
            "mjpegScalingFactor": scale_percent,
            "mjpegServerScreenshotQuality": quality,
        }});
        self.session_call("POST", "/appium/settings", Some(settings)).map(|_| ())
    }

    pub fn screenshot(&self) -> Result<Screenshot> {
        let v = self.request("GET", "/screenshot", None)?;
        let b64 = v["value"].as_str().ok_or_else(|| SwiftWinError::protocol("screenshot: no value"))?;
        let png = swiftwin_core::base64_bytes::decode(b64).ok_or_else(|| SwiftWinError::protocol("screenshot: bad base64"))?;
        Screenshot::from_png(png)
    }
}

#[derive(Debug)]
pub enum WdaError {
    Status { code: u16, error: String, message: String },
}

impl From<WdaError> for SwiftWinError {
    fn from(e: WdaError) -> Self {
        match e {
            WdaError::Status { code, error, message } => {
                SwiftWinError::Protocol(format!("WebDriverAgent HTTP {code}: {error}: {message}"))
            }
        }
    }
}

impl Controller for WdaClient {
    fn window_size(&self) -> Result<(f64, f64)> {
        let v = self.session_call("GET", "/window/size", None)?;
        let w = v["value"]["width"].as_f64().ok_or_else(|| SwiftWinError::protocol("window size: no width"))?;
        let h = v["value"]["height"].as_f64().ok_or_else(|| SwiftWinError::protocol("window size: no height"))?;
        Ok((w, h))
    }

    fn tap(&self, at: Point) -> Result<()> {
        self.session_call("POST", "/wda/tap", Some(json!({"x": at.x, "y": at.y}))).map(|_| ())
    }

    fn swipe(&self, from: Point, to: Point, _duration_s: f64) -> Result<()> {
        let body = json!({"fromX": from.x, "fromY": from.y, "toX": to.x, "toY": to.y, "duration": SWIPE_PRESS_S});
        self.session_call("POST", "/wda/dragfromtoforduration", Some(body)).map(|_| ())
    }

    fn type_text(&self, text: &str) -> Result<()> {
        let chars: Vec<String> = text.chars().map(|c| c.to_string()).collect();
        self.session_call("POST", "/wda/keys", Some(json!({ "value": chars }))).map(|_| ())
    }

    fn press_button(&self, button: Button) -> Result<()> {
        if button == Button::Home {
            return self.request("POST", "/wda/homescreen", Some(json!({}))).map(|_| ());
        }
        self.session_call("POST", "/wda/pressButton", Some(json!({"name": button.wda_name()}))).map(|_| ())
    }

    fn ui_tree(&self) -> Result<UiNode> {
        let v = self.request("GET", &format!("/source?format=json&excluded_attributes={SOURCE_EXCLUDED}"), None)?;
        parse_source(&v["value"])
    }

    // Confirmed on iOS 27.0.1: a SwiftUI `.alert` is reported with its text and buttons, and
    // `/alert/accept {name}` presses the named button. The check adds ~0.5 s to a tap by label.
    fn alert(&self) -> Result<Option<Alert>> {
        let no_alert = |e: &SwiftWinError| matches!(e, SwiftWinError::Protocol(m) if m.contains("no such alert"));
        let text = match self.request("GET", "/alert/text", None) {
            Ok(v) => v["value"].as_str().unwrap_or("").to_string(),
            Err(e) if no_alert(&e) => return Ok(None),
            Err(e) => return Err(e),
        };
        let buttons = match self.session_call("GET", "/wda/alert/buttons", None) {
            Ok(v) => v["value"]
                .as_array()
                .map(|a| a.iter().filter_map(|b| b.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            Err(e) if no_alert(&e) => return Ok(None),
            Err(e) => return Err(e),
        };
        Ok(Some(Alert { text, buttons }))
    }

    fn alert_tap(&self, name: &str) -> Result<()> {
        self.request("POST", "/alert/accept", Some(json!({ "name": name }))).map(|_| ())
    }

    // UNTESTED-ON-DEVICE: appium WDA's `GET /session/:id/element/active` (FBFindElementCommands)
    // returns the app's `fb_activeElement` or "no such element"; type/label/rect are per-element routes.
    fn focused(&self) -> Result<Option<UiNode>> {
        let v = match self.session_call("GET", "/element/active", None) {
            Ok(v) => v,
            Err(SwiftWinError::Protocol(m)) if m.contains("no such element") => return Ok(None),
            Err(e) => return Err(e),
        };
        let id = v["value"]["ELEMENT"]
            .as_str()
            .or_else(|| v["value"]["element-6066-11e4-a52e-4f735466cecf"].as_str())
            .ok_or_else(|| SwiftWinError::protocol("element/active: no element id"))?
            .to_string();
        let kind = self.session_call("GET", &format!("/element/{id}/name"), None)?["value"].as_str().unwrap_or("Other").to_string();
        let r = self.session_call("GET", &format!("/element/{id}/rect"), None)?;
        let f = |k: &str| r["value"][k].as_f64().unwrap_or(0.0);
        let kind = if kind.starts_with("XCUIElementType") { kind } else { format!("XCUIElementType{kind}") };
        let mut n = UiNode::new(&kind, swiftwin_core::Rect::new(f("x"), f("y"), f("width"), f("height")));
        n.label = self.session_call("GET", &format!("/element/{id}/attribute/label"), None).ok().and_then(|v| v["value"].as_str().map(str::to_string));
        Ok(Some(n))
    }

    // UNTESTED-ON-DEVICE: `POST /wda/device/appearance {"name": "dark"|"light"}` (FBCustomCommands,
    // no session), which sets XCUIDevice's appearance.
    fn set_appearance(&self, dark: bool) -> Result<()> {
        self.request("POST", "/wda/device/appearance", Some(json!({"name": if dark { "dark" } else { "light" }}))).map(|_| ())
    }
}

/// Convert WDA's JSON source into a `UiNode` tree.
pub fn parse_source(v: &Value) -> Result<UiNode> {
    if !v.is_object() {
        return Err(SwiftWinError::protocol("WDA source is not a JSON object"));
    }
    Ok(parse_node(v))
}

fn parse_node(v: &Value) -> UiNode {
    let s = |k: &str| match &v[k] {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    };
    let kind = s("type").unwrap_or_else(|| "Other".into());
    let kind = if kind.starts_with("XCUIElementType") { kind } else { format!("XCUIElementType{kind}") };
    let r = &v["rect"];
    let num = |x: &Value| x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse().ok())).unwrap_or(0.0);
    let enabled = match &v["isEnabled"] {
        Value::Bool(b) => *b,
        Value::String(s) => s != "0" && !s.eq_ignore_ascii_case("false"),
        Value::Number(n) => n.as_i64() != Some(0),
        _ => true,
    };
    UiNode {
        kind,
        label: s("label"),
        name: s("name"),
        value: s("value"),
        rect: Rect::new(num(&r["x"]), num(&r["y"]), num(&r["width"]), num(&r["height"])),
        enabled,
        children: v["children"].as_array().map(|c| c.iter().map(parse_node).collect()).unwrap_or_default(),
    }
}

/// Inverse of `parse_source` (used by the fake WDA server).
pub fn to_wda_json(n: &UiNode) -> Value {
    json!({
        "type": n.kind.strip_prefix("XCUIElementType").unwrap_or(&n.kind),
        "label": n.label,
        "name": n.name,
        "value": n.value,
        "rect": {"x": n.rect.x, "y": n.rect.y, "width": n.rect.width, "height": n.rect.height},
        "isEnabled": if n.enabled { "1" } else { "0" },
        "children": n.children.iter().map(to_wda_json).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_shaped_source() {
        let v: Value = serde_json::from_str(
            r#"{"type":"Application","label":"Hello","name":"Hello","value":null,"rawIdentifier":null,
                "rect":{"x":0,"y":0,"width":393,"height":852},"isEnabled":"1",
                "children":[{"type":"Button","label":"Tap me","name":"Tap me","value":null,
                  "rect":{"x":20,"y":165,"width":170,"height":50},"isEnabled":"0"},
                  {"type":"Slider","value":0.5,"rect":{"x":0,"y":0,"width":1,"height":1}}]}"#,
        )
        .unwrap();
        let t = parse_source(&v).unwrap();
        assert_eq!(t.kind, "XCUIElementTypeApplication");
        assert_eq!(t.children[0].kind, "XCUIElementTypeButton");
        assert!(!t.children[0].enabled);
        assert_eq!(t.children[1].value.as_deref(), Some("0.5"));
        assert_eq!(parse_source(&to_wda_json(&t)).unwrap(), t);
    }
}
