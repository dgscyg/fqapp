//! The key store failed at startup, so the core started without the pool key;
//! Dart provisions it later. The pool must then load the *existing* identity
//! from the encrypted file - never register a replacement in between.
//!
//! Runs in its own test-process: it starts with the process-global pool key
//! unprovisioned and provisions it halfway through.

mod common;

use common::{pool_json, MockReply, MockUpstream, TempDir};
use fqapi_core::device::pool_crypto;
use fqapi_core::device::{DevicePool, POOL_LOCKED_ERROR};

const KEY: [u8; 32] = [0x3C; 32];

#[tokio::test]
async fn a_late_pool_key_unlocks_the_existing_identity() {
    // Any registration attempt is recorded; none may happen.
    let upstream = MockUpstream::start(|_| MockReply::status(500)).await;

    let dir = TempDir::new("pool-key-late");
    let path = dir.join("device_pool.json");
    let encrypted =
        pool_crypto::encrypt_pool_file_with_key(pool_json(1).as_bytes(), &KEY).expect("encrypt");
    std::fs::write(&path, &encrypted).expect("seed");

    assert!(
        pool_crypto::pool_key().is_none(),
        "this test needs a process with no pool key provisioned"
    );
    let pool = DevicePool::with_mock_origin(&path, upstream.origin.clone())
        .await
        .expect("pool");
    assert!(pool.is_locked().await);
    assert_eq!(pool.get().await.unwrap_err(), POOL_LOCKED_ERROR);
    assert_eq!(std::fs::read(&path).expect("file"), encrypted);

    // Dart's retry gets the key from the platform store and hands it over.
    pool_crypto::provision_pool_key(KEY.to_vec()).expect("provision");
    let device = pool.get().await.expect("existing identity");
    assert_eq!(device.device_id, "0000000000000000");
    assert_eq!(device.secret_key, format!("{:032x}", 0));
    assert!(!pool.is_locked().await);
    assert_eq!(pool.status().await["locked"], false);

    assert!(
        upstream.requests().is_empty(),
        "unlocking must reuse the stored identity, not register a new one"
    );
    let entries: Vec<String> = std::fs::read_dir(dir.path())
        .expect("read dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec!["device_pool.json".to_string()], "no backup");
    let on_disk = std::fs::read(&path).expect("file");
    assert!(pool_crypto::pool_file_is_encrypted(&on_disk));
    let plain = pool_crypto::decrypt_pool_file(&on_disk).expect("still decryptable");
    assert!(String::from_utf8_lossy(&plain).contains("0000000000000000"));

    upstream.shutdown();
}
