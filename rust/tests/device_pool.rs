//! Device pool contract tests: Go-compatible persistence, single-device identity,
//! concurrent access, and registration against a mock upstream.

mod common;

use common::{pool_json, MockReply, MockUpstream, TempDir};
use fqapi_core::device::{DevicePool, MIN_POOL_SIZE};

/// From rust/testdata/device_vectors.json: a registerkey response payload that
/// decrypts to `SECRETKEY_16byte` under the reference implementation.
const REGISTERKEY_INPUT_B64: &str =
    "ABEiM0RVZneImaq7zN3u//NnA1PPLklvy0VWJ2Qnd45A/zarUCjsToTdr+Zm34ab";
const REGISTERKEY_HEX: &str = "5345435245544B45595F313662797465";

#[tokio::test]
async fn loads_the_go_compatible_pool_json() {
    let dir = TempDir::new("pool-load");
    let path = dir.write("device_pool.json", &pool_json(3));

    let pool = DevicePool::new(&path).await.expect("pool loads");
    assert_eq!(
        pool.count().await,
        1,
        "a historical multi-device pool migrates to one"
    );

    let device = pool
        .get_by_id("0000000000000002")
        .await
        .expect("MRU device 2");
    assert_eq!(device.secret_key, format!("{:032x}", 2));
    assert_eq!(device.cdid, "cdid-2");
    assert_eq!(device.platform, "android");
    // Unknown ids are reported, not silently replaced.
    assert!(pool.get_by_id("missing").await.is_err());
}

#[tokio::test]
async fn missing_and_corrupt_pool_files_start_empty() {
    let dir = TempDir::new("pool-empty");
    let missing = dir.join("missing.json");
    let pool = DevicePool::new(&missing).await.expect("pool");
    assert_eq!(pool.count().await, 0);

    let corrupt = dir.write("corrupt.json", "{not json");
    let pool = DevicePool::new(&corrupt).await.expect("pool");
    assert_eq!(pool.count().await, 0);
}

#[tokio::test]
async fn loading_a_multi_device_pool_keeps_only_the_most_recently_used() {
    let dir = TempDir::new("pool-migrate");
    // pool_json orders last_used ascending, so device 4 is the MRU.
    let path = dir.write("device_pool.json", &pool_json(5));
    let pool = DevicePool::new(&path).await.expect("pool");

    assert_eq!(pool.count().await, 1);
    let first = pool.get().await.expect("device");
    assert_eq!(first.device_id, format!("{:016}", 4), "MRU kept");
    for step in 1..=3 {
        let device = pool.get().await.expect("device");
        assert_eq!(
            device.device_id, first.device_id,
            "single-device identity changed at step {step}"
        );
    }
    assert_eq!(pool.count().await, 1, "get() must not create extra devices");

    let persisted: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("pool file")).expect("json");
    assert_eq!(
        persisted["android"].as_array().map(|a| a.len()),
        Some(1),
        "migration must be written back"
    );
    assert_eq!(
        persisted["android"][0]["device_id"].as_str(),
        Some("0000000000000004")
    );
}

#[tokio::test]
async fn concurrent_gets_do_not_corrupt_the_pool_file() {
    let dir = TempDir::new("pool-concurrent");
    let path = dir.write("device_pool.json", &pool_json(5));
    let pool = DevicePool::new(&path).await.expect("pool");

    let mut handles = Vec::new();
    for _ in 0..8 {
        let pool = pool.clone();
        handles.push(tokio::spawn(async move {
            pool.get().await.map(|d| d.device_id)
        }));
    }
    for handle in handles {
        assert!(handle.await.expect("join").is_ok(), "concurrent get failed");
    }

    // Usage counters are written back at most every 30s; a structural save
    // commits whatever the concurrent picks left in memory.
    pool.refill(0).await.expect("flush usage counters");

    // The file must still be valid Go-compatible JSON with every device intact.
    let text = std::fs::read_to_string(&path).expect("pool file");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid JSON after races");
    let devices = parsed["android"].as_array().expect("android array");
    assert_eq!(devices.len(), 1, "the pool must stay a single identity");
    let mut hot = 0;
    for device in devices {
        if device["use_count"].as_i64().unwrap_or(0) >= 1 {
            hot += 1;
        }
        assert!(!device["device_id"].as_str().unwrap_or("").is_empty());
    }
    assert!(hot >= 1, "the single device must record use");
    assert!(parsed["last_update"].is_string());
}

#[tokio::test]
async fn refill_registers_a_device_and_decrypts_its_secret_key() {
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

    let dir = TempDir::new("pool-refill");
    let pool = DevicePool::with_mock_origin(dir.join("device_pool.json"), upstream.origin.clone())
        .await
        .expect("pool");

    // The registration flow intentionally waits 2s after app_alert_check;
    // this is the one test that pays that cost.
    pool.refill(1).await.expect("refill");
    assert_eq!(pool.count().await, 1);

    let device = pool
        .get_by_id("7276663560427471412")
        .await
        .expect("new device");
    assert_eq!(
        device.secret_key, REGISTERKEY_HEX,
        "the registerkey response must decrypt to the reference secret key"
    );
    assert_eq!(device.install_id, "7276663560427471000");
    assert_eq!(device.status, "active");

    // Registration really hit the mock endpoints in order.
    let paths: Vec<String> = upstream.requests().iter().map(|r| r.path.clone()).collect();
    assert!(paths.contains(&"/service/2/device_register/".to_string()));
    assert!(paths.contains(&"/reading/crypt/registerkey".to_string()));
    upstream.shutdown();
}

#[tokio::test]
async fn a_retired_pool_refuses_further_writes() {
    let dir = TempDir::new("pool-retired");
    let path = dir.write("device_pool.json", &pool_json(5));
    let pool = DevicePool::new(&path).await.expect("pool");

    // Simulate the new core having written its own state.
    let marker = "{\"android\":[],\"last_update\":\"new-core\"}";
    std::fs::write(&path, marker).expect("overwrite");

    pool.retire().await;
    assert!(pool.is_retired());

    // Every path that would persist (get -> save) must leave the file alone.
    let device = pool
        .get()
        .await
        .expect("the retired pool is still readable");
    assert!(!device.device_id.is_empty());
    assert_eq!(
        std::fs::read_to_string(&path).expect("pool file"),
        marker,
        "a retired pool must not overwrite the new instance's device state"
    );
}

/// A pool whose `secret_key`s are all empty - the state a failing `registerKey`
/// leaves behind, since `register()` still returns `Ok` in that case.
fn empty_key_pool_json(count: usize) -> String {
    let devices: Vec<serde_json::Value> = (0..count)
        .map(|i| {
            serde_json::json!({
                "device_id": format!("{i:016}"),
                "install_id": format!("{}", 9_000_000_000_000_000_000u64 + i as u64),
                "secret_key": "",
                "platform": "android",
                "status": "active",
                "created_time": "2026-01-01 00:00:00",
                "last_used": format!("2026-01-01 00:00:{i:02}"),
                "use_count": i,
                "cdid": format!("cdid-{i}"),
            })
        })
        .collect();
    serde_json::json!({ "android": devices, "last_update": "2026-01-01 00:00:00" }).to_string()
}

/// A mock upstream whose `registerKey` never yields a key, so every
/// registration comes back with an empty `secret_key`.
async fn register_key_fails_upstream() -> MockUpstream {
    MockUpstream::start(|recorded| match recorded.path.as_str() {
        "/service/2/device_register/" => MockReply::json(serde_json::json!({
            "device_id_str": "7276663560427471412",
            "install_id_str": "7276663560427471000",
        })),
        "/reading/crypt/registerkey" => MockReply::status(500),
        _ => MockReply::status(200),
    })
    .await
}

#[tokio::test]
async fn a_registration_without_a_secret_key_does_not_take_a_pool_slot() {
    let upstream = register_key_fails_upstream().await;
    let dir = TempDir::new("pool-empty-key-refill");
    let pool = DevicePool::with_mock_origin(dir.join("device_pool.json"), upstream.origin.clone())
        .await
        .expect("pool");

    let err = pool
        .refill(1)
        .await
        .expect_err("the degradation must be reported, not hidden");
    assert!(err.contains("no secret key"), "unexpected error: {err}");
    assert_eq!(
        pool.count().await,
        0,
        "a device that cannot decrypt must not occupy a pool slot"
    );
    upstream.shutdown();
}

#[tokio::test]
async fn get_never_delivers_a_device_without_a_secret_key() {
    // An empty-key slot is treated as first-launch and a registration is
    // attempted. When that registration also yields no key, get() must
    // still refuse to hand the unusable device to a caller, and must not
    // persist the failed registration in its place.
    let upstream = register_key_fails_upstream().await;
    let dir = TempDir::new("pool-empty-key-get");
    let path = dir.write("device_pool.json", &empty_key_pool_json(MIN_POOL_SIZE));
    let pool = DevicePool::with_mock_origin(path, upstream.origin.clone())
        .await
        .expect("pool");

    let err = pool
        .get()
        .await
        .expect_err("an unusable device must not be handed to a caller");
    assert!(err.contains("no secret key"), "unexpected error: {err}");
    assert_eq!(
        pool.count().await,
        1,
        "a failed first registration must leave the placeholder in place"
    );
    upstream.shutdown();
}

#[tokio::test]
async fn concurrent_gets_on_a_degraded_pool_stay_safe() {
    // `get()` on a placeholder / empty-key pool attempts first-launch
    // registration under `maintenance_lock`. A concurrent `get()` must not
    // index an emptied vec, and neither call may return the unusable device.
    // Registration is made to fail here so the check stays focused on the
    // invariant rather than a successful bind.
    let upstream = MockUpstream::start(|_| MockReply::status(500)).await;
    let dir = TempDir::new("pool-empty-key-race");
    let path = dir.write("device_pool.json", &empty_key_pool_json(1));
    let pool = DevicePool::with_mock_origin(path, upstream.origin.clone())
        .await
        .expect("pool");

    let (a, b, c, d) = tokio::join!(pool.get(), pool.get(), pool.get(), pool.get());
    for device in [a, b, c, d].into_iter().flatten() {
        assert!(
            !device.secret_key.is_empty(),
            "get() must never return a device that cannot decrypt"
        );
    }
    upstream.shutdown();
}

#[tokio::test]
async fn regenerate_replaces_the_single_persisted_device() {
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

    let dir = TempDir::new("pool-regen");
    let path = dir.write("device_pool.json", &pool_json(1));
    let pool = DevicePool::with_mock_origin(path.clone(), upstream.origin.clone())
        .await
        .expect("pool");
    let before = pool.get().await.expect("seed");
    assert_eq!(before.device_id, "0000000000000000");

    let fresh = pool.regenerate().await.expect("regenerate");
    assert_eq!(fresh.device_id, "7276663560427471412");
    assert_eq!(fresh.secret_key, REGISTERKEY_HEX);
    assert_eq!(pool.count().await, 1);
    let again = pool.get().await.expect("after regen");
    assert_eq!(again.device_id, fresh.device_id);
    assert!(pool.get_by_id(&before.device_id).await.is_err());

    let persisted: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("pool file")).expect("json");
    assert_eq!(persisted["android"].as_array().map(|a| a.len()), Some(1));
    assert_eq!(
        persisted["android"][0]["device_id"].as_str(),
        Some("7276663560427471412")
    );
    upstream.shutdown();
}

#[tokio::test]
async fn replace_failed_does_not_create_a_new_device() {
    let dir = TempDir::new("pool-no-auto-replace");
    let path = dir.write("device_pool.json", &pool_json(1));
    let pool = DevicePool::new(&path).await.expect("pool");
    let before = pool.get().await.expect("seed");

    let err = pool
        .replace_failed(&before.device_id)
        .await
        .expect_err("must surface the failure");
    assert!(err.contains("not replaced"), "unexpected error: {err}");
    assert_eq!(pool.count().await, 1);
    let after = pool.get().await.expect("same device");
    assert_eq!(after.device_id, before.device_id);
}

/// The shipped `device_pool.example.json` seed: all-zero ids, empty secret.
fn placeholder_pool_json() -> String {
    serde_json::json!({
        "android": [{
            "device_id": "0000000000000000",
            "install_id": "0000000000000000",
            "secret_key": "",
            "platform": "android",
            "status": "active",
            "created_time": "1970-01-01 00:00:00",
            "last_used": "1970-01-01 00:00:00",
            "use_count": 0
        }],
        "last_update": "1970-01-01 00:00:00"
    })
    .to_string()
}

async fn successful_register_mock() -> MockUpstream {
    MockUpstream::start(|recorded| match recorded.path.as_str() {
        "/service/2/device_register/" => MockReply::json(serde_json::json!({
            "device_id_str": "7276663560427471412",
            "install_id_str": "7276663560427471000",
        })),
        "/reading/crypt/registerkey" => MockReply::json(serde_json::json!({
            "data": { "key": REGISTERKEY_INPUT_B64 },
        })),
        _ => MockReply::status(200),
    })
    .await
}

#[tokio::test]
async fn placeholder_pool_registers_one_real_device_on_first_get() {
    let upstream = successful_register_mock().await;
    let dir = TempDir::new("pool-placeholder-get");
    let path = dir.write("device_pool.json", &placeholder_pool_json());
    let pool = DevicePool::with_mock_origin(path.clone(), upstream.origin.clone())
        .await
        .expect("pool");

    let device = pool.get().await.expect("first-launch register");
    assert_eq!(device.device_id, "7276663560427471412");
    assert_eq!(device.secret_key, REGISTERKEY_HEX);
    assert_eq!(pool.count().await, 1);
    let again = pool.get().await.expect("reuse");
    assert_eq!(again.device_id, device.device_id);
    assert_eq!(pool.count().await, 1, "must not register a second device");

    let persisted: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("pool file")).expect("json");
    assert_eq!(
        persisted["android"][0]["device_id"].as_str(),
        Some("7276663560427471412")
    );
    assert_ne!(
        persisted["android"][0]["secret_key"].as_str().unwrap_or(""),
        ""
    );
    upstream.shutdown();
}

#[tokio::test]
async fn empty_pool_registers_one_real_device_on_first_get() {
    let upstream = successful_register_mock().await;
    let dir = TempDir::new("pool-empty-get");
    let pool = DevicePool::with_mock_origin(dir.join("device_pool.json"), upstream.origin.clone())
        .await
        .expect("pool");
    assert_eq!(pool.count().await, 0);

    let device = pool.get().await.expect("first-launch register");
    assert_eq!(device.device_id, "7276663560427471412");
    assert_eq!(device.secret_key, REGISTERKEY_HEX);
    assert_eq!(pool.count().await, 1);
    upstream.shutdown();
}

#[tokio::test]
async fn concurrent_gets_on_a_placeholder_pool_register_once() {
    let upstream = successful_register_mock().await;
    let dir = TempDir::new("pool-placeholder-race");
    let path = dir.write("device_pool.json", &placeholder_pool_json());
    let pool = DevicePool::with_mock_origin(path, upstream.origin.clone())
        .await
        .expect("pool");

    let (a, b, c, d) = tokio::join!(pool.get(), pool.get(), pool.get(), pool.get());
    let ids: Vec<String> = [a, b, c, d]
        .into_iter()
        .map(|r| r.expect("concurrent first launch").device_id)
        .collect();
    assert!(
        ids.iter().all(|id| id == "7276663560427471412"),
        "every waiter must receive the same registered identity: {ids:?}"
    );
    assert_eq!(pool.count().await, 1);

    let registers = upstream
        .requests()
        .iter()
        .filter(|r| r.path == "/service/2/device_register/")
        .count();
    assert_eq!(
        registers, 1,
        "single-flight first launch must hit device_register once, got {registers}"
    );
    upstream.shutdown();
}
