//! In-place Mach-O edits needed before linking with the swift.org toolchain's lld.
//!
//! The Swift fork of LLVM makes lld's Mach-O port refuse every input object that
//! carries a platform load command ("This version of lld does not support linking
//! for platform iOS"; `checkCompatibility` in swiftlang/llvm-project
//! `lld/MachO/InputFiles.cpp`, marked "Swift LLVM fork downstream change"). Inputs
//! without one are accepted, and `.tbd` stubs are checked by a separate, unchanged
//! path. So SwiftWin removes `LC_BUILD_VERSION` / `LC_VERSION_MIN_*` from the objects
//! and static archives it links, and gives lld the platform with `-platform_version`,
//! which writes the output's own `LC_BUILD_VERSION`.

use swiftwin_core::{Result, SwiftWinError};

const MH_MAGIC: u32 = 0xfeed_face;
const MH_MAGIC_64: u32 = 0xfeed_facf;
/// Stored big-endian on disk.
const FAT_MAGIC: u32 = 0xcafe_babe;
const FAT_MAGIC_64: u32 = 0xcafe_babf;
const AR_MAGIC: &[u8] = b"!<arch>\n";

/// LC_VERSION_MIN_MACOSX, _IPHONEOS, _TVOS, _WATCHOS and LC_BUILD_VERSION.
pub const PLATFORM_COMMANDS: [u32; 5] = [0x24, 0x25, 0x2f, 0x30, 0x32];

fn le32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}
fn be32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}
fn be64(d: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(d.get(at..at + 8)?.try_into().ok()?))
}
fn bad(what: &str) -> SwiftWinError {
    SwiftWinError::invalid(format!("malformed Mach-O: {what}"))
}

/// Remove the platform load commands from every Mach-O object in `data` (a thin
/// object, a fat file or a static archive), in place. Section offsets don't move:
/// the freed bytes become header padding. Returns how many commands were removed;
/// data that isn't Mach-O is left alone.
pub fn strip_platform_load_commands(data: &mut [u8]) -> Result<usize> {
    if data.starts_with(AR_MAGIC) {
        return strip_archive(data);
    }
    if matches!(be32(data, 0), Some(FAT_MAGIC) | Some(FAT_MAGIC_64)) {
        return strip_fat(data);
    }
    match le32(data, 0) {
        Some(MH_MAGIC_64) => strip_thin(data, 32),
        Some(MH_MAGIC) => strip_thin(data, 28),
        _ => Ok(0),
    }
}

fn strip_thin(data: &mut [u8], header: usize) -> Result<usize> {
    let ncmds = le32(data, 16).ok_or_else(|| bad("header"))?;
    let sizeofcmds = le32(data, 20).ok_or_else(|| bad("header"))? as usize;
    let mut end = header.checked_add(sizeofcmds).filter(|&e| e <= data.len()).ok_or_else(|| bad("load commands"))?;
    let (mut off, mut kept, mut removed) = (header, 0u32, 0usize);
    for _ in 0..ncmds {
        let cmd = le32(data, off).ok_or_else(|| bad("load command"))?;
        let size = le32(data, off + 4).ok_or_else(|| bad("load command"))? as usize;
        if size < 8 || off + size > end {
            return Err(bad("load command size"));
        }
        if PLATFORM_COMMANDS.contains(&cmd) {
            data.copy_within(off + size..end, off);
            data[end - size..end].fill(0);
            end -= size;
            removed += 1;
        } else {
            off += size;
            kept += 1;
        }
    }
    data[16..20].copy_from_slice(&kept.to_le_bytes());
    data[20..24].copy_from_slice(&((end - header) as u32).to_le_bytes());
    Ok(removed)
}

fn strip_fat(data: &mut [u8]) -> Result<usize> {
    let is64 = be32(data, 0) == Some(FAT_MAGIC_64);
    let n = be32(data, 4).ok_or_else(|| bad("fat header"))? as usize;
    if n > 64 {
        return Err(bad("fat header"));
    }
    let mut total = 0;
    for i in 0..n {
        let (off, size) = if is64 {
            let b = 8 + i * 32;
            (be64(data, b + 8), be64(data, b + 16))
        } else {
            let b = 8 + i * 20;
            (be32(data, b + 8).map(u64::from), be32(data, b + 12).map(u64::from))
        };
        let (off, size) = (off.ok_or_else(|| bad("fat arch"))? as usize, size.ok_or_else(|| bad("fat arch"))? as usize);
        let end = off.checked_add(size).filter(|&e| e <= data.len()).ok_or_else(|| bad("fat slice"))?;
        total += strip_platform_load_commands(&mut data[off..end])?;
    }
    Ok(total)
}

fn strip_archive(data: &mut [u8]) -> Result<usize> {
    let mut pos = AR_MAGIC.len();
    let mut total = 0;
    while pos + 60 <= data.len() {
        let hdr = &data[pos..pos + 60];
        if &hdr[58..60] != b"`\n" {
            return Err(bad("archive member header"));
        }
        let name = String::from_utf8_lossy(&hdr[0..16]).trim_end().to_string();
        let size: usize = String::from_utf8_lossy(&hdr[48..58]).trim().parse().map_err(|_| bad("archive member size"))?;
        let body = pos + 60;
        let end = body.checked_add(size).filter(|&e| e <= data.len()).ok_or_else(|| bad("archive member"))?;
        // BSD long names ("#1/<len>") are stored at the start of the member data.
        let name_len = name.strip_prefix("#1/").and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
        let start = (body + name_len).min(end);
        total += strip_platform_load_commands(&mut data[start..end])?;
        pos = end + (size & 1);
    }
    Ok(total)
}

/// Platform load commands in a thin Mach-O (for tests and diagnostics).
pub fn platform_commands(data: &[u8]) -> Vec<u32> {
    let header = match le32(data, 0) {
        Some(MH_MAGIC_64) => 32,
        Some(MH_MAGIC) => 28,
        _ => return Vec::new(),
    };
    let (Some(ncmds), Some(_)) = (le32(data, 16), le32(data, 20)) else { return Vec::new() };
    let mut out = Vec::new();
    let mut off = header;
    for _ in 0..ncmds {
        let (Some(cmd), Some(size)) = (le32(data, off), le32(data, off + 4)) else { break };
        if PLATFORM_COMMANDS.contains(&cmd) {
            out.push(cmd);
        }
        if size < 8 {
            break;
        }
        off += size as usize;
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A thin arm64 object: LC_SEGMENT_64 (72 bytes), LC_BUILD_VERSION (24),
    /// LC_LINKER_OPTION (24), then 16 bytes of "section data".
    pub(crate) fn object() -> Vec<u8> {
        let mut cmds = Vec::new();
        cmds.extend_from_slice(&0x19u32.to_le_bytes());
        cmds.extend_from_slice(&72u32.to_le_bytes());
        cmds.extend_from_slice(&[0xAA; 64]);
        cmds.extend_from_slice(&0x32u32.to_le_bytes());
        cmds.extend_from_slice(&24u32.to_le_bytes());
        cmds.extend_from_slice(&[2, 0, 0, 0, 0, 0, 17, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // ios, minos 17.0
        cmds.extend_from_slice(&0x2du32.to_le_bytes());
        cmds.extend_from_slice(&24u32.to_le_bytes());
        cmds.extend_from_slice(&1u32.to_le_bytes());
        cmds.extend_from_slice(b"-lobjc\0\0\0\0\0\0");
        let mut o = Vec::new();
        o.extend_from_slice(&MH_MAGIC_64.to_le_bytes());
        o.extend_from_slice(&0x0100_000cu32.to_le_bytes()); // CPU_TYPE_ARM64
        o.extend_from_slice(&0u32.to_le_bytes());
        o.extend_from_slice(&1u32.to_le_bytes()); // MH_OBJECT
        o.extend_from_slice(&3u32.to_le_bytes());
        o.extend_from_slice(&(cmds.len() as u32).to_le_bytes());
        o.extend_from_slice(&0u32.to_le_bytes());
        o.extend_from_slice(&0u32.to_le_bytes());
        o.extend_from_slice(&cmds);
        o.extend_from_slice(b"SECTION-DATA-123");
        o
    }

    fn check_stripped(o: &[u8], len: usize) {
        assert_eq!(o.len(), len);
        assert_eq!(le32(o, 16), Some(2));
        assert_eq!(le32(o, 20), Some(96));
        assert!(platform_commands(o).is_empty());
        assert_eq!(le32(o, 32 + 72), Some(0x2d), "LC_LINKER_OPTION moved up");
        assert_eq!(&o[len - 16..], b"SECTION-DATA-123", "section data untouched");
        assert!(o[32 + 96..32 + 120].iter().all(|&b| b == 0), "freed bytes zeroed");
    }

    #[test]
    fn strips_thin_object() {
        let mut o = object();
        let len = o.len();
        assert_eq!(platform_commands(&o), vec![0x32]);
        assert_eq!(strip_platform_load_commands(&mut o).unwrap(), 1);
        check_stripped(&o, len);
        assert_eq!(strip_platform_load_commands(&mut o).unwrap(), 0, "idempotent");
    }

    #[test]
    fn strips_archive_and_fat_members() {
        let obj = object();
        // BSD archive: symbol table member, then a long-named object member (odd size → pad).
        let mut ar = AR_MAGIC.to_vec();
        let symdef = b"__.SYMDEF SORTED".to_vec();
        ar.extend_from_slice(format!("#1/16           0           0     0     644     {:<10}`\n", 16 + 4).as_bytes());
        ar.extend_from_slice(&symdef);
        ar.extend_from_slice(&[0; 4]);
        let name = b"Compat56.o\0";
        let size = name.len() + obj.len();
        ar.extend_from_slice(format!("#1/{:<13}0           0     0     644     {:<10}`\n", name.len(), size).as_bytes());
        let member_at = ar.len() + name.len();
        ar.extend_from_slice(name);
        ar.extend_from_slice(&obj);
        if size % 2 == 1 {
            ar.push(b'\n');
        }
        let mut a = ar.clone();
        assert_eq!(strip_platform_load_commands(&mut a).unwrap(), 1);
        check_stripped(&a[member_at..member_at + obj.len()], obj.len());
        assert_eq!(&a[..member_at], &ar[..member_at], "headers and symbol table untouched");

        // Fat file with two arm64 slices at 4096-aligned offsets.
        let mut fat = Vec::new();
        fat.extend_from_slice(&FAT_MAGIC.to_be_bytes());
        fat.extend_from_slice(&2u32.to_be_bytes());
        for i in 0..2u32 {
            fat.extend_from_slice(&0x0100_000cu32.to_be_bytes());
            fat.extend_from_slice(&0u32.to_be_bytes());
            fat.extend_from_slice(&(4096 * (i + 1)).to_be_bytes());
            fat.extend_from_slice(&(obj.len() as u32).to_be_bytes());
            fat.extend_from_slice(&12u32.to_be_bytes());
        }
        fat.resize(4096, 0);
        fat.extend_from_slice(&obj);
        fat.resize(8192, 0);
        fat.extend_from_slice(&obj);
        assert_eq!(strip_platform_load_commands(&mut fat).unwrap(), 2);
        check_stripped(&fat[4096..4096 + obj.len()], obj.len());
        check_stripped(&fat[8192..8192 + obj.len()], obj.len());
    }

    #[test]
    fn leaves_other_data_alone_and_rejects_garbage() {
        let mut text = b"--- !tapi-tbd\n".to_vec();
        assert_eq!(strip_platform_load_commands(&mut text).unwrap(), 0);
        assert_eq!(text, b"--- !tapi-tbd\n");
        let mut o = object();
        o[20..24].copy_from_slice(&10_000u32.to_le_bytes()); // sizeofcmds past the end
        assert!(strip_platform_load_commands(&mut o).is_err());
    }
}
