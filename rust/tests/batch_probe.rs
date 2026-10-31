// One-off diagnostic probe for the batch_full download path (real API),
// driven through the app's own server routes: search -> directory ->
// content(batch), exactly as the Dart layer calls them.
//
// Run: FQSIGN_LIB=/path/to/libfqsign.so cargo test --test batch_probe -- --nocapture --ignored
use fqapi_core::device::register::Registrar;
use fqapi_core::endpoints::{Ctx, Params, Server};
use fqapi_core::sign::Manager;
use fqapi_core::upstream::UpstreamClient;
use fqapi_core::upstream::UpstreamRequest;
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
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    );
    let handler = server
        .handler(route)
        .unwrap_or_else(|| panic!("no route {route}"));
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
    let sample: Vec<String> = item_ids.iter().take(30).cloned().collect();
    let joined = sample.join(",");
    println!("book_id={book_id} requesting {} item_ids", sample.len());

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
                        content.chars().take(80).collect::<String>()
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
            println!(
                "--- pseries response head ---\n{}",
                t.chars().take(900).collect::<String>()
            );
        }
        let mut vids = Vec::new();
        collect_strings(&ps, "vid", &mut vids, 0);
        collect_strings(&ps, "video_id", &mut vids, 0);
        collect_strings(&ps, "item_id", &mut vids, 0);
        vids.retain(|s| !s.is_empty() && s.len() > 5);
        vids.dedup();
        if !vids.is_empty() {
            println!(
                "series {cand}: {} candidate ids, first vid={}",
                vids.len(),
                vids[0]
            );
            vid = vids[0].clone();
            break;
        }
    }
    let series_id = series_ids.first().cloned().unwrap_or_default();
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
    } else {
        vid
    };
    println!("testing vid={vid} series={series_id}");

    // 6. book_detail_legacy + video (stream) with the same signer.
    let detail = call(&server, "book_detail_legacy", &[("book_id", &book_id)]).await;
    println!(
        "book_detail_legacy: code={:?} keys={:?}",
        detail.get("code"),
        detail
            .as_object()
            .map(|o| o.keys().take(6).collect::<Vec<_>>())
    );
    let video = call(
        &server,
        "video",
        &[
            ("video_id", "7673417971355618329"),
            ("mode", "stream"),
            ("series_id", "7673415977467382809"),
        ],
    )
    .await;
    println!("(fixture vid 7246649164543885884)");
    println!(
        "video: video_url={:?} variants={}",
        video
            .get("video_url")
            .and_then(|v| v.as_str())
            .map(|s| s.chars().take(60).collect::<String>()),
        video
            .get("variants")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
    );
    // 6c. Drama TOC through the app's video_detail route (the Go's
    //     DramaVideoDetail contract; works from this network).
    let toc = call(
        &server,
        "video_detail",
        &[("series_id", "7673415977467382809")],
    )
    .await;
    {
        let t = serde_json::to_string_pretty(&toc).unwrap();
        println!(
            "--- video_detail response ({} bytes) ---\n{}",
            t.len(),
            t.chars().take(500).collect::<String>()
        );
    }

    {
        let t = serde_json::to_string_pretty(&video).unwrap();
        println!(
            "--- video response ---\n{}",
            t.chars().take(500).collect::<String>()
        );
    }

    // 6b. video_model with the device's own version (68132) instead of the
    //     hardcoded 72132 — a version mismatch against the registered device
    //     is a candidate cause of the empty-body rejections.
    {
        let dev2 = pool.get().await.expect("pool device");
        let up2 = up.clone();
        let vm_url = format!(
            "https://api3-normal-sinfonlinea.fqnovel.com/novel/player/video_model/v1/?iid={}&device_id={}&aid=1967&app_name=novelapp&version_code=68132&version_name=6.8.1.32&device_platform=android&device_brand=Xiaomi&os_version=13&cdid={}",
            dev2.install_id, dev2.device_id, dev2.cdid
        );
        let vm_body = serde_json::to_vec(&serde_json::json!({
            "video_id": vid,
            "content_type": 1004,
            "biz_param": {
                "detail_page_version": 0,
                "device_level": 3,
                "disable_digg_stat": false,
                "disable_video_relate_book": false,
                "from_video_id": "",
                "need_all_video_definition": true,
                "need_mp4_align": false,
                "source": 4,
                "use_os_player": false,
                "use_server_dns": false,
                "video_platform": 3,
            },
        }))
        .unwrap();
        let (vm_raw, _) = up2
            .do_request_capped(
                &UpstreamRequest {
                    method: Some("POST".into()),
                    url: vm_url,
                    body: Some(vm_body),
                    headers: vec![(
                        "content-type".into(),
                        "application/json; charset=utf-8".into(),
                    )],
                    ..UpstreamRequest::default()
                },
                1 << 22,
            )
            .await
            .expect("video_model 68132 request failed");
        let vm_text = String::from_utf8_lossy(&vm_raw);
        println!(
            "--- video_model(68132) response ({} bytes) ---\n{}",
            vm_raw.len(),
            vm_text.chars().take(400).collect::<String>()
        );
    }

    // 6c0. The server's own tab list (tab_index -> type/title), so the
    //      听书 tab can be requested by its real index.
    {
        let h = call(
            &server,
            "homepage_recommend",
            &[("tab_type", "2"), ("tab_index", "0")],
        )
        .await;
        if let Some(tabs) = h
            .get("data")
            .and_then(|d| d.get("tab_item"))
            .and_then(|t| t.as_array())
        {
            let list: Vec<String> = tabs
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    format!(
                        "{i}:{}:{}",
                        t.get("tab_type").map(|v| v.to_string()).unwrap_or_default(),
                        t.get("title").and_then(|v| v.as_str()).unwrap_or("")
                    )
                })
                .collect();
            println!("server tabs: {}", list.join(" | "));
        } else {
            println!("server tabs: absent (code={:?})", h.get("code"));
        }
    }

    // 6c0b. The Go reference's MINIMAL home-tab shape (common params +
    //       tab_type + offset + client_req_type=1 + Bearer) on host c, for
    //       推荐/听书/看剧/漫画 — the kitchen-sink params may break tab
    //       resolution.
    for tab in ["2", "5", "8", "9"] {
        let dev4 = pool.get().await.expect("pool device");
        let up4 = up.clone();
        let url = format!(
            "https://api5-normal-sinfonlinec.fqnovel.com/reading/bookapi/bookmall/tab/v?iid={}&device_id={}&ac=wifi&channel=googleplay&aid=1967&app_name=novelapp&version_code=68132&version_name=6.8.1.32&device_platform=android&os=android&ssmix=a&device_type={}&device_brand={}&language=zh&os_api={}&os_version={}&manifest_version_code=68132&resolution={}&dpi={}&update_version_code=68132&_rticket={}&host_abi={}&dragon_device_type=phone&pv_player=68132&compliance_status=0&need_personal_recommend=1&player_so_load=1&is_android_pad_screen=0&rom_version={}&cdid={}&tab_type={tab}&offset=0&client_req_type=1",
            dev4.install_id, dev4.device_id, dev4.device_type, dev4.device_brand,
            dev4.os_api, dev4.os_version, dev4.resolution, dev4.dpi,
            fqapi_core::timeutil::now_millis(), dev4.host_abi, dev4.rom_version, dev4.cdid,
        );
        match up4
            .do_request_capped(
                &UpstreamRequest {
                    url,
                    headers: {
                        let mut h = fqapi_core::endpoints::base::common_headers();
                        h.push(("Authorization".into(), "Bearer".into()));
                        h
                    },
                    ..UpstreamRequest::default()
                },
                1 << 22,
            )
            .await
        {
            Ok((raw, _)) => {
                let v: serde_json::Value = serde_json::from_slice(&raw).unwrap_or_default();
                // The answered tab item carries the cells:
                //   data.tab_item[data.tab_index].cell_data[]
                let cells = v
                    .get("data")
                    .and_then(|d| {
                        let idx = d.get("tab_index").and_then(|i| i.as_u64())? as usize;
                        d.get("tab_item")
                            .and_then(|t| t.as_array())
                            .and_then(|a| a.get(idx))
                            .and_then(|item| item.get("cell_data"))
                            .and_then(|c| c.as_array())
                            .map(|a| a.len())
                    })
                    .unwrap_or(0);
                let data_keys: Vec<String> = v
                    .get("data")
                    .and_then(|d| d.as_object())
                    .map(|o| o.keys().cloned().collect())
                    .unwrap_or_default();
                println!(
                    "gotab tab_type={tab}: {} bytes code={:?} cells={cells} data_keys={data_keys:?}",
                    raw.len(),
                    v.get("code")
                );
            }
            Err(e) => println!("gotab tab_type={tab}: request failed: {e:?}"),
        }
    }

    // 6c1. Home feed per-kind paging: 听书 (tab_type=5) page 1 then 2.
    for offset in ["0", "12"] {
        let h = call(
            &server,
            "homepage_recommend",
            &[("tab_type", "5"), ("offset", offset), ("tab_index", "4")],
        )
        .await;
        let n = h
            .get("data")
            .and_then(|d| d.get("cell_data"))
            .and_then(|c| c.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let code = h
            .get("code")
            .map(|c| c.to_string())
            .unwrap_or_else(|| "none".into());
        let idx = h
            .get("data")
            .and_then(|d| d.get("tab_index"))
            .map(|i| i.to_string())
            .unwrap_or_default();
        let answered = h
            .get("data")
            .and_then(|d| d.get("tab_item"))
            .and_then(|t| t.as_array())
            .and_then(|a| {
                let i = h
                    .get("data")
                    .and_then(|d| d.get("tab_index"))
                    .and_then(|v| v.as_u64())? as usize;
                a.get(i)
            })
            .map(|item| {
                format!(
                    "type={} title={} keys={:?}",
                    item.get("tab_type")
                        .map(|v| v.to_string())
                        .unwrap_or_default(),
                    item.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                    item.as_object()
                        .map(|o| o.keys().cloned().collect::<Vec<_>>())
                        .unwrap_or_default()
                )
            })
            .unwrap_or_default();
        println!(
            "home tab_type=5 offset={offset}: code={code} cells={n} idx={idx} answered={answered}"
        );
    }

    // 6c2. Search bucket mapping: which tab_type fills which search_tabs
    //      bucket (综合/短剧/听书/漫画/书籍...).
    for tab_type in ["1", "2", "3", "8", "11"] {
        let r = call(
            &server,
            "search",
            &[
                ("query", "听书"),
                ("tab_type", tab_type),
                ("count", "10"),
                ("offset", "0"),
            ],
        )
        .await;
        let mut buckets = Vec::new();
        if let Some(tabs) = r.get("search_tabs").and_then(|t| t.as_array()) {
            for tab in tabs {
                let title = tab.get("title").and_then(|t| t.as_str()).unwrap_or("?");
                let n = tab
                    .get("data")
                    .and_then(|d| d.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                buckets.push(format!("{title}:{n}"));
            }
        }
        println!("search tab_type={tab_type}: {}", buckets.join(","));
    }

    // 6d. Play body variants on the TOC host (fqnovel-gonidbg capture doc
    //     2.2: mixed_video_id_map {"1":[vid]}, video_platform=1024 int).
    for (name, body) in [
        (
            "minimal doc body",
            serde_json::json!({
                "mixed_video_id_map": {"1": ["7673417971355618329"]},
                "biz_param": {"video_platform": 1024},
            }),
        ),
        (
            "doc body + need_all",
            serde_json::json!({
                "mixed_video_id_map": {"1": ["7673417971355618329"]},
                "biz_param": {"video_platform": 1024, "need_all_video_definition": true},
            }),
        ),
        (
            "doc body + series_id",
            serde_json::json!({
                "series_id": "7673415977467382809",
                "mixed_video_id_map": {"1": ["7673417971355618329"]},
                "biz_param": {"video_platform": 1024, "need_all_video_definition": true},
            }),
        ),
    ] {
        let dev3 = pool.get().await.expect("pool device");
        let up3 = up.clone();
        let url = format!(
            "https://api5-normal-sinfonlinec.fqnovel.com/novel/player/multi_video_model/v1/?iid={}&device_id={}&ac=wifi&channel=googleplay&aid=1967&app_name=novelapp&version_code=68132&version_name=6.8.1.32&device_platform=android&os=android&ssmix=a&device_type={}&device_brand={}&language=zh&os_api={}&os_version={}&manifest_version_code=68132&resolution={}&dpi={}&update_version_code=68132&_rticket={}&host_abi={}&dragon_device_type=phone&pv_player=68132&compliance_status=0&need_personal_recommend=1&player_so_load=1&is_android_pad_screen=0&rom_version={}&cdid={}",
            dev3.install_id, dev3.device_id, dev3.device_type, dev3.device_brand,
            dev3.os_api, dev3.os_version, dev3.resolution, dev3.dpi,
            fqapi_core::timeutil::now_millis(), dev3.host_abi, dev3.rom_version, dev3.cdid,
        );
        match up3
            .do_request_capped(
                &UpstreamRequest {
                    method: Some("POST".into()),
                    url,
                    body: Some(serde_json::to_vec(&body).unwrap()),
                    headers: vec![
                        (
                            "content-type".into(),
                            "application/json; charset=utf-8".into(),
                        ),
                        ("Authorization".into(), "Bearer".into()),
                    ],
                    ..UpstreamRequest::default()
                },
                1 << 22,
            )
            .await
        {
            Ok((raw, _)) => println!(
                "{name}: {} bytes: {}",
                raw.len(),
                String::from_utf8_lossy(&raw)
                    .chars()
                    .take(160)
                    .collect::<String>()
            ),
            Err(e) => println!("{name}: request failed: {e:?}"),
        }
    }

    // 7. Decisive test: the Go reference's multi_video_model endpoint, signed
    //    the same way, with the Go-style body.
    if !series_id.is_empty() && !vid.is_empty() {
        let dev2 = pool.get().await.expect("pool device");
        let up2 = up.clone();
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

async fn probe_server() -> Server {
    let reg = Registrar::new().expect("registrar");
    let dev = reg.register().await.expect("registration failed");
    println!("device_id={} install_id={}", dev.device_id, dev.install_id);
    let pool_dir = std::env::temp_dir().join(format!("fq-home-{}", std::process::id()));
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
    let ctx = Arc::new(Ctx {
        up,
        pool,
        cfg: Arc::new(fqapi_core::config::Config::default()),
        src_dir: pool_dir.join("src").to_string_lossy().into_owned(),
        filters: Arc::new(fqapi_core::filter::FilterManager::default()),
    });
    Server::new(ctx)
}

/// Exercises the real `homepage_recommend` handler for every content type, so
/// the placeholder substitution / header build under test is the shipping one.
#[tokio::test]
#[ignore = "hits the real API; run explicitly"]
async fn home_tabs_probe() {
    let server = probe_server().await;
    // kind -> search bucket tab_type the Dart layer now sends.
    for (tab_type, label) in [
        ("2", "综合"),
        ("5", "听书"),
        ("8", "看剧"),
        ("9", "漫画"),
        ("16", "视频"),
    ] {
        let out = call(
            &server,
            "homepage_recommend",
            &[("tab_type", tab_type), ("offset", "0")],
        )
        .await;
        let code = out.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        let data = out.get("data").cloned().unwrap_or(Value::Null);
        let idx = data.get("tab_index").and_then(|v| v.as_i64()).unwrap_or(-1);
        let answered = data
            .get("tab_item")
            .and_then(|v| v.as_array())
            .and_then(|a| a.get(idx.max(0) as usize))
            .cloned()
            .unwrap_or(Value::Null);
        let cells = answered
            .get("cell_data")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let tabs: Vec<String> = data
            .get("tab_item")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .enumerate()
                    .map(|(i, t)| {
                        format!(
                            "{i}:{}({})",
                            t.get("tab_type").and_then(|v| v.as_i64()).unwrap_or(-1),
                            t.get("title").and_then(|v| v.as_str()).unwrap_or("?")
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        println!(
            "tab_type={tab_type} ({label}) code={code} idx={idx} answered={}({}) cells={cells}",
            answered
                .get("tab_type")
                .and_then(|v| v.as_i64())
                .unwrap_or(-1),
            answered
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("?"),
        );
        println!("  tabs={}", tabs.join(" | "));
        // search fallback with the matching bucket
        let s = call(
            &server,
            "search",
            &[("query", label), ("tab_type", tab_type)],
        )
        .await;
        let scode = s.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        let mut ids = Vec::new();
        collect_strings(&s, "book_id", &mut ids, 0);
        ids.retain(|x| !x.is_empty());
        ids.dedup();
        println!("  search bucket code={scode} items={}", ids.len());
        // paging: the upstream binds the feed session to the opening device,
        // so page 2 must echo `session_id` from page 1.
        let first = call(
            &server,
            "homepage_recommend",
            &[("tab_type", tab_type), ("offset", "0")],
        )
        .await;
        // Upstream nests the feed session somewhere under `data`; the handler
        // finds it with the same recursive scan.
        let mut sids: Vec<String> = Vec::new();
        collect_strings(&first, "session_id", &mut sids, 0);
        sids.retain(|x| !x.is_empty());
        let sid = sids.first().cloned().unwrap_or_default();
        let second = call(
            &server,
            "homepage_recommend",
            &[
                ("tab_type", tab_type),
                ("offset", "12"),
                ("session_id", &sid),
            ],
        )
        .await;
        let scode2 = second.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        let cells2 = second
            .get("data")
            .and_then(|d| d.get("tab_item"))
            .and_then(|v| v.as_array())
            .and_then(|a| {
                let idx = second
                    .get("data")
                    .and_then(|d| d.get("tab_index"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                a.get(idx.max(0) as usize)
            })
            .and_then(|t| t.get("cell_data"))
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        println!("  page2 code={scode2} cells={cells2} sid_len={}", sid.len());

        // Stage 2: the video/audio tab's items live in a cell feed
        // (`/bookmall/cell/change/v`), paged by offset + session, not by
        // re-requesting the tab. Exercise it the way the Dart layer does.
        let mut cell_ids: Vec<String> = Vec::new();
        collect_strings(&answered, "cell_id_str", &mut cell_ids, 0);
        cell_ids.retain(|x| !x.is_empty());
        cell_ids.dedup();
        if let Some(cell_id) = cell_ids.first() {
            for off in ["0", "10"] {
                let feed = call(
                    &server,
                    "series_feed",
                    &[
                        ("cell_id", cell_id.as_str()),
                        ("tab_type", tab_type),
                        ("offset", off),
                        ("session_id", &sid),
                    ],
                )
                .await;
                let fcode = feed.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
                let mut vids = Vec::new();
                collect_strings(&feed, "series_id", &mut vids, 0);
                let mut book_ids2 = Vec::new();
                collect_strings(&feed, "book_id", &mut book_ids2, 0);
                println!(
                    "  cell feed cell_id={cell_id} offset={off} code={fcode} series={} books={}",
                    vids.len(),
                    book_ids2.len()
                );
            }
        } else {
            println!("  cell feed: no cell_id_str in page1");
        }
    }
}
