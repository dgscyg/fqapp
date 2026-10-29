// One-off diagnostic probe for the batch_full download path (real API),
// driven through the app's own server routes: search -> directory ->
// content(batch), exactly as the Dart layer calls them.
//
// Run: FQSIGN_LIB=/path/to/libfqsign.so cargo test --test batch_probe -- --nocapture --ignored
use fqapi_core::device::register::Registrar;
use fqapi_core::sign::Manager;
use fqapi_core::upstream::UpstreamClient;
use fqapi_core::upstream::UpstreamRequest;
use fqapi_core::endpoints::{Ctx, Params, Server};
use serde_json::Value;
use std::sync::Arc;

fn collect_strings(node: &Value, key: &str, out: &mut Vec<String>, depth: usize) {
    if depth > 12 {
        return;
    }
    match node {
        Value::Object(map) => {
            if let Some(Value::String(s)) = map.get(key) {
                out.push(s.clone());
            }
            for v in map.values() {
                collect_strings(v, key, out, depth + 1);
            }
        }
        Value::Array(items) => {
            for v in items {
                collect_strings(v, key, out, depth + 1);
            }
        }
        _ => {}
    }
}

async fn call(server: &Server, route: &str, pairs: &[(&str, &str)]) -> Value {
    let params = Params::from_pairs(
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
    );
    let handler = server.handler(route).unwrap_or_else(|| panic!("no route {route}"));
    let out = match handler(&server.ctx, &params).await {
        Ok(o) => o,
        Err(e) => {
            println!("route {route} FAILED: {e:?}");
            return serde_json::json!({"route_error": format!("{e:?}")});
        }
    };
    serde_json::to_value(&out).expect("serialize")
}

#[tokio::test]
#[ignore = "hits the real API; run explicitly"]
async fn batch_full_probe() {
    // 1. Register a fresh device and build the app's server.
    let reg = Registrar::new().expect("registrar");
    let dev = reg.register().await.expect("registration failed");
    println!("device_id={} install_id={}", dev.device_id, dev.install_id);

    let pool_dir = std::env::temp_dir().join(format!("fq-batch-{}", std::process::id()));
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
    let up = Arc::new(UpstreamClient::new(pool.clone(), Manager::new()).expect("client"));
    let cfg = Arc::new(fqapi_core::config::Config::default());
    let filters = Arc::new(fqapi_core::filter::FilterManager::default());
    let ctx = Arc::new(Ctx {
        up: up.clone(),
        pool: pool.clone(),
        cfg,
        src_dir: pool_dir.join("src").to_string_lossy().into_owned(),
        filters,
    });
    let server = Server::new(ctx);

    // 2. Search through the app's route (normalized results = openable books).
    let search = call(&server, "search", &[("query", "剑来")]).await;
    let mut book_ids = Vec::new();
    collect_strings(&search, "book_id", &mut book_ids, 0);
    book_ids.retain(|s| !s.is_empty());
    book_ids.dedup();
    println!("search book_ids: {book_ids:?}");

    // 3. Directory for the first candidate that has chapters.
    let mut item_ids: Vec<String> = Vec::new();
    let mut book_id = String::new();
    for cand in &book_ids {
        let dir = call(&server, "directory", &[("book_id", cand)]).await;
        let code = dir.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
        let mut ids = Vec::new();
        collect_strings(&dir, "item_id", &mut ids, 0);
        ids.dedup();
        println!("book {cand}: dir code={code} chapters={}", ids.len());
        if code == 0 && !ids.is_empty() {
            book_id = cand.clone();
            item_ids = ids;
            break;
        }
    }
    assert!(!item_ids.is_empty(), "no book with chapters found");
    let sample: Vec<String> = item_ids.iter().take(3).cloned().collect();
    let joined = sample.join(",");
    println!("book_id={book_id} sample item_ids: {joined}");

    // 4. Batch through the app's content route (api_type=full + comma list).
    let batch = call(
        &server,
        "content",
        &[("item_ids", &joined), ("api_type", "full")],
    )
    .await;
    println!("--- batch response ---");
    let text = serde_json::to_string_pretty(&batch).unwrap();
    println!("{}", text.chars().take(1500).collect::<String>());
    println!(
        "code={:?} message={:?}",
        batch.get("code"),
        batch.get("message")
    );
    match batch.get("data") {
        Some(Value::Object(map)) => {
            println!(
                "data: {} keys: {:?}",
                map.len(),
                map.keys().take(5).collect::<Vec<_>>()
            );
            for (id, item) in map.iter().take(2) {
                if let Some(obj) = item.as_object() {
                    let content = obj.get("content").and_then(|c| c.as_str()).unwrap_or("");
                    println!(
                        "item {id}: keys={:?} content_len={} head={:?}",
                        obj.keys().collect::<Vec<_>>(),
                        content.len(),
                        &content.chars().take(80).collect::<String>()
                    );
                }
            }
        }
        other => println!("data is NOT an object: {other:?}"),
    }

    // 5. Find a real short-drama episode: search -> seriesId -> pseries -> vid.
    let drama_search = call(&server, "search", &[("query", "闪婚")]).await;
    {
        let t = serde_json::to_string_pretty(&drama_search).unwrap();
        let head: String = t.chars().take(600).collect();
        println!("--- drama search head ---\n{head}");
    }
    let mut series_ids = Vec::new();
    collect_strings(&drama_search, "series_id", &mut series_ids, 0);
    collect_strings(&drama_search, "pseries_id", &mut series_ids, 0);
    series_ids.retain(|s| !s.is_empty() && s.len() > 5);
    series_ids.dedup();
    println!("series candidates: {}", series_ids.len());
    let mut vid = String::new();
    for cand in series_ids.iter().take(5) {
        let ps = call(&server, "pseries", &[("pseries_id", cand)]).await;
        if vid.is_empty() {
            let t = serde_json::to_string_pretty(&ps).unwrap();
            println!("--- pseries response head ---\n{}", t.chars().take(900).collect::<String>());
        }
        let mut vids = Vec::new();
        collect_strings(&ps, "vid", &mut vids, 0);
        collect_strings(&ps, "video_id", &mut vids, 0);
        collect_strings(&ps, "item_id", &mut vids, 0);
        vids.retain(|s| !s.is_empty() && s.len() > 5);
        vids.dedup();
        if !vids.is_empty() {
            println!("series {cand}: {} candidate ids, first vid={}", vids.len(), vids[0]);
            vid = vids[0].clone();
            break;
        }
    }
    let mut series_id = series_ids.first().cloned().unwrap_or_default();
    if vid.is_empty() && !series_id.is_empty() {
        // The pseries directory fallback is the reading directory endpoint
        // with the series id as book_id; its items are the episodes.
        let dir2 = call(&server, "directory", &[("book_id", &series_id)]).await;
        let mut ids = Vec::new();
        collect_strings(&dir2, "item_id", &mut ids, 0);
        ids.retain(|s| !s.is_empty() && s.len() > 5);
        if !ids.is_empty() {
            vid = ids[0].clone();
            println!("directory episodes: {}, first vid={}", ids.len(), vid);
        }
    }
    let vid = if vid.is_empty() {
        println!("no series found; falling back to logged id");
        "7688719859982552088".to_string()
    } else { vid };
    println!("testing vid={vid} series={series_id}");

    // 6. book_detail_legacy + video (stream) with the same signer.
    let detail = call(&server, "book_detail_legacy", &[("book_id", &book_id)]).await;
    println!(
        "book_detail_legacy: code={:?} keys={:?}",
        detail.get("code"),
        detail.as_object().map(|o| o.keys().take(6).collect::<Vec<_>>())
    );
    let video = call(
        &server,
        "video",
        &[("video_id", "7246649164543885884"), ("mode", "stream")],
    )
    .await;
    println!("(fixture vid 7246649164543885884)");
    println!(
        "video: video_url={:?} variants={}",
        video.get("video_url").and_then(|v| v.as_str()).map(|s| s.chars().take(60).collect::<String>()),
        video.get("variants").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0)
    );
    {
        let t = serde_json::to_string_pretty(&video).unwrap();
        println!("--- video response ---\n{}", t.chars().take(500).collect::<String>());
    }

    // 7. Decisive test: the Go reference's multi_video_model endpoint, signed
    //    the same way, with the Go-style body.
    if !series_id.is_empty() && !vid.is_empty() {
        let dev2 = pool.get().await.expect("pool device");
        let up2 = up.clone();
        let pool2 = pool.clone();
        let common = format!(
            "iid={}&device_id={}&ac=wifi&channel=googleplay&aid=1967&app_name=novelapp&version_code=68132&version_name=6.8.1.32&device_platform=android&os=android&ssmix=a&device_type=ANA-AN00&device_brand=HUAWEI&language=zh&os_api=28&os_version=13&manifest_version_code=68132&resolution=1080*1920&dpi=480&update_version_code=68132&host_abi=arm64-v8a&dragon_device_type=phone&pv_player=68132&compliance_status=0&need_personal_recommend=1&player_so_load=1&is_android_pad_screen=0&rom_version=&cdid={}",
            dev2.install_id, dev2.device_id, dev2.cdid
        );
        let mvm_url = format!(
            "https://api3-normal-sinfonlinea.fqnovel.com/novel/player/multi_video_model/v1/?{}&_rticket=0",
            common
        );
        let mvm_body = serde_json::to_vec(&serde_json::json!({
            "series_id": series_id,
            "mixed_video_id_map": { vid.clone(): [vid.clone()] },
            "biz_param": {
                "video_platform": "1024",
                "need_all_video_definition": true,
            },
        }))
        .unwrap();
        let (mvm_raw, _) = up2
            .do_request_capped(
                &UpstreamRequest {
                    method: Some("POST".into()),
                    url: mvm_url,
                    body: Some(mvm_body),
                    headers: vec![(
                        "content-type".into(),
                        "application/json; charset=utf-8".into(),
                    )],
                    ..UpstreamRequest::default()
                },
                1 << 22,
            )
            .await
            .expect("multi_video_model request failed");
        let mvm_text = String::from_utf8_lossy(&mvm_raw);
        println!(
            "--- multi_video_model response ({} bytes) ---\n{}",
            mvm_raw.len(),
            mvm_text.chars().take(600).collect::<String>()
        );
    }
}