//! `content` endpoint.

use futures::future::BoxFuture;
use serde_json::{Map, Value};

use crate::endpoints::base::{
    dragon_read_headers, Upstream, UpstreamMode, UpstreamRequestSpec, HOST_FQNOVEL,
};
use crate::endpoints::{Ctx, Params, Server};
use crate::error::{ApiError, ApiResult};

/// Go `NovelService` / md3 `ComicFull`：国际版正文走 BaseURL
/// （sinfonlineb）+ 通用参数包。
pub(crate) const CONTENT_FULL_PATH: &str = "/reading/reader/full/v";
pub(crate) const CONTENT_BATCH_PATH: &str = "/reading/reader/batch_full/v";

/// Clients that `Uri.encodeComponent` a comma-joined list *and* then let
/// `Uri.replace(queryParameters:)` encode again send `id1%252Cid2`, which
/// arrives here as one token (`id1%2Cid2`) instead of two ids. Real item
/// ids are numeric, so turning a leftover `%2C` back into a comma is safe.
fn normalize_batch_item_ids(raw: &str) -> String {
    raw.replace("%2C", ",").replace("%2c", ",")
}

/// 正文请求（单章 full / 多章 batch_full）的国际版请求描述。
pub(crate) fn content_spec(item_ids: &str, api_type: &str, book_id: &str) -> UpstreamRequestSpec {
    let mut p = Params::from_pairs(crate::endpoints::base::oversea_params());
    let path = if api_type == "full" {
        p.set("item_id", item_ids);
        if !book_id.is_empty() {
            p.set("book_id", book_id);
        }
        CONTENT_FULL_PATH
    } else {
        p.set("item_ids", item_ids);
        p.set("book_id", if book_id.is_empty() { "0" } else { book_id });
        p.set("req_type", "1");
        p.set("novel_text_type", "1");
        CONTENT_BATCH_PATH
    };
    UpstreamRequestSpec {
        mode: UpstreamMode::DeviceSigned,
        host: HOST_FQNOVEL.to_string(),
        path: path.to_string(),
        params: p.to_pairs(),
        headers: dragon_read_headers(),
        ..Default::default()
    }
}

async fn handle_single(
    ctx: &Ctx,
    item_id: &str,
    api_type: &str,
    book_id_hint: &str,
) -> ApiResult<Value> {
    let up = Upstream::new(ctx.up.clone());
    let (raw, dev) = up
        .raw_with_device(&content_spec(item_id, api_type, book_id_hint))
        .await?;

    let mut payload: Value = serde_json::from_slice(&raw)
        .map_err(|e| ApiError::Internal(format!("parse upstream json: {e}")))?;
    drop(raw);
    let content = payload
        .get("data")
        .and_then(|d| d.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");
    if content.is_empty() {
        return Ok(payload);
    }

    let plain = crate::crypto::decrypt_upstream(content, &dev.secret_key)
        .map_err(|err| ApiError::Internal(format!("decrypt content: {err}")))?;
    let text = String::from_utf8_lossy(&plain).into_owned();
    if let Some(data) = payload.get_mut("data").and_then(Value::as_object_mut) {
        data.insert("content".to_string(), Value::String(text));
        data.insert("content_decrypted".to_string(), Value::Bool(true));
    }
    Ok(payload)
}

/// `fullItemContent`.
pub fn full_item_content(v: &Value, item_id: &str) -> (String, String, bool) {
    let item = match v
        .get("data")
        .and_then(|d| d.get("item_infos"))
        .and_then(|i| i.get(item_id))
    {
        Some(i) => i,
        None => return (String::new(), String::new(), false),
    };
    let content = item
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let title = item
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let ok = !content.trim().is_empty();
    (content, title, ok)
}

async fn handle_batch(ctx: &Ctx, item_ids: &str, book_id: &str) -> ApiResult<Value> {
    let up = Upstream::new(ctx.up.clone());
    let (raw, dev) = up
        .raw_with_device(&content_spec(item_ids, "batch", book_id))
        .await?;

    let mut root: Value = serde_json::from_slice(&raw)
        .map_err(|e| ApiError::Internal(format!("parse upstream json: {e}")))?;
    if !matches!(
        root.get("data").and_then(|d| d.as_object()),
        Some(d) if !d.is_empty()
    ) {
        return Ok(root);
    }
    // Take the map out of `root` instead of cloning it: every chapter body was
    // copied once only to replace the original immediately afterwards.
    let data = root
        .as_object_mut()
        .and_then(|root_obj| root_obj.get_mut("data"))
        .and_then(|d| d.as_object_mut())
        .expect("data is a non-empty object, checked above");

    let mut out: Map<String, Value> = Map::new();
    for (id, item) in std::mem::take(data) {
        match item {
            Value::Object(mut m) => {
                let enc = m
                    .get("content")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                if !enc.is_empty() {
                    match crate::crypto::decrypt_upstream(&enc, &dev.secret_key) {
                        Ok(plain) => {
                            m.insert(
                                "content".to_string(),
                                Value::from(String::from_utf8_lossy(&plain).into_owned()),
                            );
                        }
                        Err(err) => {
                            log::warn!("batch_full: decrypt chapter {id} failed: {err}");
                        }
                    }
                }
                out.insert(id, Value::Object(m));
            }
            other => {
                out.insert(id, other);
            }
        }
    }
    if let Some(root_obj) = root.as_object_mut() {
        root_obj.insert("data".to_string(), Value::Object(out));
    }
    Ok(root)
}

fn handle<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let item_ids = normalize_batch_item_ids(&params.get_str("item_ids"));
        if item_ids.is_empty() {
            return Err(ApiError::BadRequest(
                "missing item_ids parameter".to_string(),
            ));
        }
        let api_type = crate::endpoints::util::default_val(&params.get_str("api_type"), "full");

        let batch = item_ids.contains(',') || api_type == "batch";
        if batch {
            return handle_batch(ctx, &item_ids, &params.get_str("book_id")).await;
        }
        // 图文正文（原 novel.snssdk.com 头条接口）改走国际版 full/v：
        // 同一份 HTML（含插图 <img>），解密后带 content_decrypted 标记。
        let api_type = if api_type == "novel" {
            "full".to_string()
        } else {
            api_type
        };
        handle_single(ctx, &item_ids, &api_type, &params.get_str("book_id")).await
    })
}

pub fn register(s: &mut Server) {
    s.add_route("content", handle);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_batch_item_ids_turns_double_encoded_commas() {
        assert_eq!(normalize_batch_item_ids("111%2C222%2C333"), "111,222,333");
        assert_eq!(normalize_batch_item_ids("111%2c222"), "111,222");
        assert_eq!(normalize_batch_item_ids("111,222"), "111,222");
    }

    #[test]
    fn batch_spec_forwards_a_real_book_id() {
        let spec = content_spec("111,222", "batch", "999");
        assert_eq!(spec.path, CONTENT_BATCH_PATH);
        assert!(
            spec.params
                .iter()
                .any(|(k, v)| k == "item_ids" && v == "111,222"),
            "item_ids missing: {:?}",
            spec.params
        );
        assert!(
            spec.params
                .iter()
                .any(|(k, v)| k == "book_id" && v == "999"),
            "book_id missing: {:?}",
            spec.params
        );
    }

    #[test]
    fn batch_spec_falls_back_to_zero_without_a_book_id() {
        let spec = content_spec("111", "batch", "");
        assert!(
            spec.params.iter().any(|(k, v)| k == "book_id" && v == "0"),
            "default book_id missing: {:?}",
            spec.params
        );
    }
}
