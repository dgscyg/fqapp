//! Upgrade regression: syncing upstream brought in a config-driven
//! `UpstreamIdentity` (and, upstream, a pool that retires devices whose
//! registered version differs from it). Our build keeps exactly one device,
//! the international oversea 68132 identity, and only a manual regenerate in
//! 设置-核心 may replace it. An upgrade - with or without an `upstream`
//! section in config.json - must load that same device and never register a
//! new one.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_dir() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "fqapp-upgrade-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

/// A pool file as written by 1.0.212: one registered 68132 device, without
/// any upstream-only fields. `extra` lets a test add fields an upstream
/// build may have written (they must be ignored, not trigger eviction).
fn legacy_pool(extra: serde_json::Value) -> Vec<u8> {
    let mut device = serde_json::json!({
        "device_id": "7412589630012345",
        "install_id": "7412589630054321",
        "secret_key": "00112233445566778899aabbccddeeff",
        "platform": "android",
        "status": "active",
        "created_time": "2026-11-01 10:00:00",
        "last_used": "2026-11-17 09:00:00",
        "use_count": 42,
        "cdid": "cdid-existing",
        "openudid": "openudid-existing",
        "device_type": "SM-S9180",
        "device_brand": "samsung",
        "os_version": "14",
        "os_api": "34",
        "rom_version": "UP1A.231005.007",
        "host_abi": "arm64-v8a",
    });
    if let (Some(map), Some(more)) = (device.as_object_mut(), extra.as_object()) {
        for (k, v) in more {
            map.insert(k.clone(), v.clone());
        }
    }
    serde_json::to_vec(&serde_json::json!({
        "android": [device],
        "last_update": "2026-11-17 09:00:00",
    }))
    .expect("pool json")
}

fn upgrade_keeps_device(config_json: &str, pool_extra: serde_json::Value) {
    let dir = temp_dir();
    let config_path = dir.join("config.json");
    std::fs::write(&config_path, config_json).expect("write config");
    // The config must load (an upstream section is accepted) ...
    let config = crate::config::Config::load(config_path.to_str().unwrap())
        .expect("config with an upstream section must load");
    let _ = config.upstream.resolved();

    let pool_path = dir.join("device_pool.json");
    std::fs::write(&pool_path, legacy_pool(pool_extra.clone())).expect("write pool");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        // ... and the pool is built exactly like api::init does. Registration
        // traffic goes to a dead origin: any attempt to register a
        // replacement device fails this test instead of hitting upstream.
        let pool = DevicePool::with_mock_origin(&pool_path, "http://127.0.0.1:9")
            .await
            .expect("pool loads");
        assert_eq!(pool.count().await, 1);
        let device = pool.get().await.expect("existing device is served");
        assert_eq!(device.device_id, "7412589630012345");
        assert_eq!(device.install_id, "7412589630054321");
        assert!(device.is_registered());
        assert_eq!(
            device.key_register_ts,
            pool_extra
                .get("key_register_ts")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            "legacy pools must load without a content-key timestamp"
        );
        assert_eq!(pool.current_id().await.as_deref(), Some("7412589630012345"));
        pool.retire().await;
    });
    let saved = std::fs::read(&pool_path).expect("pool file still present");
    let text = String::from_utf8_lossy(&saved);
    if !pool_crypto::pool_file_is_encrypted(&saved) {
        assert!(text.contains("7412589630012345"), "device evicted: {text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn upgrade_without_upstream_section_keeps_existing_68132_device() {
    upgrade_keeps_device(r#"{"port":0}"#, serde_json::json!({}));
}

#[test]
fn upstream_default_identity_does_not_evict_68132_device() {
    // Upstream's default identity is 6.5.1.32 / 65132.
    upgrade_keeps_device(
        r#"{"port":0,"upstream":{"version_name":"6.5.1.32","version_code":"65132"}}"#,
        serde_json::json!({}),
    );
}

#[test]
fn newer_upstream_identity_does_not_evict_68132_device() {
    upgrade_keeps_device(
        r#"{"port":0,"upstream":{"version_name":"7.1.3.32","version_code":"71332"}}"#,
        // Field an upstream build writes next to each device.
        serde_json::json!({"registered_version_code": "65132"}),
    );
}

#[test]
fn ensure_content_key_returns_cached_registerkey_when_timestamp_is_present() {
    // gonidbg EnsureReady: a cache hit (key + key_register_ts) must not
    // hit the network. The mock origin is closed, so a fetch would fail.
    let dir = temp_dir();
    let pool_path = dir.join("device_pool.json");
    std::fs::write(
        &pool_path,
        legacy_pool(serde_json::json!({
            "key_register_ts": 1_700_000_000_000i64,
            "keyver": 1
        })),
    )
    .expect("write pool");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let pool = DevicePool::with_mock_origin(&pool_path, "http://127.0.0.1:9")
            .await
            .expect("pool loads");
        let device = pool
            .ensure_content_key("7412589630012345")
            .await
            .expect("cached registerkey must be reused");
        assert_eq!(device.key_register_ts, 1_700_000_000_000);
        assert_eq!(device.keyver, 1);
        pool.retire().await;
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ensure_content_key_fetches_when_key_register_ts_is_missing() {
    // Pools written before we stored key_register_ts look like an empty
    // gonidbg cache and must call registerkey. A closed origin proves we
    // attempted the fetch instead of silently reusing the stale secret_key.
    let dir = temp_dir();
    let pool_path = dir.join("device_pool.json");
    std::fs::write(&pool_path, legacy_pool(serde_json::json!({}))).expect("write pool");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let pool = DevicePool::with_mock_origin(&pool_path, "http://127.0.0.1:9")
            .await
            .expect("pool loads");
        let err = pool
            .ensure_content_key("7412589630012345")
            .await
            .expect_err("missing timestamp must refresh registerkey");
        assert!(
            err.contains("refresh secret_key"),
            "unexpected error: {err}"
        );
        pool.retire().await;
    });
    let _ = std::fs::remove_dir_all(&dir);
}
