// One-off diagnostic probe for the batch_full download path (real API),
// driven through the app's own server routes: search -> directory ->
// content(batch), exactly as the Dart layer calls them.
//
// Run: FQSIGN_LIB=/path/to/libfqsign.so cargo test --test batch_probe -- --nocapture --ignored
use fqapi_core::device::register::Registrar;
use fqapi_core::sign::Manager;
use fqapi_core::upstream::UpstreamClient;
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
        up,
        pool,
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
    println!("{}", &text[..text.len().min(1500)]);
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
                        &content[..content.len().min(80)]
                    );
                }
            }
        }
        other => println!("data is NOT an object: {other:?}"),
    }
}