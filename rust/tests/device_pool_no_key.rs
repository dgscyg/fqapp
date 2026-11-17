//! An encrypted pool file whose key is missing locks the pool: no automatic
//! registration, no save over the file. Only a manual regenerate may replace
//! the locked identity, and even then the encrypted bytes are kept as a backup.
//!
//! Runs in its own test-process: it needs the process-global pool key to be
//! unprovisioned, which no other pool test can arrange once a key is set.

mod common;

use common::{MockReply, MockUpstream, TempDir};
use fqapi_core::device::pool_crypto;
use fqapi_core::device::{DevicePool, POOL_LOCKED_ERROR};

/// From rust/testdata/device_vectors.json: a registerkey response payload that
/// decrypts to `SECRETKEY_16byte` under the reference implementation.
const REGISTERKEY_INPUT_B64: &str =
    "ABEiM0RVZneImaq7zN3u//NnA1PPLklvy0VWJ2Qnd45A/zarUCjsToTdr+Zm34ab";

/// Encrypted-looking bytes: the magic prefix plus an opaque payload. Without a
/// key the pool cannot read them, which is all this test needs.
fn encrypted_looking_pool_bytes() -> Vec<u8> {
    let mut bytes = b"FQPOOL1".to_vec();
    bytes.extend_from_slice(&[0x11; 64]);
    bytes
}

fn backups(dir: &TempDir) -> Vec<String> {
    std::fs::read_dir(dir.path())
        .expect("read dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("device_pool.json.corrupt-"))
        .collect()
}

#[tokio::test]
async fn a_locked_pool_never_registers_or_saves_until_a_manual_regenerate() {
    let upstream = MockUpstream::start(|recorded| match recorded.path.as_str() {
        "/service/2/device_register/" => MockReply::json(serde_json::json!({
            "device_id_str": "7276663560427471412",
            "install_id_str": "7276663560427471000",
        })),
        "/reading/crypt/registerkey" => MockReply::json(serde_json::json!({
            "data": { "key": REGISTERKEY_INPUT_B64 },
        })),
        _ => MockReply::status(200),
    })
    .await;

    let dir = TempDir::new("pool-no-key");
    let path = dir.join("device_pool.json");
    let encrypted = encrypted_looking_pool_bytes();
    std::fs::write(&path, &encrypted).expect("seed");

    // Dart provisions the key at startup; here the key store "failed", so the
    // core starts without one and the pool is locked.
    let pool = DevicePool::with_mock_origin(&path, upstream.origin.clone())
        .await
        .expect("pool");
    assert!(
        pool_crypto::pool_key().is_none(),
        "this test needs a process with no pool key provisioned"
    );
    assert!(pool.is_locked().await);
    assert_eq!(pool.status().await["locked"], true);

    // Every lookup fails with the locked error instead of registering a
    // second identity (the single-device rule).
    assert_eq!(pool.get().await.unwrap_err(), POOL_LOCKED_ERROR);
    assert_eq!(pool.get().await.unwrap_err(), POOL_LOCKED_ERROR);
    assert!(pool.get_by_id("0000000000000001").await.is_err());
    assert_eq!(pool.refill(1).await.unwrap_err(), POOL_LOCKED_ERROR);
    assert!(
        upstream.requests().is_empty(),
        "a locked pool must not contact the registration endpoints"
    );
    assert_eq!(
        std::fs::read(&path).expect("pool file"),
        encrypted,
        "the encrypted file must stay untouched while locked"
    );
    assert!(backups(&dir).is_empty(), "nothing moved aside while locked");

    // The user explicitly regenerates in Settings > 核心: the new identity is
    // saved (plaintext, no key) and the encrypted bytes survive verbatim.
    let fresh = pool.regenerate().await.expect("manual regenerate");
    assert_eq!(fresh.device_id, "7276663560427471412");
    assert!(!pool.is_locked().await);
    let moved = backups(&dir);
    assert_eq!(moved.len(), 1, "exactly one backup: {moved:?}");
    assert_eq!(
        std::fs::read(dir.join(&moved[0])).expect("backup"),
        encrypted,
        "the encrypted bytes must be preserved verbatim"
    );
    let on_disk = std::fs::read(&path).expect("pool file");
    assert!(!pool_crypto::pool_file_is_encrypted(&on_disk));
    assert_eq!(
        pool.get().await.expect("device").device_id,
        "7276663560427471412"
    );

    upstream.shutdown();
}
