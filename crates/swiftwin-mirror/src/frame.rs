use std::time::Instant;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// USB screenshots: exact pixels, a few frames per second (Claude's view).
    Usb,
    /// WebDriverAgent's MJPEG screen stream over USB: ~25 fps video (the user's view).
    Stream,
    /// AirPlay screen mirroring: smooth video (the user's view).
    AirPlay,
}

impl SourceKind {
    /// Name for messages.
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Usb => "USB screenshots",
            SourceKind::Stream => "the USB video stream",
            SourceKind::AirPlay => "AirPlay",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FrameFormat {
    Png,
    Jpeg,
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub data: Vec<u8>,
    pub format: FrameFormat,
    pub width: u32,
    pub height: u32,
    pub seq: u64,
    pub source: SourceKind,
    pub at: Instant,
}

impl Frame {
    pub fn mime(&self) -> &'static str {
        match self.format {
            FrameFormat::Png => "image/png",
            FrameFormat::Jpeg => "image/jpeg",
        }
    }
}

/// Width/height from a baseline or progressive JPEG (SOF0/1/2 marker).
pub fn jpeg_dimensions(d: &[u8]) -> Option<(u32, u32)> {
    if d.len() < 4 || d[0] != 0xFF || d[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 9 < d.len() {
        if d[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = d[i + 1];
        if m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) || m == 0xFF {
            i += if m == 0xFF { 1 } else { 2 };
            continue;
        }
        let len = u16::from_be_bytes([d[i + 2], d[i + 3]]) as usize;
        if (0xC0..=0xC2).contains(&m) {
            let h = u16::from_be_bytes([d[i + 5], d[i + 6]]) as u32;
            let w = u16::from_be_bytes([d[i + 7], d[i + 8]]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

#[cfg(test)]
pub(crate) fn tiny_jpeg(w: u16, h: u16, salt: u8) -> Vec<u8> {
    let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, b'J', salt];
    v.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08]);
    v.extend_from_slice(&h.to_be_bytes());
    v.extend_from_slice(&w.to_be_bytes());
    v.extend_from_slice(&[0x01, 0x01, 0x11, 0x00]);
    v.extend_from_slice(&[0xFF, 0xD9]);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_jpeg_size() {
        assert_eq!(jpeg_dimensions(&tiny_jpeg(1179, 2556, 1)), Some((1179, 2556)));
        assert_eq!(jpeg_dimensions(b"nope"), None);
    }
}
