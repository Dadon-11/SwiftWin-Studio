//! Streaming decoder for Apple's pbzx format (the `Content` entry of a `.xip`).
//!
//! Layout (big-endian): `"pbzx"`, u64 chunk size (16 MiB); then chunks of
//! `u64 uncompressed size, u64 stored length, <stored bytes>`. A chunk is an XZ
//! stream unless it is stored raw (no XZ magic, or stored length == uncompressed size).
//! The decoded output is a cpio (odc) archive. References: unxip (saagarjha/unxip, whose CI
//! extracts Xcode 26 and diffs the result against Apple's `xip`), and `pbzx.c` by PHPdev32 /
//! NiklasRosenstein.
//!
//! Chunks are independent, so like unxip they are decoded on a few worker threads and handed
//! out in order (single-threaded LZMA is the bottleneck: Xcode's cpio is ~10 GB).
//! `SWIFTWIN_PBZX_THREADS=1` decodes on the calling thread.

use std::collections::BTreeMap;
use std::io::{BufReader, Read};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use swiftwin_core::{Result, SwiftWinError};

const XZ_MAGIC: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0x00];
/// Upper bound on one stored chunk (Apple uses 16 MiB).
const MAX_CHUNK: u64 = 64 * 1024 * 1024;
/// Each thread holds up to two chunks in memory (stored + decoded).
const MAX_THREADS: usize = 6;

struct Job {
    idx: u64,
    data: Vec<u8>,
    size: Option<u64>,
}

type Decoded = (u64, std::io::Result<Vec<u8>>);

struct Pool {
    jobs: Option<Sender<Job>>,
    done: Receiver<Decoded>,
    workers: Vec<JoinHandle<()>>,
}

impl Pool {
    fn start(threads: usize) -> Pool {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel::<Decoded>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        let workers = (0..threads)
            .map(|_| {
                let job_rx = Arc::clone(&job_rx);
                let done_tx = done_tx.clone();
                std::thread::spawn(move || loop {
                    let job = match job_rx.lock() {
                        Ok(rx) => rx.recv(),
                        Err(_) => return,
                    };
                    let Ok(Job { idx, data, size }) = job else { return };
                    // A panic must still answer for this chunk, or the reader would wait forever.
                    let decoded = std::panic::catch_unwind(move || decode_chunk(data, size))
                        .unwrap_or_else(|_| Err(io_err("xz decoder panicked on this chunk")));
                    if done_tx.send((idx, decoded)).is_err() {
                        return;
                    }
                })
            })
            .collect();
        Pool { jobs: Some(job_tx), done, workers }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        // Closing the queue lets every worker finish its chunk and exit.
        self.jobs.take();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

pub struct PbzxReader<R> {
    inner: R,
    buf: Vec<u8>,
    pos: usize,
    /// No more chunks in `inner`.
    eof: bool,
    /// Index of the next chunk to read from `inner` / to hand out.
    next_in: u64,
    next_out: u64,
    /// Decoded chunks waiting for their turn.
    ready: BTreeMap<u64, std::io::Result<Vec<u8>>>,
    /// Chunks in flight at most.
    window: u64,
    pool: Option<Pool>,
}

fn default_threads() -> usize {
    if let Some(n) = std::env::var("SWIFTWIN_PBZX_THREADS").ok().and_then(|v| v.parse::<usize>().ok()) {
        return n;
    }
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

impl<R: Read> PbzxReader<R> {
    pub fn new(inner: R) -> Result<Self> {
        Self::with_threads(inner, default_threads())
    }

    /// `threads` = 1 decodes on the calling thread.
    pub fn with_threads(mut inner: R, threads: usize) -> Result<Self> {
        let mut magic = [0u8; 12];
        inner.read_exact(&mut magic).map_err(|_| SwiftWinError::invalid("Content is too short for pbzx"))?;
        if &magic[0..4] != b"pbzx" {
            return Err(SwiftWinError::invalid("Content is not a pbzx stream"));
        }
        let threads = threads.clamp(1, MAX_THREADS);
        let pool = (threads > 1).then(|| Pool::start(threads));
        let window = if pool.is_some() { threads as u64 + 1 } else { 1 };
        Ok(PbzxReader { inner, buf: Vec::new(), pos: 0, eof: false, next_in: 0, next_out: 0, ready: BTreeMap::new(), window, pool })
    }

    /// The next stored chunk and its uncompressed size (if plausible), or `None` at the end.
    fn read_stored(&mut self) -> std::io::Result<Option<(Vec<u8>, Option<u64>)>> {
        let mut hdr = [0u8; 16];
        match read_full(&mut self.inner, &mut hdr)? {
            0 => return Ok(None),
            16 => {}
            _ => return Err(io_err("truncated pbzx chunk header")),
        }
        let size = u64::from_be_bytes(hdr[0..8].try_into().unwrap());
        let len = u64::from_be_bytes(hdr[8..16].try_into().unwrap());
        if len > MAX_CHUNK {
            return Err(io_err("pbzx chunk too large"));
        }
        let mut data = vec![0u8; len as usize];
        self.inner.read_exact(&mut data)?;
        Ok(Some((data, (size <= MAX_CHUNK).then_some(size))))
    }

    /// Make the next decoded chunk current. Returns false at the end of the stream.
    fn next_chunk(&mut self) -> std::io::Result<bool> {
        while !self.eof && self.next_in - self.next_out < self.window {
            match self.read_stored()? {
                None => self.eof = true,
                Some((data, size)) => {
                    let idx = self.next_in;
                    self.next_in += 1;
                    match &self.pool {
                        Some(pool) => {
                            let tx = pool.jobs.as_ref().ok_or_else(|| io_err("pbzx decoder stopped"))?;
                            tx.send(Job { idx, data, size }).map_err(|_| io_err("pbzx decoder threads stopped"))?;
                        }
                        None => {
                            self.ready.insert(idx, decode_chunk(data, size));
                        }
                    }
                }
            }
        }
        if self.next_out == self.next_in {
            return Ok(false);
        }
        let decoded = loop {
            if let Some(r) = self.ready.remove(&self.next_out) {
                break r;
            }
            let pool = self.pool.as_ref().ok_or_else(|| io_err("pbzx chunk went missing"))?;
            let (idx, r) = pool.done.recv().map_err(|_| io_err("pbzx decoder threads stopped"))?;
            self.ready.insert(idx, r);
        };
        self.next_out += 1;
        self.buf = decoded?;
        self.pos = 0;
        Ok(true)
    }
}

/// Decode one stored chunk; `size` is the uncompressed size from its header.
fn decode_chunk(data: Vec<u8>, size: Option<u64>) -> std::io::Result<Vec<u8>> {
    let raw = !data.starts_with(&XZ_MAGIC) || size == Some(data.len() as u64);
    let out = if raw {
        data
    } else {
        let mut out = Vec::with_capacity(size.unwrap_or(0) as usize);
        // lzma-rs verifies CRC32/CRC64 checks but rejects SHA-256 ("not yet implemented"), so for
        // any other check type walk the blocks ourselves; the size check below still applies.
        let check = data.get(7).map_or(0, |b| b & 0x0F);
        let r = if matches!(check, 0x00 | 0x01 | 0x04) {
            lzma_rs::xz_decompress(&mut BufReader::new(&data[..]), &mut out).map_err(|e| format!("{e:?}"))
        } else {
            decode_xz_blocks(&data, &mut out).map(|_| ())
        };
        r.map_err(|e| io_err(&format!("xz chunk: {e}")))?;
        out
    };
    if let Some(s) = size {
        if out.len() as u64 != s {
            return Err(io_err(&format!("pbzx chunk decoded to {} bytes, its header says {s}", out.len())));
        }
    }
    Ok(out)
}

/// Minimal `.xz` reader for check types lzma-rs cannot verify: decodes each block (LZMA2 only)
/// and skips its check field. Returns the offset of the index, which is not validated.
fn decode_xz_blocks(data: &[u8], out: &mut Vec<u8>) -> std::result::Result<usize, String> {
    // Check field size by check id (xz file format 1.0.4, section 2.1.1.2).
    const CHECK_SIZES: [usize; 16] = [0, 4, 4, 4, 8, 8, 8, 16, 16, 16, 32, 32, 32, 64, 64, 64];
    let check_size = CHECK_SIZES[(data.get(7).copied().unwrap_or(0) & 0x0F) as usize];
    let mut pos = 12; // stream header
    loop {
        let size_byte = *data.get(pos).ok_or("truncated xz stream")?;
        if size_byte == 0 {
            return Ok(pos); // index indicator: no more blocks
        }
        let header_len = (size_byte as usize + 1) * 4;
        let header = data.get(pos..pos + header_len).ok_or("truncated xz block header")?;
        let flags = header[1];
        if flags & 0x03 != 0 {
            return Err("xz block uses more than one filter".into());
        }
        // Skip the optional compressed and uncompressed size fields (multibyte integers).
        let mut p = 2;
        for bit in [0x40u8, 0x80] {
            if flags & bit != 0 {
                while *header.get(p).ok_or("bad xz block header")? & 0x80 != 0 {
                    p += 1;
                }
                p += 1;
            }
        }
        if header.get(p) != Some(&0x21) {
            return Err("xz block filter is not LZMA2".into());
        }
        pos += header_len;
        let mut cur = std::io::Cursor::new(&data[pos..]);
        lzma_rs::lzma2_decompress(&mut cur, out).map_err(|e| format!("{e:?}"))?;
        pos += cur.position() as usize;
        pos = (pos + 3) & !3; // block padding
        pos += check_size;
    }
}

fn io_err(m: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_string())
}

fn read_full(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

impl<R: Read> Read for PbzxReader<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.buf.len() {
            if !self.next_chunk()? {
                return Ok(0);
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

#[cfg(test)]
pub(crate) mod build {
    pub fn xz(data: &[u8]) -> Vec<u8> {
        let mut x = Vec::new();
        lzma_rs::xz_compress(&mut std::io::BufReader::new(data), &mut x).unwrap();
        x
    }

    /// Encode `data` as pbzx the way Apple does: header chunk size, then per chunk
    /// `uncompressed size, stored length`; odd chunks are stored raw.
    pub fn pbzx(data: &[u8], chunk: usize) -> Vec<u8> {
        let mut out = b"pbzx".to_vec();
        out.extend_from_slice(&(chunk as u64).to_be_bytes());
        for (i, c) in data.chunks(chunk).enumerate() {
            let payload = if i % 2 == 0 { xz(c) } else { c.to_vec() };
            out.extend_from_slice(&(c.len() as u64).to_be_bytes());
            out.extend_from_slice(&(payload.len() as u64).to_be_bytes());
            out.extend_from_slice(&payload);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        (0..50_000u32).flat_map(|i| (i % 251).to_le_bytes()).collect()
    }

    #[test]
    fn roundtrip_mixed_chunks() {
        let data = sample();
        let enc = build::pbzx(&data, 30_000);
        for threads in [1, 4] {
            let mut out = Vec::new();
            PbzxReader::with_threads(&enc[..], threads).unwrap().read_to_end(&mut out).unwrap();
            assert_eq!(out, data, "threads = {threads}");
        }
    }

    #[test]
    fn rejects_bad_magic() {
        assert!(PbzxReader::new(&b"nope00000000"[..]).is_err());
    }

    #[test]
    fn size_mismatch_and_corruption_are_errors() {
        let data = sample();
        let mut enc = build::pbzx(&data, 30_000);
        enc[12 + 7] ^= 1; // first chunk's uncompressed size
        let mut out = Vec::new();
        assert!(PbzxReader::with_threads(&enc[..], 3).unwrap().read_to_end(&mut out).is_err());
        let mut enc = build::pbzx(&data, 30_000);
        enc[12 + 16 + 1] ^= 0xFF; // first chunk's XZ magic: now "raw", but longer than its size
        let mut out = Vec::new();
        assert!(PbzxReader::with_threads(&enc[..], 1).unwrap().read_to_end(&mut out).is_err());
    }

    #[test]
    fn decodes_xz_with_a_sha256_check() {
        let data: Vec<u8> = (0..40_000u32).map(|i| (i % 13) as u8).collect();
        let plain = build::xz(&data); // check type None
        let mut out = Vec::new();
        let index = decode_xz_blocks(&plain, &mut out).unwrap();
        assert_eq!(out, data);
        // The same stream with a SHA-256 check (flags byte 0x0A, its CRC32, and a 32-byte check
        // after the block), which lzma-rs cannot verify.
        let mut sha = plain[..12].to_vec();
        sha[7] = 0x0A;
        let mut crc = flate2::Crc::new();
        crc.update(&sha[6..8]);
        sha[8..12].copy_from_slice(&crc.sum().to_le_bytes());
        sha.extend_from_slice(&plain[12..index]);
        sha.extend_from_slice(&[0xAB; 32]);
        sha.extend_from_slice(&plain[index..]);
        assert_eq!(decode_chunk(sha, Some(data.len() as u64)).unwrap(), data);
    }
}
