//! bytevc2 → VVC sample-entry rewrite for the loopback media proxy.
//!
//! bytevc2 is a standard VVC/H.266 bitstream; only the MP4 wrapper is
//! vendor-specific: the sample entry is `bvc2` and its configuration box is
//! `bv2C` (an 18-byte vendor header followed by the standard VVC
//! parameter-set NAL arrays). Stock FFmpeg ≥ 7.1 / vvdec decode the stream
//! once the entry says `vvc1` and carries a standard `vvcC`.
//!
//! The rewrite is planned once from `moov` and applied as **equal-length**
//! byte patches while the proxy streams the file: the new `vvcC` is shorter
//! than `bv2C`, so the remaining bytes become a `free` box inside the sample
//! entry. Nothing moves, so `stco`/`co64`, `senc` and `Range` offsets stay
//! valid and CENC (AES-CTR over sample data) is untouched.

/// One byte-range replacement in the served file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Patch {
    pub offset: u64,
    pub bytes: Vec<u8>,
}

/// Positional reader: fills a prefix of the buffer, `Ok(0)` at EOF.
pub(crate) type ReadAt<'a> = dyn FnMut(u64, &mut [u8]) -> Result<usize, ()> + 'a;

/// Upper bound for `moov`, matching the native CENC parser.
const MAX_MOOV: u64 = 16 * 1024 * 1024;
const MAX_TOP_LEVEL_BOXES: usize = 4096;
const CONTAINERS: [&[u8; 4]; 5] = [b"moov", b"trak", b"mdia", b"minf", b"stbl"];
/// `VisualSampleEntry` fields before its child boxes (ISO/IEC 14496-12).
const VISUAL_ENTRY_FIELDS: usize = 78;

fn read_exact(
    read: &mut ReadAt<'_>,
    mut offset: u64,
    mut buf: &mut [u8],
) -> Result<(), ()> {
    while !buf.is_empty() {
        let n = read(offset, buf)?;
        if n == 0 {
            return Err(());
        }
        offset += n as u64;
        buf = &mut buf[n..];
    }
    Ok(())
}

/// Box header at `pos` inside `data[..end]`: (type, header length, total size).
fn child_box(data: &[u8], pos: usize, end: usize) -> Option<([u8; 4], usize, usize)> {
    if pos.checked_add(8)? > end {
        return None;
    }
    let size32 = u32::from_be_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
    let kind: [u8; 4] = data[pos + 4..pos + 8].try_into().ok()?;
    let (header, size) = match size32 {
        0 => (8, end - pos),
        1 => {
            if pos + 16 > end {
                return None;
            }
            let size64 = u64::from_be_bytes(data[pos + 8..pos + 16].try_into().ok()?);
            (16, usize::try_from(size64).ok()?)
        }
        n => (8, n),
    };
    if size < header || pos.checked_add(size)? > end {
        return None;
    }
    Some((kind, header, size))
}

/// Builds `vvcC` (FullBox v0) content from a `bv2C` payload: the vendor header
/// is skipped by locating the NAL arrays that run exactly to the end.
/// Returns the new payload and where the arrays started.
pub(crate) fn vvcc_from_bv2c(cfg: &[u8]) -> Option<(Vec<u8>, usize)> {
    let valid_type = |t: u8| (12..=17).contains(&t);
    'start: for start in 0..cfg.len().saturating_sub(8) {
        let arrays = cfg[start];
        if !(1..=8).contains(&arrays) || cfg[start + 1] & 0x80 == 0 {
            continue;
        }
        if !valid_type(cfg[start + 1] & 0x1f) {
            continue;
        }
        let mut j = start + 1;
        for _ in 0..arrays {
            if j + 3 > cfg.len() {
                continue 'start;
            }
            // Later arrays may carry SEI/OPI/DCI; only NAL headers must agree.
            let array_type = cfg[j] & 0x1f;
            let count = u16::from_be_bytes([cfg[j + 1], cfg[j + 2]]);
            j += 3;
            for _ in 0..count {
                if j + 2 > cfg.len() {
                    continue 'start;
                }
                let len = u16::from_be_bytes([cfg[j], cfg[j + 1]]) as usize;
                if len < 2 || j + 2 + len > cfg.len() || cfg[j + 3] >> 3 != array_type {
                    continue 'start;
                }
                j += 2 + len;
            }
        }
        if j == cfg.len() {
            // version/flags 0; reserved '11111'b, LengthSizeMinusOne = 3,
            // ptl_present_flag = 0; then the arrays verbatim.
            let mut out = vec![0, 0, 0, 0, 0xFE];
            out.extend_from_slice(&cfg[start..]);
            return Some((out, start));
        }
    }
    None
}

/// Equal-length replacement of a whole `bv2C` box (`size` bytes, 32-bit header).
fn rewrite_bv2c_box(payload: &[u8], size: usize) -> Option<Vec<u8>> {
    let (vvcc, _) = vvcc_from_bv2c(payload)?;
    let vvcc_size = 8 + vvcc.len();
    let pad = size.checked_sub(vvcc_size)?;
    if pad != 0 && pad < 8 {
        return None;
    }
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(&(vvcc_size as u32).to_be_bytes());
    out.extend_from_slice(b"vvcC");
    out.extend_from_slice(&vvcc);
    if pad != 0 {
        out.extend_from_slice(&(pad as u32).to_be_bytes());
        out.extend_from_slice(b"free");
        out.resize(size, 0);
    }
    Some(out)
}

/// Patches for one sample entry (`bvc2`, or `encv` whose `frma` is `bvc2`).
fn plan_entry(
    moov: &[u8],
    base: u64,
    pos: usize,
    kind: [u8; 4],
    header: usize,
    size: usize,
    out: &mut Vec<Patch>,
) -> Result<(), ()> {
    let end = pos + size;
    let mut child = pos + header + VISUAL_ENTRY_FIELDS;
    let mut config = None;
    let mut frma = None;
    while let Some((ck, ch, cs)) = child_box(moov, child, end) {
        match &ck {
            b"bv2C" if ch == 8 => config = Some((child, cs)),
            b"sinf" => {
                let mut s = child + ch;
                while let Some((sk, sh, ss)) = child_box(moov, s, child + cs) {
                    if &sk == b"frma" && ss == sh + 4 && &moov[s + sh..s + sh + 4] == b"bvc2" {
                        frma = Some(s + sh);
                    }
                    s += ss;
                }
            }
            _ => (),
        }
        child += cs;
    }
    let is_bvc2 = &kind == b"bvc2";
    if !is_bvc2 && frma.is_none() {
        return Ok(());
    }
    let (config_pos, config_size) = config.ok_or(())?;
    let replacement =
        rewrite_bv2c_box(&moov[config_pos + 8..config_pos + config_size], config_size).ok_or(())?;
    if is_bvc2 {
        out.push(Patch {
            offset: base + pos as u64 + 4,
            bytes: b"vvc1".to_vec(),
        });
    }
    if let Some(frma) = frma {
        out.push(Patch {
            offset: base + frma as u64,
            bytes: b"vvc1".to_vec(),
        });
    }
    out.push(Patch {
        offset: base + config_pos as u64,
        bytes: replacement,
    });
    Ok(())
}

fn plan_moov(
    moov: &[u8],
    base: u64,
    start: usize,
    end: usize,
    out: &mut Vec<Patch>,
) -> Result<(), ()> {
    let mut pos = start;
    while let Some((kind, header, size)) = child_box(moov, pos, end) {
        if CONTAINERS.contains(&&kind) {
            plan_moov(moov, base, pos + header, pos + size, out)?;
        } else if &kind == b"stsd" && size >= header + 8 {
            let mut entry = pos + header + 8;
            while let Some((ek, eh, es)) = child_box(moov, entry, pos + size) {
                if (&ek == b"bvc2" || &ek == b"encv") && es >= eh + VISUAL_ENTRY_FIELDS {
                    plan_entry(moov, base, entry, ek, eh, es, out)?;
                }
                entry += es;
            }
        }
        pos += size;
    }
    Ok(())
}

/// Reads the top-level boxes and `moov` through `read` and returns the
/// patches that turn every bytevc2 track into a standard `vvc1` track.
/// An empty list means the file has no bytevc2 track (served unchanged).
/// `Err` means a bytevc2 track exists but cannot be rewritten safely.
pub(crate) fn plan_vvc_rewrite(
    read: &mut ReadAt<'_>,
    size: u64,
) -> Result<Vec<Patch>, ()> {
    let mut offset = 0u64;
    for _ in 0..MAX_TOP_LEVEL_BOXES {
        if offset.checked_add(8).ok_or(())? > size {
            break;
        }
        let mut head = [0u8; 16];
        let head_len = if size - offset >= 16 { 16 } else { 8 };
        read_exact(read, offset, &mut head[..head_len])?;
        let size32 = u32::from_be_bytes(head[0..4].try_into().map_err(|_| ())?) as u64;
        let (header, box_size) = match size32 {
            0 => (8, size - offset),
            1 if head_len == 16 => (
                16,
                u64::from_be_bytes(head[8..16].try_into().map_err(|_| ())?),
            ),
            1 => return Err(()),
            n => (8, n),
        };
        if box_size < header || offset.checked_add(box_size).ok_or(())? > size {
            return Err(());
        }
        if &head[4..8] == b"moov" {
            if box_size > MAX_MOOV {
                return Err(());
            }
            let mut moov = vec![0u8; box_size as usize];
            read_exact(read, offset, &mut moov)?;
            let mut out = Vec::new();
            plan_moov(&moov, offset, header as usize, moov.len(), &mut out)?;
            out.sort_by_key(|patch| patch.offset);
            return Ok(out);
        }
        offset += box_size;
    }
    Ok(Vec::new())
}

/// Overlays every patch that intersects `[offset, offset + buf.len())`.
pub(crate) fn apply_patches(patches: &[Patch], offset: u64, buf: &mut [u8]) {
    let end = offset + buf.len() as u64;
    for patch in patches {
        let patch_end = patch.offset + patch.bytes.len() as u64;
        if patch_end <= offset || patch.offset >= end {
            continue;
        }
        let from = patch.offset.max(offset);
        let to = patch_end.min(end);
        let dst = (from - offset) as usize..(to - offset) as usize;
        let src = (from - patch.offset) as usize..(to - patch.offset) as usize;
        buf[dst].copy_from_slice(&patch.bytes[src]);
    }
}

#[cfg(test)]
#[path = "media_vvc_tests.rs"]
pub(crate) mod tests;
