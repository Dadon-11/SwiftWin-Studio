//! Which source to show. Pure logic with an injected clock, so it is fully testable.
//!
//! There are two sources: exact USB screenshots, and a smooth video source (`video`:
//! WebDriverAgent's USB stream or AirPlay).
//!
//! * `Auto`: show the video while it is healthy (a frame within `stale_after`);
//!   otherwise fall back to USB screenshots. Switch back to the video only after it has
//!   delivered frames steadily for `recover_after` (hysteresis, no flapping).
//! * `UsbOnly` / `VideoOnly`: fixed.

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::frame::SourceKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Auto,
    UsbOnly,
    VideoOnly,
}

#[derive(Debug, Clone)]
pub struct Policy {
    pub mode: Mode,
    pub stale_after: Duration,
    pub recover_after: Duration,
    /// Which video source this policy switches to (`Stream` or `AirPlay`).
    pub video: SourceKind,
    active: SourceKind,
    last_video: Option<Instant>,
    /// Start of the current unbroken run of video frames.
    video_run_start: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Switch {
    pub from: SourceKind,
    pub to: SourceKind,
    pub reason: String,
}

impl Policy {
    pub fn new(mode: Mode) -> Self {
        Policy {
            mode,
            stale_after: Duration::from_millis(1500),
            recover_after: Duration::from_millis(1000),
            video: SourceKind::AirPlay,
            active: if mode == Mode::VideoOnly { SourceKind::AirPlay } else { SourceKind::Usb },
            last_video: None,
            video_run_start: None,
        }
    }

    /// Use `kind` as the video source.
    pub fn with_video(mut self, kind: SourceKind) -> Self {
        if self.active == self.video {
            self.active = kind;
        }
        self.video = kind;
        self
    }

    pub fn active(&self) -> SourceKind {
        self.active
    }

    pub fn on_video_frame(&mut self, now: Instant) {
        let continuing = self.last_video.is_some_and(|t| now.duration_since(t) <= self.stale_after);
        if !continuing {
            self.video_run_start = Some(now);
        }
        self.last_video = Some(now);
    }

    pub fn video_healthy(&self, now: Instant) -> bool {
        self.last_video.is_some_and(|t| now.duration_since(t) <= self.stale_after)
    }

    /// Re-evaluate; returns a `Switch` when the active source changes.
    pub fn tick(&mut self, now: Instant) -> Option<Switch> {
        let video = self.video;
        let want = match self.mode {
            Mode::UsbOnly => SourceKind::Usb,
            Mode::VideoOnly => video,
            Mode::Auto => {
                let healthy = self.video_healthy(now);
                match self.active {
                    a if a == video && !healthy => SourceKind::Usb,
                    SourceKind::Usb
                        if healthy && self.video_run_start.is_some_and(|s| now.duration_since(s) >= self.recover_after) =>
                    {
                        video
                    }
                    a => a,
                }
            }
        };
        if want == self.active {
            return None;
        }
        let reason = match want {
            SourceKind::Usb if self.mode == Mode::Auto => {
                format!("{} stopped delivering frames; using USB screenshots", video.label())
            }
            v if v == video && self.mode == Mode::Auto => format!("{} is running; switching to smooth video", v.label()),
            _ => "mode changed".into(),
        };
        let s = Switch { from: self.active, to: want, reason };
        self.active = want;
        Some(s)
    }

    pub fn set_mode(&mut self, mode: Mode, now: Instant) -> Option<Switch> {
        self.mode = mode;
        self.tick(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(base: Instant, n: u64) -> Instant {
        base + Duration::from_millis(n)
    }

    #[test]
    fn starts_on_usb_and_upgrades_after_steady_video() {
        let t = Instant::now();
        let mut p = Policy::new(Mode::Auto);
        assert_eq!(p.active(), SourceKind::Usb);
        p.on_video_frame(ms(t, 0));
        assert_eq!(p.tick(ms(t, 10)), None, "one frame is not enough");
        for i in 1..=12 {
            p.on_video_frame(ms(t, i * 100));
        }
        let s = p.tick(ms(t, 1200)).unwrap();
        assert_eq!((s.from, s.to), (SourceKind::Usb, SourceKind::AirPlay));
    }

    #[test]
    fn falls_back_when_video_goes_quiet_and_does_not_flap() {
        let t = Instant::now();
        let mut p = Policy::new(Mode::Auto).with_video(SourceKind::Stream);
        for i in 0..=15 {
            p.on_video_frame(ms(t, i * 100));
        }
        let s = p.tick(ms(t, 1500)).unwrap();
        assert_eq!(p.active(), SourceKind::Stream);
        assert!(s.reason.contains("USB video stream"), "{}", s.reason);
        assert_eq!(p.tick(ms(t, 2500)), None, "1 s of silence is tolerated");
        let s = p.tick(ms(t, 3100)).unwrap();
        assert_eq!(s.to, SourceKind::Usb);
        // A single new frame does not switch back immediately.
        p.on_video_frame(ms(t, 3200));
        assert_eq!(p.tick(ms(t, 3300)), None);
        for i in 1..=10 {
            p.on_video_frame(ms(t, 3200 + i * 100));
        }
        assert_eq!(p.tick(ms(t, 4300)).unwrap().to, SourceKind::Stream);
    }

    #[test]
    fn fixed_modes() {
        let t = Instant::now();
        let mut p = Policy::new(Mode::UsbOnly);
        for i in 0..30 {
            p.on_video_frame(ms(t, i * 100));
        }
        assert_eq!(p.tick(ms(t, 3000)), None);
        assert_eq!(p.set_mode(Mode::VideoOnly, ms(t, 3000)).unwrap().to, SourceKind::AirPlay);
        assert_eq!(p.tick(ms(t, 99_000)), None, "VideoOnly never falls back");
        let p = Policy::new(Mode::VideoOnly).with_video(SourceKind::Stream);
        assert_eq!(p.active(), SourceKind::Stream);
    }
}
