//! MJPEG: splitting a byte stream into JPEG frames, and reading an MJPEG-over-HTTP
//! server such as WebDriverAgent's screen stream (device port 9100, forwarded over USB).
//!
//! Confirmed on iOS 27.0.1 (iPhone18,3) with appium WebDriverAgent v16.15.3: `GET /` answers
//! `multipart/x-mixed-replace` with one full-size JPEG (1206x2622, ~150 KB at quality 25)
//! per part, at ~9 fps by default and ~23 fps with `mjpegServerFramerate` raised to 30,
//! whether or not the screen changes.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use swiftwin_core::{Result, SwiftWinError};

/// Splits a byte stream into complete JPEG images. Only relies on the JPEG SOI/EOI
/// markers, so it works with or without multipart headers.
#[derive(Default)]
pub struct MjpegParser {
    buf: Vec<u8>,
}

impl MjpegParser {
    pub fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        loop {
            let Some(soi) = find(&self.buf, &[0xFF, 0xD8, 0xFF]) else {
                // Keep the last two bytes in case a marker is split across reads.
                let keep = self.buf.len().min(2);
                self.buf.drain(..self.buf.len() - keep);
                break;
            };
            let Some(eoi) = find(&self.buf[soi + 2..], &[0xFF, 0xD9]).map(|e| e + soi + 2) else {
                self.buf.drain(..soi);
                break;
            };
            out.push(self.buf[soi..eoi + 2].to_vec());
            self.buf.drain(..eoi + 2);
        }
        if self.buf.len() > 32 * 1024 * 1024 {
            self.buf.clear(); // corrupt stream; resync
        }
        out
    }
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

/// A stream that delivers nothing for this long is treated as stalled.
pub const STALL_AFTER: Duration = Duration::from_secs(5);

/// Read the MJPEG stream at `addr` (`host:port`), calling `on_frame` for every JPEG.
/// Returns when the server closes the stream, `stop` is set, `on_frame` returns `false`,
/// or nothing arrives for [`STALL_AFTER`].
pub fn read_http(
    addr: &str,
    stop: &AtomicBool,
    mut on_frame: impl FnMut(Vec<u8>) -> bool,
) -> Result<()> {
    let sock = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| SwiftWinError::invalid(format!("bad stream address {addr}")))?;
    let mut s = TcpStream::connect_timeout(&sock, Duration::from_secs(5))?;
    s.set_read_timeout(Some(Duration::from_millis(500)))?;
    s.set_nodelay(true)?;
    s.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nAccept: multipart/x-mixed-replace\r\n\r\n")?;
    let mut parser = MjpegParser::default();
    let mut buf = vec![0u8; 256 * 1024];
    let mut last_data = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        match s.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                last_data = Instant::now();
                for jpeg in parser.push(&buf[..n]) {
                    if !on_frame(jpeg) {
                        return Ok(());
                    }
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if last_data.elapsed() > STALL_AFTER {
                    return Err(SwiftWinError::Timeout(format!(
                        "the video stream at {addr} sent nothing for {}s",
                        STALL_AFTER.as_secs()
                    )));
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::frame::tiny_jpeg;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    #[test]
    fn parser_handles_split_and_multipart() {
        let mut stream = Vec::new();
        for i in 0..3 {
            let j = tiny_jpeg(100, 200, i);
            stream.extend_from_slice(
                format!(
                    "--swiftwin\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                    j.len()
                )
                .as_bytes(),
            );
            stream.extend_from_slice(&j);
            stream.extend_from_slice(b"\r\n");
        }
        let mut p = MjpegParser::default();
        let mut got = Vec::new();
        for chunk in stream.chunks(7) {
            got.extend(p.push(chunk));
        }
        assert_eq!(got.len(), 3);
        assert_eq!(got[2], tiny_jpeg(100, 200, 2));
    }

    /// An MJPEG server shaped like WebDriverAgent's: answers `GET /` with a multipart
    /// stream of `frames` JPEGs (`frame(i)`), one every `every`, then closes.
    pub(crate) fn serve_mjpeg(
        frames: usize,
        every: Duration,
        frame: impl Fn(usize) -> Vec<u8> + Send + 'static,
    ) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let Ok((s, _)) = l.accept() else { return };
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            while r.read_line(&mut line).unwrap_or(0) > 2 {
                line.clear();
            }
            let mut s = s;
            let head = "HTTP/1.1 200 OK\r\nServer: WDA MJPEG Server\r\nConnection: close\r\nMax-Age: 0\r\nExpires: 0\r\n\
                        Cache-Control: no-cache, private\r\nPragma: no-cache\r\n\
                        Content-Type: multipart/x-mixed-replace; boundary=--BoundaryString\r\n\r\n";
            if s.write_all(head.as_bytes()).is_err() {
                return;
            }
            for i in 0..frames {
                let j = frame(i);
                let part = format!(
                    "--BoundaryString\r\nContent-type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                    j.len()
                );
                if s.write_all(part.as_bytes())
                    .and_then(|_| s.write_all(&j))
                    .and_then(|_| s.write_all(b"\r\n\r\n"))
                    .is_err()
                {
                    return;
                }
                std::thread::sleep(every);
            }
        });
        addr
    }

    #[test]
    fn reads_an_http_mjpeg_stream_until_it_ends() {
        let addr = serve_mjpeg(5, Duration::from_millis(10), |i| {
            tiny_jpeg(1206, 2622, i as u8)
        });
        let mut got = Vec::new();
        read_http(&addr, &AtomicBool::new(false), |j| {
            got.push(j);
            true
        })
        .unwrap();
        assert_eq!(got.len(), 5);
        assert_eq!(crate::frame::jpeg_dimensions(&got[4]), Some((1206, 2622)));
    }

    #[test]
    fn stops_when_asked() {
        let addr = serve_mjpeg(1000, Duration::from_millis(5), |i| {
            tiny_jpeg(10, 20, i as u8)
        });
        let mut n = 0;
        read_http(&addr, &AtomicBool::new(false), |_| {
            n += 1;
            n < 3
        })
        .unwrap();
        assert_eq!(n, 3);
        let addr = serve_mjpeg(1000, Duration::from_millis(5), |i| {
            tiny_jpeg(10, 20, i as u8)
        });
        read_http(&addr, &AtomicBool::new(true), |_| {
            panic!("stopped before reading")
        })
        .unwrap();
    }

    #[test]
    fn a_closed_port_is_an_error() {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(read_http(
            &format!("127.0.0.1:{port}"),
            &AtomicBool::new(false),
            |_| true
        )
        .is_err());
    }
}
