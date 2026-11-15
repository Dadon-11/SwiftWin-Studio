//! Streaming extractor for "odc" (portable ASCII, magic `070707`) cpio archives,
//! the payload format inside Xcode's pbzx `Content`.
//!
//! Header: 76 ASCII octal bytes – magic(6) dev(6) ino(6) mode(6) uid(6) gid(6)
//! nlink(6) rdev(6) mtime(11) namesize(6) filesize(11) – then the name
//! (NUL-terminated, `namesize` bytes) and `filesize` bytes of data.
//! Symlink targets are stored as data. The archive ends with `TRAILER!!!`.
//!
//! Hard links matter: unxip's CI run on Xcode 26 beta 5 reports "97302 files, 26175
//! directories, 9665 symlinks, and 25560 hardlinks". Entries that share `(dev, ino)` are one
//! file. Like unxip and GNU cpio, a later entry is linked to the first copy written and its own
//! data (if any) is ignored. When the data sat only in an entry that was not selected, the
//! selected one is left as an empty placeholder and listed in [`Extracted::missing`], and a
//! second pass over the archive ([`fill_missing`]) writes it.
//!
//! Apple's own convention (seen in Xcode 26.3's `.xip`: 25,609 of 159,279 entries): the first
//! entry of an inode carries the data, every later one carries the full size but holds
//! `NULLcanary` followed by zeros, and `nlink` is 0. Such an entry is treated like a link with
//! no data of its own, never written as content.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use swiftwin_core::{Result, SwiftWinError};

const S_IFMT: u32 = 0o170000;
const S_IFDIR: u32 = 0o040000;
const S_IFREG: u32 = 0o100000;
const S_IFLNK: u32 = 0o120000;

/// Start of the data Apple stores for a hard link whose real data is in another entry.
const CANARY: &[u8] = b"NULLcanary";

/// Read the first bytes of an entry's data, enough to recognise [`CANARY`].
fn read_head(data: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(CANARY.len());
    (&mut *data).take(CANARY.len() as u64).read_to_end(&mut head)?;
    Ok(head)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Symlink(String),
    Other,
}

#[derive(Debug, Default, Clone)]
pub struct ExtractStats {
    pub files: u64,
    pub dirs: u64,
    pub symlinks: u64,
    pub hardlinks: u64,
    pub bytes: u64,
    pub skipped: u64,
}

/// A pending symlink to materialise after extraction (Windows can't always create symlinks).
#[derive(Debug, Clone)]
pub struct PendingLink {
    pub at: PathBuf,
    pub target: String,
}

/// `(dev, ino)` of an archive entry: entries with the same key are hard links of one file.
pub type InodeKey = (u64, u64);

#[derive(Debug, Default)]
pub struct Extracted {
    pub stats: ExtractStats,
    pub links: Vec<PendingLink>,
    /// Selected hard links whose data only appeared in entries that were not selected:
    /// placeholders (written empty) to fill with [`fill_missing`].
    pub missing: HashMap<InodeKey, Vec<PathBuf>>,
}

fn octal(b: &[u8]) -> Result<u64> {
    let s = std::str::from_utf8(b).map_err(|_| SwiftWinError::invalid("cpio: non-ASCII header"))?;
    u64::from_str_radix(s.trim_matches(|c: char| c == '\0' || c == ' '), 8)
        .map_err(|_| SwiftWinError::invalid(format!("cpio: bad octal field {s:?}")))
}

/// A file name Windows cannot store as-is: a reserved device name (`CON`, `aux.h`, `COM1`…,
/// with or without extension), a character from `<>:"|?*\` or a control character (`:` would
/// even write an NTFS alternate data stream), or a trailing dot or space.
pub fn windows_unsafe(component: &str) -> bool {
    if component.chars().any(|c| (c as u32) < 0x20 || "<>:\"|?*\\".contains(c)) {
        return true;
    }
    if component.ends_with('.') || component.ends_with(' ') {
        return true;
    }
    let stem = component.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let numbered = (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && matches!(stem.as_bytes()[3], b'1'..=b'9');
    numbered || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
}

/// Make an archive path safe to join under a destination: no `..`, no absolute paths, and no
/// names Windows cannot store (see [`windows_unsafe`]); `None` means skip the entry.
pub fn sanitize(name: &str) -> Option<PathBuf> {
    // A `\` is part of a file name in the archive but a separator on Windows.
    if name.contains('\\') {
        return None;
    }
    let mut p = PathBuf::new();
    for c in Path::new(name.trim_start_matches("./")).components() {
        match c {
            Component::Normal(x) if !windows_unsafe(&x.to_string_lossy()) => p.push(x),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if p.as_os_str().is_empty() {
        None
    } else {
        Some(p)
    }
}

struct Header {
    dev: u64,
    ino: u64,
    mode: u32,
    filesize: u64,
    name: String,
}

impl Header {
    fn key(&self) -> InodeKey {
        (self.dev, self.ino)
    }

    fn kind(&self) -> Kind {
        match self.mode & S_IFMT {
            S_IFDIR => Kind::Dir,
            S_IFREG => Kind::File,
            S_IFLNK => Kind::Symlink(String::new()),
            _ => Kind::Other,
        }
    }
}

/// The next entry's header and name, or `None` at the trailer.
fn read_header(r: &mut impl Read) -> Result<Option<Header>> {
    let mut hdr = [0u8; 76];
    r.read_exact(&mut hdr).map_err(|e| SwiftWinError::invalid(format!("cpio: truncated header ({e})")))?;
    if &hdr[0..6] != b"070707" {
        return Err(SwiftWinError::invalid("cpio: bad magic (only odc/070707 is supported)"));
    }
    let namesize = octal(&hdr[59..65])? as usize;
    if namesize == 0 || namesize > 64 * 1024 {
        return Err(SwiftWinError::invalid(format!("cpio: bad name size {namesize}")));
    }
    let mut name = vec![0u8; namesize];
    r.read_exact(&mut name)?;
    let name = String::from_utf8_lossy(name.strip_suffix(&[0]).unwrap_or(&name)).into_owned();
    if name == "TRAILER!!!" {
        return Ok(None);
    }
    Ok(Some(Header {
        dev: octal(&hdr[6..12])?,
        ino: octal(&hdr[12..18])?,
        mode: octal(&hdr[18..24])? as u32,
        filesize: octal(&hdr[65..76])?,
        name,
    }))
}

/// Write `data` to a fresh file at `dest`; returns the bytes written.
fn write_file(dest: &Path, data: &mut impl Read, mode: u32) -> io::Result<u64> {
    // Never write through an existing hard link: that would change the other names too.
    let _ = std::fs::remove_file(dest);
    let mut f = io::BufWriter::new(std::fs::File::create(dest)?);
    let n = io::copy(data, &mut f)?;
    f.flush()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dest, std::fs::Permissions::from_mode(mode & 0o777));
    }
    #[cfg(not(unix))]
    let _ = mode;
    Ok(n)
}

/// `dst` becomes another name for `src` (a hard link: no privilege needed on NTFS), or a copy
/// where that fails (other volume, or NTFS's 1023-links-per-file limit).
fn link_or_copy(src: &Path, dst: &Path) -> io::Result<()> {
    let _ = std::fs::remove_file(dst);
    if std::fs::hard_link(src, dst).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dst).map(|_| ())
}

/// Walk the archive. For each entry `select(path)` returns where to write it
/// (or `None` to skip). Symlinks are returned for later resolution.
pub fn extract<R: Read>(
    mut r: R,
    mut select: impl FnMut(&str) -> Option<PathBuf>,
    mut progress: impl FnMut(&ExtractStats),
) -> Result<Extracted> {
    let mut stats = ExtractStats::default();
    let mut links = Vec::new();
    // First fully written copy of each inode.
    let mut written: HashMap<InodeKey, PathBuf> = HashMap::new();
    // Selected entries written empty that may still get their data from a later entry.
    let mut waiting: HashMap<InodeKey, Vec<PathBuf>> = HashMap::new();
    // Inodes whose data went by in an entry that was not selected.
    let mut skipped_data: HashSet<InodeKey> = HashSet::new();
    let mut n: u64 = 0;
    while let Some(h) = read_header(&mut r)? {
        let dest = select(&h.name);
        let mut data = (&mut r).take(h.filesize);
        match (h.kind(), dest) {
            (Kind::Dir, Some(d)) => {
                std::fs::create_dir_all(&d)?;
                stats.dirs += 1;
            }
            (Kind::File, Some(d)) => {
                if let Some(parent) = d.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let head = read_head(&mut data)?;
                if let Some(src) = written.get(&h.key()) {
                    link_or_copy(src, &d)?;
                    stats.hardlinks += 1;
                } else if h.filesize > 0 && head != CANARY {
                    stats.bytes += write_file(&d, &mut (&head[..]).chain(&mut data), h.mode)?;
                    stats.files += 1;
                    for w in waiting.remove(&h.key()).unwrap_or_default() {
                        link_or_copy(&d, &w)?;
                        stats.hardlinks += 1;
                    }
                    written.insert(h.key(), d);
                } else {
                    // A genuinely empty file, or a hard link whose data is in another entry
                    // (no data, or Apple's NULLcanary placeholder).
                    write_file(&d, &mut io::empty(), h.mode)?;
                    stats.files += 1;
                    waiting.entry(h.key()).or_default().push(d);
                }
            }
            (Kind::Symlink(_), Some(d)) => {
                let mut t = String::new();
                data.read_to_string(&mut t)?;
                links.push(PendingLink { at: d, target: t.trim_end_matches('\0').to_string() });
                stats.symlinks += 1;
            }
            (Kind::File, None) => {
                if h.filesize > 0 {
                    let head = read_head(&mut data)?;
                    if head == CANARY {
                        // A placeholder: carries nothing for anyone.
                    } else if let Some(mut ws) = waiting.remove(&h.key()) {
                        // The data for selected links that came earlier (and were written empty).
                        let first = ws.remove(0);
                        stats.bytes += write_file(&first, &mut (&head[..]).chain(&mut data), h.mode)?;
                        for w in ws {
                            link_or_copy(&first, &w)?;
                        }
                        written.insert(h.key(), first);
                    } else {
                        skipped_data.insert(h.key());
                    }
                }
                stats.skipped += 1;
            }
            _ => stats.skipped += 1,
        }
        io::copy(&mut data, &mut io::sink())?; // drain what was not consumed
        n += 1;
        if n.is_multiple_of(2000) {
            progress(&stats);
        }
    }
    progress(&stats);
    let missing = waiting.into_iter().filter(|(k, _)| skipped_data.contains(k)).collect();
    Ok(Extracted { stats, links, missing })
}

/// Second pass over the same archive: write the data of each inode in `missing` into its
/// placeholders, removing it from the map. Stops once the map is empty; what is left had no
/// data anywhere. Returns the bytes written.
pub fn fill_missing<R: Read>(mut r: R, missing: &mut HashMap<InodeKey, Vec<PathBuf>>) -> Result<u64> {
    let mut bytes = 0;
    while !missing.is_empty() {
        let Some(h) = read_header(&mut r)? else { break };
        let mut data = (&mut r).take(h.filesize);
        if h.kind() == Kind::File && h.filesize > 0 && missing.contains_key(&h.key()) {
            let head = read_head(&mut data)?;
            if head != CANARY {
                let mut ws = missing.remove(&h.key()).unwrap_or_default();
                let first = ws.remove(0);
                bytes += write_file(&first, &mut (&head[..]).chain(&mut data), h.mode)?;
                for w in ws {
                    link_or_copy(&first, &w)?;
                }
            }
        }
        io::copy(&mut data, &mut io::sink())?;
    }
    Ok(bytes)
}

/// Resolve `.` and `..` without touching the disk.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Create the symlinks once everything is in its final place under `root`. A real symlink on
/// Windows needs Developer Mode or admin rights, so files become hard links (else symlinks,
/// else copies) and folders become symlinks (else NTFS junctions, else copies). Links whose
/// target is missing or outside `root` are returned unresolved. Repeats until no more progress
/// (links to links).
pub fn materialize_links(links: Vec<PendingLink>, root: &Path) -> Result<Vec<PendingLink>> {
    let root = normalize(root);
    let mut pending = links;
    loop {
        let before = pending.len();
        let mut still = Vec::new();
        for l in pending {
            if std::fs::symlink_metadata(&l.at).is_ok() {
                continue;
            }
            let target = normalize(&l.at.parent().unwrap_or(Path::new(".")).join(&l.target));
            if l.target.is_empty() || !target.starts_with(&root) || !target.exists() {
                still.push(l);
                continue;
            }
            if let Some(p) = l.at.parent() {
                std::fs::create_dir_all(p)?;
            }
            make_link(&l.target, &target, &l.at)?;
        }
        if still.is_empty() || still.len() == before {
            return Ok(still);
        }
        pending = still;
    }
}

fn make_link(rel: &str, target: &Path, at: &Path) -> io::Result<()> {
    if target.is_dir() {
        if symlink_dir(rel, at) || junction(target, at) {
            return Ok(());
        }
        copy_any(target, at)
    } else {
        if std::fs::hard_link(target, at).is_ok() || symlink_file(rel, at) {
            return Ok(());
        }
        std::fs::copy(target, at).map(|_| ())
    }
}

/// The link text as the OS expects it: on Windows `/` is not a separator inside a reparse
/// point, so a relative symlink to `../x/y` would be created but never resolve.
fn native_target(rel: &str) -> String {
    if cfg!(windows) {
        rel.replace('/', "\\")
    } else {
        rel.to_string()
    }
}

fn symlink_dir(rel: &str, at: &Path) -> bool {
    #[cfg(unix)]
    let ok = std::os::unix::fs::symlink(native_target(rel), at).is_ok();
    #[cfg(windows)]
    let ok = std::os::windows::fs::symlink_dir(native_target(rel), at).is_ok();
    #[cfg(not(any(unix, windows)))]
    let ok = {
        let _ = native_target(rel);
        false
    };
    keep_if_resolves(ok, at, true)
}

fn symlink_file(rel: &str, at: &Path) -> bool {
    #[cfg(unix)]
    let ok = std::os::unix::fs::symlink(native_target(rel), at).is_ok();
    #[cfg(windows)]
    let ok = std::os::windows::fs::symlink_file(native_target(rel), at).is_ok();
    #[cfg(not(any(unix, windows)))]
    let ok = {
        let _ = native_target(rel);
        false
    };
    keep_if_resolves(ok, at, false)
}

/// An NTFS junction: a folder link that needs no privilege. Its target is absolute, which is
/// why links are only made after the SDK reached its final folder.
fn junction(target: &Path, at: &Path) -> bool {
    #[cfg(windows)]
    {
        let ok = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(at)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        keep_if_resolves(ok, at, true)
    }
    #[cfg(not(windows))]
    {
        let _ = (target, at);
        false
    }
}

/// Keep a freshly created link only if it resolves to the right kind of entry.
fn keep_if_resolves(created: bool, at: &Path, dir: bool) -> bool {
    if !created {
        return false;
    }
    if (dir && at.is_dir()) || (!dir && at.is_file()) {
        return true;
    }
    // A folder link is removed with remove_dir on Windows, any symlink with remove_file on Unix.
    let _ = std::fs::remove_dir(at);
    let _ = std::fs::remove_file(at);
    false
}

/// Copy a file or a whole folder, following links, but never back into a folder being copied
/// (a link to an ancestor would otherwise recurse forever).
pub fn copy_any(from: &Path, to: &Path) -> io::Result<()> {
    copy_tree(from, to, &mut Vec::new())
}

fn copy_tree(from: &Path, to: &Path, ancestors: &mut Vec<PathBuf>) -> io::Result<()> {
    if !from.is_dir() {
        return std::fs::copy(from, to).map(|_| ());
    }
    let real = std::fs::canonicalize(from)?;
    if ancestors.contains(&real) {
        return Ok(());
    }
    ancestors.push(real);
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        copy_tree(&e.path(), &to.join(e.file_name()), ancestors)?;
    }
    ancestors.pop();
    Ok(())
}

#[cfg(test)]
pub(crate) mod build {
    pub enum E<'a> {
        Dir(&'a str),
        File(&'a str, &'a [u8]),
        Link(&'a str, &'a str),
        /// A regular file sharing inode `ino` with the other `Hard` entries using it
        /// (`data` may be empty, as for a link whose data is stored with another entry).
        Hard(&'a str, u64, &'a [u8]),
    }

    fn push(out: &mut Vec<u8>, name: &str, ino: u64, mode: u32, nlink: u32, data: &[u8]) {
        let hdr = format!(
            "070707{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:011o}{:06o}{:011o}",
            1, ino, mode, 0, 0, nlink, 0, 0, name.len() + 1, data.len()
        );
        assert_eq!(hdr.len(), 76);
        out.extend_from_slice(hdr.as_bytes());
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        out.extend_from_slice(data);
    }

    /// An odc archive; every entry except `Hard` gets its own inode (1000 + index).
    pub fn cpio(entries: &[E]) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            let ino = 1000 + i as u64;
            match e {
                E::Dir(n) => push(&mut out, n, ino, 0o040755, 1, b""),
                E::File(n, d) => push(&mut out, n, ino, 0o100644, 1, d),
                E::Link(n, t) => push(&mut out, n, ino, 0o120755, 1, t.as_bytes()),
                E::Hard(n, ino, d) => push(&mut out, n, *ino, 0o100644, 2, d),
            }
        }
        push(&mut out, "TRAILER!!!", 0, 0, 1, b"");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::build::{cpio, E};
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("swiftwin-cpio-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn sanitizes_paths() {
        assert_eq!(sanitize("./a/b"), Some(PathBuf::from("a/b")));
        assert_eq!(sanitize("a/../../etc"), None);
        assert_eq!(sanitize("/abs"), None);
        assert_eq!(sanitize("."), None);
    }

    #[test]
    fn rejects_names_windows_cannot_store() {
        for bad in ["./a/aux.h", "./a/NUL", "./a/com1.txt", "./a/LPT9", "./a/b:c", "./a/q?", "./a/trail.", "./a/sp ", "./a/b\\c", "./a/Icon\r"] {
            assert_eq!(sanitize(bad), None, "{bad:?}");
        }
        for good in ["./a/console.h", "./a/auxv.h", "./a/com10", "./a/.hidden", "./a/x.tbd", "./a/COM0"] {
            assert!(sanitize(good).is_some(), "{good:?}");
        }
    }

    #[test]
    fn extracts_selected_entries_and_links() {
        let arc = cpio(&[
            E::Dir("./keep"),
            E::File("./keep/a.txt", b"hello"),
            E::File("./skip/b.txt", b"nope"),
            E::Link("./keep/link.txt", "a.txt"),
            E::Dir("./keep/dir"),
            E::File("./keep/dir/x", b"x"),
            E::Link("./keep/dirlink", "dir"),
            E::Link("./keep/dir/up", "../a.txt"),
            E::Link("./keep/evil", "../../../../../../outside.txt"),
            E::File("./keep/empty", b""),
        ]);
        let dest = tmp("links");
        let ex = extract(
            &arc[..],
            |n| {
                let p = sanitize(n)?;
                p.starts_with("keep").then(|| dest.join(p))
            },
            |_| {},
        )
        .unwrap();
        assert_eq!(ex.stats.files, 3);
        assert_eq!(ex.stats.skipped, 1);
        assert_eq!(ex.links.len(), 4);
        assert!(ex.missing.is_empty(), "an empty file is not a missing hard link");
        let unresolved = materialize_links(ex.links, &dest).unwrap();
        assert_eq!(unresolved.len(), 1, "a link out of the root stays unresolved");
        assert_eq!(unresolved[0].target, "../../../../../../outside.txt");
        assert_eq!(std::fs::read_to_string(dest.join("keep/link.txt")).unwrap(), "hello");
        assert_eq!(std::fs::read_to_string(dest.join("keep/dirlink/x")).unwrap(), "x");
        assert_eq!(std::fs::read_to_string(dest.join("keep/dir/up")).unwrap(), "hello");
        assert_eq!(std::fs::metadata(dest.join("keep/empty")).unwrap().len(), 0);
        std::fs::remove_dir_all(dest).ok();
    }

    #[test]
    fn hard_links_in_every_layout() {
        let arc = cpio(&[
            // Data with the first entry, both selected.
            E::Hard("./keep/a", 7, b"first"),
            E::Hard("./keep/b", 7, b""),
            // Data with the first entry, which is not selected: needs the second pass.
            E::Hard("./skip/c", 8, b"elsewhere"),
            E::Hard("./keep/d", 8, b""),
            // Data with the last entry, which is not selected.
            E::Hard("./keep/e", 9, b""),
            E::Hard("./skip/f", 9, b"late"),
            // Data repeated in every entry.
            E::Hard("./keep/g", 10, b"same"),
            E::Hard("./keep/h", 10, b"same"),
        ]);
        let dest = tmp("hard");
        let select = |n: &str| {
            let p = sanitize(n)?;
            p.starts_with("keep").then(|| dest.join(p))
        };
        let mut ex = extract(&arc[..], select, |_| {}).unwrap();
        let read = |n: &str| std::fs::read_to_string(dest.join("keep").join(n)).unwrap();
        assert_eq!(read("a"), "first");
        assert_eq!(read("b"), "first");
        assert_eq!(read("e"), "late");
        assert_eq!(read("g"), "same");
        assert_eq!(read("h"), "same");
        assert_eq!(ex.stats.hardlinks, 2);
        assert_eq!(read("d"), "", "placeholder until the second pass");
        assert_eq!(ex.missing.len(), 1);
        assert_eq!(fill_missing(&arc[..], &mut ex.missing).unwrap(), 9);
        assert!(ex.missing.is_empty());
        assert_eq!(read("d"), "elsewhere");
        std::fs::remove_dir_all(dest).ok();
    }

    #[test]
    fn apple_canary_placeholders_are_links_not_content() {
        // Xcode 26.3's layout: the data comes first; later entries of the inode are
        // full-size "NULLcanary" + zeros.
        let real: &[u8] = b"#define X 1\n";
        let mut canary = CANARY.to_vec();
        canary.resize(real.len(), 0);
        let arc = cpio(&[
            // Data in a skipped platform, placeholders in the kept one: second pass.
            E::Hard("./skip/a.h", 20, real),
            E::Hard("./keep/a.h", 20, &canary),
            E::Hard("./keep/a2.h", 20, &canary),
            // A skipped placeholder must not fill the kept ones.
            E::Hard("./skip/a3.h", 20, &canary),
            // Data kept: a later kept placeholder links to it.
            E::Hard("./keep/b.h", 21, b"bbbbbbbbbbbb"),
            E::Hard("./keep/c.h", 21, &canary),
        ]);
        let dest = tmp("canary");
        let select = |n: &str| {
            let p = sanitize(n)?;
            p.starts_with("keep").then(|| dest.join(p))
        };
        let mut ex = extract(&arc[..], select, |_| {}).unwrap();
        let read = |n: &str| std::fs::read(dest.join("keep").join(n)).unwrap();
        assert_eq!(read("c.h"), b"bbbbbbbbbbbb");
        assert_eq!(read("a.h"), b"", "placeholder until the second pass, never the canary");
        assert_eq!(ex.missing.len(), 1);
        assert_eq!(fill_missing(&arc[..], &mut ex.missing).unwrap(), real.len() as u64);
        assert_eq!(read("a.h"), real);
        assert_eq!(read("a2.h"), real);
        std::fs::remove_dir_all(dest).ok();
    }

    #[test]
    fn copy_any_survives_a_link_to_an_ancestor() {
        let d = tmp("cycle");
        std::fs::create_dir_all(d.join("src/sub")).unwrap();
        std::fs::write(d.join("src/sub/f"), b"f").unwrap();
        let looped = materialize_links(vec![PendingLink { at: d.join("src/sub/loop"), target: "..".into() }], &d).unwrap();
        assert!(looped.is_empty());
        copy_any(&d.join("src"), &d.join("dst")).unwrap();
        assert_eq!(std::fs::read(d.join("dst/sub/f")).unwrap(), b"f");
        std::fs::remove_dir_all(d).ok();
    }
}
