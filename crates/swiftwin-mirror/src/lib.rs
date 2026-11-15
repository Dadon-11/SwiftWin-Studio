//! Hybrid mirroring.
//!
//! * **USB screenshots** – exact frames (what Claude sees), a few per second.
//! * **Video** – smooth frames for the person watching: WebDriverAgent's MJPEG
//!   stream over USB (~25 fps, `Feed::Mjpeg`) or AirPlay (UxPlay prototype, `Feed::Frames`).
//! * **Automatic fallback** – `policy::Policy` picks the source; when the video
//!   stalls the mirror continues from USB screenshots within ~1.5 s, and returns to
//!   the video once it is steady again.

pub mod airplay;
pub mod frame;
pub mod mjpeg;
pub mod policy;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub use frame::{Frame, FrameFormat, SourceKind};
pub use policy::{Mode, Policy, Switch};
use swiftwin_core::{Result, Screenshot};

/// Something that returns an exact screenshot (e.g. `Phone::screenshot`).
pub type ScreenshotFn = Arc<dyn Fn() -> Result<Screenshot> + Send + Sync>;

/// Opens an MJPEG-over-HTTP stream and returns its `host:port` (e.g. `Phone::video_stream`).
pub type OpenStreamFn = Arc<dyn Fn() -> Result<String> + Send + Sync>;

/// A smooth video source shown instead of USB screenshots while it delivers frames.
pub struct Video {
    /// `SourceKind::Stream` or `SourceKind::AirPlay`.
    pub kind: SourceKind,
    pub feed: Feed,
}

pub enum Feed {
    /// Complete JPEG frames pushed by a receiver (AirPlay through UxPlay).
    Frames(Receiver<Vec<u8>>),
    /// An MJPEG-over-HTTP server (WebDriverAgent's screen stream). The function is called
    /// again, after a pause that grows while it keeps failing, whenever the stream ends.
    Mjpeg(OpenStreamFn),
}

/// Pause before reopening a video stream that ended; doubles while it keeps failing.
const REOPEN_FIRST: Duration = Duration::from_millis(500);
const REOPEN_MAX: Duration = Duration::from_secs(30);

struct Inner {
    policy: Mutex<Policy>,
    latest: Mutex<Option<Frame>>,
    cv: Condvar,
    seq: AtomicU64,
    switches: Mutex<Vec<Switch>>,
    stop: AtomicBool,
    usb_errors: AtomicU64,
    last_error: Mutex<Option<String>>,
    video: Option<SourceKind>,
    video_error: Mutex<Option<String>>,
}

#[derive(Clone)]
pub struct Mirror {
    inner: Arc<Inner>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MirrorStatus {
    pub active: SourceKind,
    pub mode: Mode,
    /// The video source, if any (`stream` or `airplay`).
    pub video: Option<SourceKind>,
    pub video_healthy: bool,
    /// Why the video stream last ended or could not be opened.
    pub video_error: Option<String>,
    pub frames: u64,
    pub usb_errors: u64,
    pub last_error: Option<String>,
    pub switches: Vec<Switch>,
}

impl Mirror {
    /// Start mirroring from USB screenshots (`usb`, at most `usb_fps`) and, if given, a video source.
    pub fn start(usb: ScreenshotFn, video: Option<Video>, policy: Policy, usb_fps: f64) -> Mirror {
        let policy = match &video {
            Some(v) => policy.with_video(v.kind),
            None => policy,
        };
        let inner = Arc::new(Inner {
            policy: Mutex::new(policy),
            latest: Mutex::new(None),
            cv: Condvar::new(),
            seq: AtomicU64::new(0),
            switches: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
            usb_errors: AtomicU64::new(0),
            last_error: Mutex::new(None),
            video: video.as_ref().map(|v| v.kind),
            video_error: Mutex::new(None),
        });
        let m = Mirror { inner };
        let interval = Duration::from_secs_f64(1.0 / usb_fps.clamp(0.5, 30.0));

        // USB poller: runs while USB is the active source.
        let me = m.clone();
        std::thread::Builder::new()
            .name("mirror-usb".into())
            .spawn(move || {
                let mut last: Option<Vec<u8>> = None;
                while !me.inner.stop.load(Ordering::SeqCst) {
                    let started = Instant::now();
                    if me.active() == SourceKind::Usb {
                        match usb() {
                            Ok(s) => {
                                *me.inner.last_error.lock().unwrap() = None;
                                // Only publish when the picture changed (saves bandwidth); a
                                // source switch always publishes.
                                let changed = last.as_ref() != Some(&s.png)
                                    || me.latest().is_none_or(|f| f.source != SourceKind::Usb);
                                if changed {
                                    last = Some(s.png.clone());
                                    me.publish(s.png, FrameFormat::Png, s.width, s.height, SourceKind::Usb);
                                }
                            }
                            Err(e) => {
                                me.inner.usb_errors.fetch_add(1, Ordering::SeqCst);
                                *me.inner.last_error.lock().unwrap() = Some(e.to_string());
                            }
                        }
                    }
                    let spent = started.elapsed();
                    std::thread::sleep(interval.saturating_sub(spent).max(Duration::from_millis(10)));
                }
            })
            .expect("spawn mirror-usb");

        if let Some(Video { kind, feed }) = video {
            let me = m.clone();
            let run = move || {
                let mut last: Option<Vec<u8>> = None;
                match feed {
                    Feed::Frames(rx) => loop {
                        if me.inner.stop.load(Ordering::SeqCst) {
                            return;
                        }
                        match rx.recv_timeout(Duration::from_millis(200)) {
                            Ok(jpeg) => me.on_video_frame(kind, jpeg, &mut last),
                            Err(RecvTimeoutError::Timeout) => {}
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    },
                    Feed::Mjpeg(open) => {
                        let mut pause = REOPEN_FIRST;
                        while !me.inner.stop.load(Ordering::SeqCst) {
                            let mut frames = 0u64;
                            let res = open().and_then(|addr| {
                                mjpeg::read_http(&addr, &me.inner.stop, |jpeg| {
                                    frames += 1;
                                    me.on_video_frame(kind, jpeg, &mut last);
                                    true
                                })
                            });
                            if let Err(e) = res {
                                *me.inner.video_error.lock().unwrap() = Some(e.to_string());
                            }
                            pause = if frames > 0 { REOPEN_FIRST } else { (pause * 2).min(REOPEN_MAX) };
                            let until = Instant::now() + pause;
                            while Instant::now() < until && !me.inner.stop.load(Ordering::SeqCst) {
                                std::thread::sleep(Duration::from_millis(50));
                            }
                        }
                    }
                }
            };
            std::thread::Builder::new().name("mirror-video".into()).spawn(run).expect("spawn mirror-video");
        }

        // Policy ticker (detects the video going quiet).
        let me = m.clone();
        std::thread::Builder::new()
            .name("mirror-policy".into())
            .spawn(move || {
                while !me.inner.stop.load(Ordering::SeqCst) {
                    me.tick();
                    std::thread::sleep(Duration::from_millis(100));
                }
            })
            .expect("spawn mirror-policy");
        m
    }

    /// A video frame arrived: it keeps the video source healthy, and is shown while that
    /// source is active and the picture changed (WDA's stream repeats static screens).
    fn on_video_frame(&self, kind: SourceKind, jpeg: Vec<u8>, last: &mut Option<Vec<u8>>) {
        self.inner.policy.lock().unwrap().on_video_frame(Instant::now());
        self.tick();
        if self.active() != kind {
            return;
        }
        let changed = last.as_ref() != Some(&jpeg) || self.latest().is_none_or(|f| f.source != kind);
        if changed {
            let (w, h) = frame::jpeg_dimensions(&jpeg).unwrap_or((0, 0));
            *last = Some(jpeg.clone());
            self.publish(jpeg, FrameFormat::Jpeg, w, h, kind);
        }
    }

    fn tick(&self) {
        let sw = self.inner.policy.lock().unwrap().tick(Instant::now());
        if let Some(s) = sw {
            self.inner.switches.lock().unwrap().push(s);
            self.inner.cv.notify_all();
        }
    }

    /// Show a frame from `source`, unless another source became active meanwhile (a slow
    /// screenshot must not replace the video that just took over).
    fn publish(&self, data: Vec<u8>, format: FrameFormat, width: u32, height: u32, source: SourceKind) {
        let policy = self.inner.policy.lock().unwrap();
        if policy.active() != source {
            return;
        }
        let seq = self.inner.seq.fetch_add(1, Ordering::SeqCst) + 1;
        *self.inner.latest.lock().unwrap() = Some(Frame { data, format, width, height, seq, source, at: Instant::now() });
        drop(policy);
        self.inner.cv.notify_all();
    }

    pub fn active(&self) -> SourceKind {
        self.inner.policy.lock().unwrap().active()
    }

    pub fn latest(&self) -> Option<Frame> {
        self.inner.latest.lock().unwrap().clone()
    }

    /// Wait for a frame newer than `after_seq` (0 = any).
    pub fn wait_next(&self, after_seq: u64, timeout: Duration) -> Option<Frame> {
        let deadline = Instant::now() + timeout;
        let mut g = self.inner.latest.lock().unwrap();
        loop {
            if let Some(f) = g.as_ref() {
                if f.seq > after_seq {
                    return Some(f.clone());
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            g = self.inner.cv.wait_timeout(g, deadline - now).unwrap().0;
        }
    }

    pub fn set_mode(&self, mode: Mode) {
        let sw = self.inner.policy.lock().unwrap().set_mode(mode, Instant::now());
        if let Some(s) = sw {
            self.inner.switches.lock().unwrap().push(s);
        }
    }

    pub fn status(&self) -> MirrorStatus {
        let p = self.inner.policy.lock().unwrap();
        MirrorStatus {
            active: p.active(),
            mode: p.mode,
            video: self.inner.video,
            video_healthy: p.video_healthy(Instant::now()),
            video_error: self.inner.video_error.lock().unwrap().clone(),
            frames: self.inner.seq.load(Ordering::SeqCst),
            usb_errors: self.inner.usb_errors.load(Ordering::SeqCst),
            last_error: self.inner.last_error.lock().unwrap().clone(),
            switches: self.inner.switches.lock().unwrap().clone(),
        }
    }

    pub fn stop(&self) {
        self.inner.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use swiftwin_device::{Controller, Device, MockDevice};

    fn usb_from(m: &MockDevice) -> ScreenshotFn {
        let m = m.clone();
        Arc::new(move || m.screenshot())
    }

    #[test]
    fn usb_stream_publishes_changes_only() {
        let phone = MockDevice::new();
        let mirror = Mirror::start(usb_from(&phone), None, Policy::new(Mode::Auto), 20.0);
        let f1 = mirror.wait_next(0, Duration::from_secs(5)).unwrap();
        assert_eq!((f1.format, f1.source, f1.width), (FrameFormat::Png, SourceKind::Usb, 393));
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(mirror.latest().unwrap().seq, f1.seq, "no new frame while the screen is static");
        let icon = phone.ui_tree().unwrap().find("Hello").unwrap().rect.center();
        phone.tap(icon).unwrap();
        let f2 = mirror.wait_next(f1.seq, Duration::from_secs(5)).unwrap();
        assert_ne!(f2.data, f1.data);
        mirror.stop();
    }

    #[test]
    fn falls_back_to_usb_when_airplay_stops_and_recovers() {
        let phone = MockDevice::new();
        let (tx, rx) = channel();
        let mut policy = Policy::new(Mode::Auto);
        policy.stale_after = Duration::from_millis(250);
        policy.recover_after = Duration::from_millis(200);
        let mirror = Mirror::start(usb_from(&phone), Some(Video { kind: SourceKind::AirPlay, feed: Feed::Frames(rx) }), policy, 20.0);
        assert_eq!(mirror.wait_next(0, Duration::from_secs(5)).unwrap().source, SourceKind::Usb);

        let feed = |tx: &std::sync::mpsc::Sender<Vec<u8>>, ms: u64| {
            let end = Instant::now() + Duration::from_millis(ms);
            let mut i = 0u8;
            while Instant::now() < end {
                tx.send(frame::tiny_jpeg(1179, 2556, i)).unwrap();
                i = i.wrapping_add(1);
                std::thread::sleep(Duration::from_millis(30));
            }
        };
        feed(&tx, 1000);
        assert_eq!(mirror.active(), SourceKind::AirPlay);
        let f = mirror.latest().unwrap();
        assert_eq!((f.format, f.width, f.height), (FrameFormat::Jpeg, 1179, 2556));

        // AirPlay stalls -> USB takes over.
        std::thread::sleep(Duration::from_millis(1000));
        assert_eq!(mirror.active(), SourceKind::Usb);
        let f = mirror.wait_next(mirror.latest().unwrap().seq - 1, Duration::from_secs(5)).unwrap();
        assert_eq!(f.source, SourceKind::Usb);

        // ...and AirPlay comes back.
        feed(&tx, 1000);
        assert_eq!(mirror.active(), SourceKind::AirPlay);
        let st = mirror.status();
        let tos: Vec<SourceKind> = st.switches.iter().map(|s| s.to).collect();
        assert_eq!(tos, vec![SourceKind::AirPlay, SourceKind::Usb, SourceKind::AirPlay]);
        mirror.set_mode(Mode::UsbOnly);
        assert_eq!(mirror.active(), SourceKind::Usb);
        mirror.stop();
    }

    #[test]
    fn shows_the_usb_video_stream_and_reopens_it() {
        use std::sync::atomic::AtomicUsize;
        let phone = MockDevice::new();
        // Each open serves the same frame (a static screen) 60 times at ~50 fps, then the
        // stream ends, like WDA restarting; the next open shows a new picture.
        let opens = Arc::new(AtomicUsize::new(0));
        let o = opens.clone();
        let open: OpenStreamFn = Arc::new(move || {
            let n = o.fetch_add(1, Ordering::SeqCst);
            Ok(mjpeg::tests::serve_mjpeg(60, Duration::from_millis(20), move |_| frame::tiny_jpeg(1206, 2622, n as u8)))
        });
        let mut policy = Policy::new(Mode::Auto);
        policy.stale_after = Duration::from_millis(1500);
        policy.recover_after = Duration::from_millis(200);
        let mirror = Mirror::start(usb_from(&phone), Some(Video { kind: SourceKind::Stream, feed: Feed::Mjpeg(open) }), policy, 20.0);
        let deadline = Instant::now() + Duration::from_secs(5);
        while mirror.active() != SourceKind::Stream && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(mirror.active(), SourceKind::Stream);
        let f = mirror.wait_next(0, Duration::from_secs(5)).unwrap();
        let f = if f.source == SourceKind::Stream { f } else { mirror.wait_next(f.seq, Duration::from_secs(5)).unwrap() };
        assert_eq!((f.format, f.source, f.width, f.height), (FrameFormat::Jpeg, SourceKind::Stream, 1206, 2622));
        // Repeated frames of a static screen are not published again.
        let before = mirror.status().frames;
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(mirror.status().frames, before, "static frames were re-published");
        // The stream ends after ~1.2 s and is reopened.
        let deadline = Instant::now() + Duration::from_secs(10);
        while opens.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(opens.load(Ordering::SeqCst) >= 2, "the stream was not reopened");
        let f2 = mirror.wait_next(f.seq, Duration::from_secs(5)).unwrap();
        assert_eq!(f2.source, SourceKind::Stream);
        let st = mirror.status();
        assert_eq!((st.video, st.active), (Some(SourceKind::Stream), SourceKind::Stream), "no fallback while reopening");
        mirror.stop();
    }

    #[test]
    fn falls_back_to_usb_while_the_stream_cannot_open() {
        let phone = MockDevice::new();
        let open: OpenStreamFn = Arc::new(|| Err(swiftwin_core::SwiftWinError::NotFound("WebDriverAgent is not installed".into())));
        let mirror = Mirror::start(usb_from(&phone), Some(Video { kind: SourceKind::Stream, feed: Feed::Mjpeg(open) }), Policy::new(Mode::Auto), 20.0);
        let f = mirror.wait_next(0, Duration::from_secs(5)).unwrap();
        assert_eq!(f.source, SourceKind::Usb);
        std::thread::sleep(Duration::from_millis(100));
        let st = mirror.status();
        assert!(!st.video_healthy);
        assert!(st.video_error.unwrap().contains("not installed"));
        mirror.stop();
    }

    #[test]
    fn usb_errors_are_reported() {
        let failing: ScreenshotFn = Arc::new(|| Err(swiftwin_core::SwiftWinError::NoDevice));
        let mirror = Mirror::start(failing, None, Policy::new(Mode::UsbOnly), 30.0);
        std::thread::sleep(Duration::from_millis(200));
        let st = mirror.status();
        assert!(st.usb_errors > 0);
        assert!(st.last_error.unwrap().contains("iPhone"));
        assert!(mirror.latest().is_none());
        mirror.stop();
    }
}
