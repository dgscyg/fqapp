// End-to-end validation against the REAL FQNovel API (network required):
// random device registration -> emulated-signer content request.
//
// Not part of the normal suite: run explicitly with
//   cargo test --test e2e_real_api -- --nocapture --ignored
// and optionally FQSIGN_LIB=... to exercise the emulated signer (without
// it the pure-algorithm path is validated instead).
use fqapi_core::device::register::Registrar;
use fqapi_core::sign::Manager;
use fqapi_core::upstream::{UpstreamClient, UpstreamRequest};

#[tokio::test]
#[ignore = "hits the real API; run explicitly"]
async fn register_and_search_with_real_api() {
    // 1. Register a fresh random device (random model surface from models.rs).
    let reg = Registrar::new().expect("registrar");
    let dev = reg
        .register()
        .await
        .expect("device registration failed (see error chain)");
    println!(
        "registered: device_id={} install_id={} model surface ok, secret_key {}",
        dev.device_id,
        dev.install_id,
        if dev.secret_key.is_empty() {
            "EMPTY"
        } else {
            "present"
        }
    );
    assert!(!dev.device_id.is_empty(), "no device_id");
    assert!(!dev.install_id.is_empty(), "no install_id");

    // 2. A signed content request through the full client path (emu signer
    //    when FQSIGN_LIB/the search path finds the artifact, pure algo
    //    otherwise). search_cue is the call that failed in the field log.
    // Pool from a temp file holding just this device.
    let pool_dir = std::env::temp_dir().join(format!("fq-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&pool_dir).unwrap();
    let pool_file = pool_dir.join("device_pool.json");
    let pool_json = serde_json::json!({
        "android": [dev],
        "last_update": "2026-01-01 00:00:00",
    });
    std::fs::write(&pool_file, pool_json.to_string()).unwrap();
    let pool = fqapi_core::device::DevicePool::new(&pool_file)
        .await
        .expect("pool");
    let client = UpstreamClient::new(pool, Manager::new()).expect("client");
    let req = UpstreamRequest {
        url: format!(
            "https://api5-normal-sinfonlineb.fqnovel.com/reading/bookapi/search/cue/v?\
query=%E5%89%91&aid=1967&app_name=novelapp&channel=xiaomi_1967_64&device_platform=android&\
device_id={}&iid={}&os=android&os_version=14&device_brand=OPPO&device_type=PHK110&\
resolution=1216*2672&dpi=480&version_code=65132&version_name=6.5.1.32&\
manifest_version_code=65132&update_version_code=65132&ssmix=a&ac=wifi&host_abi=arm64-v8a",
            dev.device_id, dev.install_id
        ),
        body: None,
        ..UpstreamRequest::default()
    };
    // The emulated signer must be loaded when FQSIGN_LIB points at a real
    // artifact (otherwise this test silently validates the fallback).
    if std::env::var("FQSIGN_LIB")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        assert!(
            fqapi_core::sign::emu::EmuSigner::global().is_some(),
            "FQSIGN_LIB set but the emulated signer failed to load"
        );
        println!("emu signer: LOADED (signing through the emulated SDK)");
    } else {
        println!("emu signer: not requested (FQSIGN_LIB unset) - pure algorithm");
    }
    let (body, _dev) = client
        .do_request_capped(&req, 1 << 20)
        .await
        .expect("search_cue request failed");
    let text = String::from_utf8_lossy(&body);
    println!(
        "search_cue response (first 300 bytes): {}",
        &text[..text.len().min(300)]
    );
    assert!(!body.is_empty(), "empty response body");
}
