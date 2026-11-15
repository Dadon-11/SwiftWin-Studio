//! A fake WebDriverAgent HTTP server backed by any `Controller` (normally `MockDevice`).
//!
//! It answers the same routes and JSON shapes as the real WDA, so the HTTP
//! client, session handling and source parsing are exercised end-to-end
//! without a phone. Used by tests and by `swiftwin --mock wda serve`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use swiftwin_core::{Button, Point};
use swiftwin_device::{Controller, Device};

pub struct FakeWda {
    pub port: u16,
    state: Arc<Shared>,
}

struct Shared {
    session: Mutex<Option<String>>,
    counter: AtomicU64,
    requests: Mutex<Vec<String>>,
    settings: Mutex<serde_json::Map<String, Value>>,
    alert: Mutex<Option<swiftwin_core::Alert>>,
    alert_taps: Mutex<Vec<String>>,
}

impl FakeWda {
    /// Serve on 127.0.0.1 (random port) until the process exits.
    pub fn start(control: Arc<dyn Controller>, device: Arc<dyn Device>) -> std::io::Result<FakeWda> {
        Self::start_on(0, control, device)
    }

    pub fn start_on(port: u16, control: Arc<dyn Controller>, device: Arc<dyn Device>) -> std::io::Result<FakeWda> {
        let l = TcpListener::bind(("127.0.0.1", port))?;
        let port = l.local_addr()?.port();
        let state = Arc::new(Shared {
            session: Mutex::new(None),
            counter: AtomicU64::new(0),
            requests: Mutex::new(vec![]),
            settings: Mutex::new(serde_json::Map::new()),
            alert: Mutex::new(None),
            alert_taps: Mutex::new(vec![]),
        });
        let st = state.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let (c, d, st) = (control.clone(), device.clone(), st.clone());
                std::thread::spawn(move || {
                    let _ = handle(s, c.as_ref(), d.as_ref(), &st);
                });
            }
        });
        Ok(FakeWda { port, state })
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Forget the current session (like WDA restarting), to test re-creation.
    pub fn expire_session(&self) {
        *self.state.session.lock().unwrap() = None;
    }

    /// "METHOD /path" of every request received.
    pub fn requests(&self) -> Vec<String> {
        self.state.requests.lock().unwrap().clone()
    }

    /// Show an alert (like a confirmation dialog) until one of its buttons is tapped.
    pub fn show_alert(&self, text: &str, buttons: &[&str]) {
        *self.state.alert.lock().unwrap() =
            Some(swiftwin_core::Alert { text: text.into(), buttons: buttons.iter().map(|b| b.to_string()).collect() });
    }

    /// Alert buttons tapped through `/alert/accept`.
    pub fn alert_taps(&self) -> Vec<String> {
        self.state.alert_taps.lock().unwrap().clone()
    }

    /// Settings set through `/appium/settings`.
    pub fn settings(&self) -> Value {
        Value::Object(self.state.settings.lock().unwrap().clone())
    }
}

fn handle(s: TcpStream, c: &dyn Controller, d: &dyn Device, st: &Shared) -> std::io::Result<()> {
    let mut r = BufReader::new(s.try_clone()?);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    st.requests.lock().unwrap().push(format!("{method} {target}"));
    let (code, v) = route(&method, &target, &body, c, d, st);
    let text = v.to_string();
    let mut s = s;
    write!(
        s,
        "HTTP/1.1 {code} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        if code == 200 { "OK" } else { "Error" },
        text.len()
    )?;
    s.flush()
}

fn err(code: u16, error: &str, message: impl std::fmt::Display) -> (u16, Value) {
    (code, json!({"value": {"error": error, "message": message.to_string()}}))
}

fn ok(v: Value) -> (u16, Value) {
    (200, json!({"value": v, "sessionId": null}))
}

fn route(method: &str, target: &str, body: &Value, c: &dyn Controller, d: &dyn Device, st: &Shared) -> (u16, Value) {
    let path = target.split('?').next().unwrap_or("");
    let segs: Vec<&str> = path.trim_matches('/').split('/').collect();
    // Strip /session/:id, checking the id.
    let (rest, needs_session): (Vec<&str>, bool) = if segs.first() == Some(&"session") && segs.len() > 2 {
        let current = st.session.lock().unwrap().clone();
        if current.as_deref() != Some(segs[1]) {
            return err(404, "invalid session id", "Session does not exist");
        }
        (segs[2..].to_vec(), true)
    } else {
        (segs.clone(), false)
    };
    let p = |k: &str| body[k].as_f64().unwrap_or(0.0);
    let res = match (method, rest.join("/").as_str()) {
        ("GET", "status") => return ok(json!({"ready": true, "state": "success", "message": "Fake WDA is ready"})),
        ("POST", "session") if !needs_session => {
            let id = format!("FAKE-{}", st.counter.fetch_add(1, Ordering::SeqCst) + 1);
            *st.session.lock().unwrap() = Some(id.clone());
            return (200, json!({"value": {"sessionId": id, "capabilities": {}}, "sessionId": id}));
        }
        ("GET", "window/size") => c.window_size().map(|(w, h)| json!({"width": w, "height": h})),
        ("POST", "wda/tap") => c.tap(Point { x: p("x"), y: p("y") }).map(|_| Value::Null),
        ("POST", "wda/dragfromtoforduration") => c
            .swipe(Point { x: p("fromX"), y: p("fromY") }, Point { x: p("toX"), y: p("toY") }, p("duration"))
            .map(|_| Value::Null),
        ("POST", "wda/keys") => {
            let text: String =
                body["value"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
            c.type_text(&text).map(|_| Value::Null)
        }
        ("POST", "wda/pressButton") => match body["name"].as_str().unwrap_or("").parse::<Button>() {
            Ok(b) => c.press_button(b).map(|_| Value::Null),
            Err(e) => Err(e),
        },
        ("POST", "wda/homescreen") => c.press_button(Button::Home).map(|_| Value::Null),
        ("POST", "appium/settings") if needs_session => {
            let mut all = st.settings.lock().unwrap();
            if let Some(m) = body["settings"].as_object() {
                all.extend(m.clone());
            }
            Ok(Value::Object(all.clone()))
        }
        ("GET", "alert/text") => {
            return match st.alert.lock().unwrap().as_ref() {
                Some(a) => ok(json!(a.text)),
                None => err(404, "no such alert", "An attempt was made to operate on a modal dialog when one was not open"),
            }
        }
        ("GET", "wda/alert/buttons") if needs_session => {
            return match st.alert.lock().unwrap().as_ref() {
                Some(a) => ok(json!(a.buttons)),
                None => err(404, "no such alert", "An attempt was made to operate on a modal dialog when one was not open"),
            }
        }
        ("POST", "alert/accept") => {
            let mut alert = st.alert.lock().unwrap();
            let name = body["name"].as_str().unwrap_or("").to_string();
            return match alert.as_ref() {
                None => err(404, "no such alert", "An attempt was made to operate on a modal dialog when one was not open"),
                Some(a) if !name.is_empty() && !a.buttons.iter().any(|b| *b == name) => {
                    err(400, "invalid element state", format!("Failed to find button with label '{name}' for alert"))
                }
                Some(_) => {
                    st.alert_taps.lock().unwrap().push(name);
                    *alert = None;
                    ok(Value::Null)
                }
            };
        }
        ("GET", "source") => c.ui_tree().map(|t| crate::wda::to_wda_json(&t)),
        ("GET", "screenshot") => d.screenshot().map(|s| Value::String(swiftwin_core::base64_bytes::encode(&s.png))),
        _ => return err(404, "unknown command", format!("Unhandled endpoint: {method} {path}")),
    };
    match res {
        Ok(v) => ok(v),
        Err(e) => err(500, "invalid element state", e),
    }
}
