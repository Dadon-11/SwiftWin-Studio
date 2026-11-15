//! Reader for XAR archives (the container format of `.xip` files).
//!
//! Layout: 28-byte big-endian header (`xar!`, header size, version,
//! compressed TOC length, uncompressed TOC length, checksum algorithm),
//! then a zlib-compressed XML table of contents, then the heap. Each file in
//! the TOC has `<data><offset/><length/><size/><encoding style=.../></data>`
//! with offsets relative to the start of the heap.
//! Reference: https://github.com/mackyle/xar/wiki/xarformat

use std::io::{Read, Seek, SeekFrom};

use flate2::read::ZlibDecoder;
use swiftwin_core::{Result, SwiftWinError};

pub const MAGIC: &[u8; 4] = b"xar!";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XarEntry {
    pub name: String,
    /// Offset in the heap.
    pub offset: u64,
    /// Stored (possibly compressed) length.
    pub length: u64,
    /// Extracted size.
    pub size: u64,
    pub encoding: String,
}

pub struct Xar<R> {
    r: R,
    heap_start: u64,
    pub toc_xml: String,
    pub entries: Vec<XarEntry>,
}

impl<R: Read + Seek> Xar<R> {
    pub fn open(mut r: R) -> Result<Self> {
        let mut h = [0u8; 28];
        r.read_exact(&mut h).map_err(|_| SwiftWinError::invalid("file too short to be a .xip"))?;
        if &h[0..4] != MAGIC {
            return Err(SwiftWinError::invalid("not a XAR/.xip archive (bad magic)"));
        }
        let header_size = u16::from_be_bytes([h[4], h[5]]) as u64;
        let toc_c = u64::from_be_bytes(h[8..16].try_into().unwrap());
        let toc_u = u64::from_be_bytes(h[16..24].try_into().unwrap());
        if toc_c > 64 * 1024 * 1024 || toc_u > 256 * 1024 * 1024 {
            return Err(SwiftWinError::invalid("XAR table of contents is unreasonably large"));
        }
        r.seek(SeekFrom::Start(header_size))?;
        let mut comp = vec![0u8; toc_c as usize];
        r.read_exact(&mut comp)?;
        let mut toc_xml = String::new();
        ZlibDecoder::new(&comp[..])
            .read_to_string(&mut toc_xml)
            .map_err(|e| SwiftWinError::invalid(format!("XAR TOC: {e}")))?;
        let entries = parse_toc(&toc_xml);
        Ok(Xar { r, heap_start: header_size + toc_c, toc_xml, entries })
    }

    pub fn entry(&self, name: &str) -> Option<&XarEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// A reader over the stored bytes of `entry` (no decompression: `.xip`
    /// stores `Content` uncompressed as a pbzx stream).
    pub fn raw_reader(&mut self, entry: &XarEntry) -> Result<std::io::Take<&mut R>> {
        self.r.seek(SeekFrom::Start(self.heap_start + entry.offset))?;
        Ok((&mut self.r).take(entry.length))
    }

    /// Like [`raw_reader`](Self::raw_reader), but takes ownership of the archive.
    pub fn into_raw_reader(mut self, entry: &XarEntry) -> Result<std::io::Take<R>> {
        self.r.seek(SeekFrom::Start(self.heap_start + entry.offset))?;
        Ok(self.r.take(entry.length))
    }
}

/// Extract `<file>` entries (name + data block) from the TOC XML.
/// Only top-level files with data matter for `.xip` (`Content`, `Metadata`).
pub fn parse_toc(xml: &str) -> Vec<XarEntry> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<file") {
        rest = &rest[i + 5..];
        let end = rest.find("</file>").unwrap_or(rest.len());
        // Nested files would appear before </file>; .xip has none, so the slice is fine.
        let block = &rest[..end];
        let tag = |t: &str, within: &str| -> Option<String> {
            let open = format!("<{t}>");
            let s = within.find(&open)? + open.len();
            let e = within[s..].find(&format!("</{t}>"))? + s;
            Some(within[s..e].trim().to_string())
        };
        if let (Some(name), Some(ds)) = (tag("name", block), block.find("<data>")) {
            let data = &block[ds..];
            let num = |t: &str| tag(t, data).and_then(|v| v.parse::<u64>().ok());
            let encoding = data
                .find("<encoding")
                .and_then(|p| {
                    let s = &data[p..];
                    let q = s.find("style=\"")? + 7;
                    let e = s[q..].find('"')? + q;
                    Some(s[q..e].to_string())
                })
                .unwrap_or_default();
            if let (Some(offset), Some(length)) = (num("offset"), num("length")) {
                out.push(XarEntry { name, offset, length, size: num("size").unwrap_or(length), encoding });
            }
        }
        rest = &rest[end..];
    }
    out
}

#[cfg(test)]
pub(crate) mod build {
    //! Writes small XAR archives for tests.
    use flate2::write::ZlibEncoder;
    use std::io::Write;

    pub fn xar(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut heap = Vec::new();
        let mut toc = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xar><toc><checksum style=\"sha1\"><offset>0</offset><size>20</size></checksum>");
        heap.extend_from_slice(&[0u8; 20]); // fake checksum
        for (i, (name, data)) in files.iter().enumerate() {
            let off = heap.len();
            heap.extend_from_slice(data);
            toc.push_str(&format!(
                "<file id=\"{}\"><data><length>{}</length><encoding style=\"application/octet-stream\"/><offset>{}</offset><size>{}</size></data><type>file</type><name>{}</name></file>",
                i + 1, data.len(), off, data.len(), name
            ));
        }
        toc.push_str("</toc></xar>");
        let mut z = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(toc.as_bytes()).unwrap();
        let ctoc = z.finish().unwrap();
        let mut out = Vec::new();
        out.extend_from_slice(b"xar!");
        out.extend_from_slice(&28u16.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&(ctoc.len() as u64).to_be_bytes());
        out.extend_from_slice(&(toc.len() as u64).to_be_bytes());
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&ctoc);
        out.extend_from_slice(&heap);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reads_entries() {
        let data = build::xar(&[("Metadata", b"<plist/>"), ("Content", b"pbzx-data")]);
        let mut x = Xar::open(Cursor::new(data)).unwrap();
        assert_eq!(x.entries.len(), 2);
        let e = x.entry("Content").unwrap().clone();
        assert_eq!(e.encoding, "application/octet-stream");
        let mut s = String::new();
        x.raw_reader(&e).unwrap().read_to_string(&mut s).unwrap();
        assert_eq!(s, "pbzx-data");
        let mut again = String::new();
        x.into_raw_reader(&e).unwrap().read_to_string(&mut again).unwrap();
        assert_eq!(again, "pbzx-data");
    }

    #[test]
    fn rejects_non_xar() {
        assert!(Xar::open(Cursor::new(b"PK\x03\x04 not a xar at all......".to_vec())).is_err());
    }
}
