//! AirPlay screen-mirroring receiver.
//!
//! Prototype: run **UxPlay** (GPL-3.0, separate process) as the AirPlay
//! receiver and have its GStreamer video sink re-encode decoded frames as
//! JPEG and push them, multipart-framed, to a local TCP port that SwiftWin
//! listens on:
//!
//! `uxplay -n SwiftWin -nh -vs "jpegenc quality=80 ! multipartmux boundary=swiftwin ! tcpclientsink host=127.0.0.1 port=<p>"`
//!
//! The user starts mirroring from the iPhone (Control Center > Screen
//! Mirroring > "SwiftWin"). Frames are parsed with [`MjpegParser`].

use std::io::Read;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};

use swiftwin_core::{Result, SwiftWinError};

pub use crate::mjpeg::MjpegParser;

/// Listen on 127.0.0.1 (random port) for MJPEG pushed by UxPlay's tcpclientsink.
pub fn listen_frames() -> Result<(u16, Receiver<Vec<u8>>)> {
    let l = TcpListener::bind("127.0.0.1:0")?;
    let port = l.local_addr()?.port();
    let (tx, rx) = channel();
    std::thread::Builder::new().name("airplay-frames".into()).spawn(move || {
        for s in l.incoming().flatten() {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut s = s;
                let mut p = MjpegParser::default();
                let mut buf = vec![0u8; 256 * 1024];
                while let Ok(n) = s.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    for f in p.push(&buf[..n]) {
                        if tx.send(f).is_err() {
                            return;
                        }
                    }
                }
            });
        }
    })?;
    Ok((port, rx))
}

pub fn uxplay_args(receiver_name: &str, frame_port: u16) -> Vec<String> {
    vec![
        "-n".into(),
        receiver_name.into(),
        "-nh".into(),
        "-vs".into(),
        format!("jpegenc quality=80 ! multipartmux boundary=swiftwin ! tcpclientsink host=127.0.0.1 port={frame_port}"),
        "-as".into(),
        "0".into(),
    ]
}

/// `SWIFTWIN_UXPLAY`, `<data>/tools/uxplay/uxplay(.exe)`, then PATH.
pub fn find_uxplay() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SWIFTWIN_UXPLAY") {
        return Some(PathBuf::from(p));
    }
    let exe = if cfg!(windows) { "uxplay.exe" } else { "uxplay" };
    let local = swiftwin_core::paths::tools_dir().join("uxplay").join(exe);
    if local.exists() {
        return Some(local);
    }
    swiftwin_core::paths::find_on_path(exe)
}

/// A running UxPlay receiver plus the stream of JPEG frames it produces.
pub struct AirPlayReceiver {
    child: Child,
    pub frames: Option<Receiver<Vec<u8>>>,
    pub name: String,
}

impl AirPlayReceiver {
    // UNTESTED-ON-DEVICE: UxPlay on Windows (needs Bonjour + GStreamer) and an iPhone mirroring to it.
    pub fn start(receiver_name: &str) -> Result<AirPlayReceiver> {
        let exe = find_uxplay().ok_or_else(|| SwiftWinError::ToolMissing {
            tool: "UxPlay".into(),
            hint: "AirPlay mirroring needs UxPlay + Bonjour (see docs/dependencies.md); USB mirroring works without it".into(),
        })?;
        let (port, rx) = listen_frames()?;
        let child = Command::new(&exe)
            .args(uxplay_args(receiver_name, port))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| SwiftWinError::tool("uxplay", e))?;
        Ok(AirPlayReceiver { child, frames: Some(rx), name: receiver_name.into() })
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for AirPlayReceiver {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::tiny_jpeg;
    use std::io::Write;

    #[test]
    fn tcp_listener_delivers_frames() {
        let (port, rx) = listen_frames().unwrap();
        let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        c.write_all(&tiny_jpeg(10, 20, 1)).unwrap();
        c.write_all(&tiny_jpeg(10, 20, 2)).unwrap();
        let f = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(crate::frame::jpeg_dimensions(&f), Some((10, 20)));
        assert!(rx.recv_timeout(std::time::Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn args() {
        let a = uxplay_args("SwiftWin", 5555);
        assert_eq!(a[0..3], ["-n", "SwiftWin", "-nh"]);
        assert!(a[4].ends_with("tcpclientsink host=127.0.0.1 port=5555"));
    }
}
