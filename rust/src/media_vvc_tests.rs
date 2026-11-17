use super::*;

pub(crate) fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// 18-byte vendor header + VPS/SPS/PPS arrays, the `bv2C` shape seen on the CDN.
fn bv2c_payload() -> Vec<u8> {
    let mut cfg = vec![
        0x01, 0x07, 0x42, 0x00, 0x01, 0x02, 0x69, 0, 0, 0, 0, 0, 0x04, 0x38, 0x07, 0x80, 0, 0,
    ];
    cfg.push(3); // num_of_arrays
    for (nal_type, body) in [(15u8, &[1u8, 2, 3][..]), (16, &[4, 5, 6, 7]), (23, &[8])] {
        cfg.push(0x80 | nal_type);
        cfg.extend_from_slice(&1u16.to_be_bytes());
        let nal = [&[0x00, nal_type << 3 | 1][..], body].concat();
        cfg.extend_from_slice(&(nal.len() as u16).to_be_bytes());
        cfg.extend_from_slice(&nal);
    }
    cfg
}

fn visual_entry(kind: &[u8; 4], children: &[Vec<u8>]) -> Vec<u8> {
    let mut body = vec![0u8; 78];
    body[7] = 1; // data_reference_index
    for child in children {
        body.extend_from_slice(child);
    }
    bx(kind, &body)
}

fn movie(entry: Vec<u8>) -> (Vec<u8>, std::ops::Range<usize>) {
    let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
    stsd.extend_from_slice(&entry);
    let stbl = bx(
        b"stbl",
        &[bx(b"stsd", &stsd), bx(b"stco", &[0; 8])].concat(),
    );
    let moov = bx(b"moov", &bx(b"trak", &bx(b"mdia", &bx(b"minf", &stbl))));
    let ftyp = bx(b"ftyp", b"isom\0\0\0\0isomiso2");
    let mdat = bx(b"mdat", &(0..=255u8).cycle().take(5000).collect::<Vec<_>>());
    let start = ftyp.len() + moov.len() + 8;
    let file = [ftyp, moov, mdat].concat();
    let end = file.len();
    (file, start..end)
}

fn reader(file: &[u8]) -> impl FnMut(u64, &mut [u8]) -> Result<usize, ()> + '_ {
    // Short reads exercise the read_exact loop.
    move |offset, buf| {
        let offset = offset as usize;
        if offset >= file.len() {
            return Ok(0);
        }
        let n = buf.len().min(file.len() - offset).min(7);
        buf[..n].copy_from_slice(&file[offset..offset + n]);
        Ok(n)
    }
}

fn rewritten(file: &[u8]) -> Vec<u8> {
    let patches = plan_vvc_rewrite(&mut reader(file), file.len() as u64).unwrap();
    let mut out = file.to_vec();
    apply_patches(&patches, 0, &mut out);
    out
}

fn find(data: &[u8], needle: &[u8]) -> Option<usize> {
    data.windows(needle.len()).position(|w| w == needle)
}

#[test]
fn vendor_header_is_skipped() {
    let cfg = bv2c_payload();
    let (vvcc, start) = vvcc_from_bv2c(&cfg).unwrap();
    assert_eq!(start, 18);
    assert_eq!(&vvcc[..5], &[0, 0, 0, 0, 0xFE]);
    assert_eq!(&vvcc[5..], &cfg[18..]);
}

#[test]
fn clear_bvc2_entry_becomes_vvc1_without_moving_bytes() {
    let (file, mdat) = movie(visual_entry(b"bvc2", &[bx(b"bv2C", &bv2c_payload())]));
    let out = rewritten(&file);
    assert_eq!(out.len(), file.len());
    assert_eq!(out[mdat.clone()], file[mdat]);
    assert!(find(&out, b"bvc2").is_none());
    assert!(find(&out, b"bv2C").is_none());
    let entry = find(&out, b"vvc1").unwrap() - 4;
    let config = find(&out, b"vvcC").unwrap() - 4;
    assert_eq!(config, entry + 8 + 78);
    let vvcc_size = u32::from_be_bytes(out[config..config + 4].try_into().unwrap()) as usize;
    let payload = &out[config + 8..config + vvcc_size];
    assert_eq!(payload, &vvcc_from_bv2c(&bv2c_payload()).unwrap().0[..]);
    // The leftover bytes are a well-formed `free` box ending the entry.
    let pad = config + vvcc_size;
    assert_eq!(&out[pad + 4..pad + 8], b"free");
    let pad_size = u32::from_be_bytes(out[pad..pad + 4].try_into().unwrap()) as usize;
    assert_eq!(pad_size, 13);
    let entry_size = u32::from_be_bytes(out[entry..entry + 4].try_into().unwrap()) as usize;
    assert_eq!(pad + pad_size, entry + entry_size);
}

#[test]
fn encrypted_entry_rewrites_frma_and_config() {
    let sinf = bx(
        b"sinf",
        &[bx(b"frma", b"bvc2"), bx(b"schm", &[0; 12])].concat(),
    );
    let (file, _) = movie(visual_entry(b"encv", &[bx(b"bv2C", &bv2c_payload()), sinf]));
    let out = rewritten(&file);
    assert_eq!(out.len(), file.len());
    assert!(
        find(&out, b"encv").is_some(),
        "the CENC layer keeps its entry"
    );
    assert!(find(&out, b"frmavvc1").is_some());
    assert!(find(&out, b"vvcC").is_some());
}

#[test]
fn other_codecs_pass_through() {
    let (file, _) = movie(visual_entry(b"hvc1", &[bx(b"hvcC", &[1; 30])]));
    assert!(plan_vvc_rewrite(&mut reader(&file), file.len() as u64)
        .unwrap()
        .is_empty());
    let sinf = bx(b"sinf", &bx(b"frma", b"hvc1"));
    let (file, _) = movie(visual_entry(b"encv", &[bx(b"hvcC", &[1; 30]), sinf]));
    assert!(plan_vvc_rewrite(&mut reader(&file), file.len() as u64)
        .unwrap()
        .is_empty());
    // Not an MP4 at all: nothing to patch, the bytes are served as-is.
    let junk = vec![0u8; 4];
    assert!(plan_vvc_rewrite(&mut reader(&junk), 4).unwrap().is_empty());
}

#[test]
fn unparseable_vendor_config_is_an_error() {
    let (file, _) = movie(visual_entry(b"bvc2", &[bx(b"bv2C", &[9; 40])]));
    assert!(plan_vvc_rewrite(&mut reader(&file), file.len() as u64).is_err());
    let (file, _) = movie(visual_entry(b"bvc2", &[]));
    assert!(plan_vvc_rewrite(&mut reader(&file), file.len() as u64).is_err());
}

#[test]
fn ranged_reads_match_the_whole_file_rewrite() {
    let (file, _) = movie(visual_entry(b"bvc2", &[bx(b"bv2C", &bv2c_payload())]));
    let patches = plan_vvc_rewrite(&mut reader(&file), file.len() as u64).unwrap();
    let whole = rewritten(&file);
    for chunk in [1usize, 3, 17, 64, 4096] {
        for start in (0..file.len()).step_by(5) {
            let end = (start + chunk).min(file.len());
            let mut buf = file[start..end].to_vec();
            apply_patches(&patches, start as u64, &mut buf);
            assert_eq!(buf, whole[start..end], "chunk {chunk} at {start}");
        }
    }
}

/// Real CDN sample (not committed). Set FQAPP_TEST_BVC2_SAMPLE to an
/// encrypted bytevc2 MP4 to check the planner on production metadata.
#[test]
fn real_bvc2_sample_when_available() {
    let Ok(path) = std::env::var("FQAPP_TEST_BVC2_SAMPLE") else {
        return;
    };
    let file = std::fs::read(path).unwrap();
    let out = rewritten(&file);
    assert_eq!(out.len(), file.len());
    assert!(find(&out, b"bv2C").is_none());
    assert!(find(&out, b"vvcC").is_some());
}

/// Synthetic clear bytevc2 movie and its expected rewrite (proxy tests).
pub(crate) fn synthetic_bvc2_movie() -> (Vec<u8>, Vec<u8>) {
    let (file, _) = movie(visual_entry(b"bvc2", &[bx(b"bv2C", &bv2c_payload())]));
    let expected = rewritten(&file);
    (file, expected)
}

pub(crate) fn synthetic_hvc1_movie() -> (Vec<u8>, ()) {
    let (file, _) = movie(visual_entry(b"hvc1", &[bx(b"hvcC", &[1; 30])]));
    (file, ())
}
