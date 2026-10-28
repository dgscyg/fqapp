// Emulated libmetasec signer (libfqsign artifact) contract test.
//
// The artifact is a prebuilt binary that is NOT part of this repo, so the
// test skips when it cannot be found. Point FQSIGN_LIB at libfqsign.so to
// run it; CI can drop the artifact next to the test binary instead.
//
// What is verified when the artifact is present:
//   * the library boots (embedded resources, real clock)
//   * one signature produces the full 8-header set plus the caller-side
//     x-ss-req-ticket / x-ss-stub headers
//   * a second signature also succeeds (the emulated .so state evolves
//     per call — the wrapper must not reset or reuse a stale blob)

use fqapi_core::sign::emu;

const URL: &str = "https://api5-normal-sinfonlineb.fqnovel.com/reading/reader/batch_full/v?\
manifest_version_code=68132&_rticket=1754299683771&iid=4223674528611611&channel=googleplay\
&device_type=ANA-AN00&aid=1967&device_id=4223674528607515&os=android&os_version=13\
&version_code=68132&app_name=novelapp&device_platform=android";

fn request_headers() -> Vec<(String, String)> {
    vec![
        ("user-agent".into(),
         "com.dragon.read.oversea.gp/68132 (Linux; U; Android 12; zh_CN; ANA-AN00; Build/V417IR;tt-ok/3.12.13.4-tiktok)".into()),
        ("cookie".into(),
         "store-region=cn-zj; store-region-src=did; install_id=4223674528611611;".into()),
        ("accept-encoding".into(), "gzip".into()),
    ]
}

fn find_lib() -> Option<String> {
    if let Ok(p) = std::env::var("FQSIGN_LIB") {
        if std::path::Path::new(&p).is_file() {
            return Some(p);
        }
    }
    let name = if cfg!(windows) {
        "fqsign.dll"
    } else {
        "libfqsign.so"
    };
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    for d in [dir.to_path_buf(), dir.join("../../..")] {
        let p = d.join(name);
        if p.is_file() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    std::env::current_dir().ok().and_then(|cwd| {
        let p = cwd.join(name);
        p.is_file().then(|| p.to_string_lossy().into_owned())
    })
}

#[test]
fn emu_signer_full_header_set() {
    let Some(lib) = find_lib() else {
        eprintln!("skipping: libfqsign artifact not found (set FQSIGN_LIB to run)");
        return;
    };
    std::env::set_var("FQSIGN_LIB", &lib);

    let sig = emu::sign_request(URL, &request_headers(), None, 1754299683771, None)
        .expect("emu sign failed (see warnings in log)");
    let names: Vec<String> = sig.iter().map(|(k, _)| k.to_lowercase()).collect();
    for expected in [
        "x-argus",
        "x-gorgon",
        "x-helios",
        "x-khronos",
        "x-ladon",
        "x-medusa",
        "x-neptune",
        "x-soter",
        "x-ss-req-ticket",
    ] {
        assert!(
            names.iter().any(|k| k == expected),
            "missing header {expected} (got: {names:?})"
        );
    }
    // Values must be non-empty.
    for (k, v) in &sig {
        assert!(!v.is_empty(), "empty value for {k}");
    }

    // Second call: state evolves, but signing must still succeed.
    let sig2 = emu::sign_request(URL, &request_headers(), None, 1754299684000, None)
        .expect("second emu sign failed");
    assert_eq!(sig2.len(), sig.len(), "header count changed between calls");
}

#[test]
fn emu_signer_device_identity_reboots() {
    let Some(lib) = find_lib() else {
        eprintln!("skipping: libfqsign artifact not found (set FQSIGN_LIB to run)");
        return;
    };
    std::env::set_var("FQSIGN_LIB", &lib);

    // A device serial must be accepted (engine re-boots with the identity)
    // and still produce the full header set.
    let sig = emu::sign_request(
        URL,
        &request_headers(),
        None,
        1754299683771,
        Some("0157feed0000beef"),
    )
    .expect("emu sign with device serial failed");
    let names: Vec<String> = sig.iter().map(|(k, _)| k.to_lowercase()).collect();
    assert!(names.iter().any(|k| k == "x-argus"), "missing x-argus");

    // Rotating to another serial must also succeed (second re-boot).
    let sig2 = emu::sign_request(
        URL,
        &request_headers(),
        None,
        1754299684000,
        Some("0157cafe00112233"),
    )
    .expect("emu sign after device rotation failed");
    assert_eq!(sig2.len(), sig.len(), "header count changed after rotation");
}
