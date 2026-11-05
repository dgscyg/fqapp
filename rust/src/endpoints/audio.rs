//! Audio endpoints: book detail, chapter playinfo, audio play, and the
//! speech-text timeline.

use futures::future::BoxFuture;
use serde_json::{Map, Value};

use crate::endpoints::base::{
    dragon_read_headers, Upstream, UpstreamMode, UpstreamRequestSpec, HOST_FQNOVEL,
};
use crate::endpoints::util::default_val;
use crate::endpoints::{Ctx, Params, Server};
use crate::error::{ApiError, ApiResult};

/// Go's url.Values.Set.
pub(crate) fn set_param(params: &mut Vec<(String, String)>, key: &str, value: impl Into<String>) {
    let value = value.into();
    params.retain(|(k, _)| k != key);
    params.push((key.to_string(), value));
}

/// oversea param bag (shared in `endpoints::base`).
pub(crate) fn oversea_params() -> Vec<(String, String)> {
    crate::endpoints::base::oversea_params()
}

/// Audio book detail: GET /api/v1/audio/books/{id}.
const AUDIO_BOOK_DETAIL_PATH: &str = "/reading/bookapi/audio/detail/";

fn handle_book_detail<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let book_id = params.get_str("book_id");
        if book_id.is_empty() {
            return Err(ApiError::BadRequest("缺少book_id参数".to_string()));
        }

        let mut p = oversea_params();
        set_param(&mut p, "version", "v3");
        set_param(&mut p, "book_id", book_id);

        Upstream::new(ctx.up.clone())
            .json(&UpstreamRequestSpec {
                mode: UpstreamMode::DeviceSigned,
                host: HOST_FQNOVEL.to_string(),
                path: AUDIO_BOOK_DETAIL_PATH.to_string(),
                params: p,
                headers: dragon_read_headers(),
                ..Default::default()
            })
            .await
    })
}

/// Audio chapter playinfo: GET /api/v1/audio/books/{id}/chapters/{cid}.
const AUDIO_CHAPTER_INFO_PATH: &str = "/reading/reader/audio/playinfo/";

fn handle_chapter_info<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let book_id = params.get_str("book_id");
        let chapter_id = params.get_str("chapter_id");
        if book_id.is_empty() || chapter_id.is_empty() {
            return Err(ApiError::BadRequest(
                "缺少参数: book_id / chapter_id 均为必填".to_string(),
            ));
        }
        let tone_id = default_val(&params.get_str("tone_id"), "0");

        let mut p = oversea_params();
        set_param(&mut p, "book_id", book_id);
        set_param(&mut p, "item_ids", chapter_id);
        set_param(&mut p, "tone_id", tone_id);
        set_param(&mut p, "req_type", "0");
        set_param(&mut p, "is_local_book", "false");

        let mut payload = Upstream::new(ctx.up.clone())
            .json(&UpstreamRequestSpec {
                mode: UpstreamMode::DeviceSigned,
                host: HOST_FQNOVEL.to_string(),
                path: AUDIO_CHAPTER_INFO_PATH.to_string(),
                params: p,
                headers: dragon_read_headers(),
                ..Default::default()
            })
            .await?;
        patch_playinfo_keys(&mut payload);
        Ok(payload)
    })
}

/// Derives key_hex for every encrypted stream inside an embedded
/// video_model JSON string; returns whether anything changed.
fn patch_model_stream_keys(model: &mut Map<String, Value>) -> bool {
    let Some(list) = model.get_mut("video_list").and_then(|v| v.as_array_mut()) else {
        return false;
    };
    let mut changed = false;
    for item in list.iter_mut() {
        let Some(stream) = item.as_object_mut() else {
            continue;
        };
        let Some(encryption) = stream
            .get_mut("encrypt_info")
            .and_then(|v| v.as_object_mut())
        else {
            continue;
        };
        if !encryption
            .get("encrypt")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            continue;
        }
        let spade_a = encryption
            .get("spade_a")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let key = crate::endpoints::video::derive_spade_content_key(&spade_a);
        if key.len() != 16 {
            continue;
        }
        encryption.insert("key_hex".to_string(), Value::String(hex::encode(&key)));
        changed = true;
    }
    changed
}

/// patchPlayinfoKeys: the playinfo answer nests the streams in
/// data[].video_model (a JSON string).
fn patch_playinfo_keys(v: &mut Value) {
    let Some(root) = v.as_object_mut() else {
        return;
    };
    let Some(rows) = root.get_mut("data").and_then(|d| d.as_array_mut()) else {
        return;
    };
    for row in rows.iter_mut() {
        let Some(entry) = row.as_object_mut() else {
            continue;
        };
        let raw_model = match entry.get("video_model").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        let Some(mut model) = crate::endpoints::video::decode_video_model_str(&raw_model) else {
            continue;
        };
        if !patch_model_stream_keys(&mut model) {
            continue;
        }
        if let Ok(patched) = serde_json::to_string(&model) {
            entry.insert("video_model".to_string(), Value::String(patched));
        }
    }
}

pub fn register(s: &mut Server) {
    s.add_route("audio_book_detail", handle_book_detail);
    s.add_route("audio_chapter_info", handle_chapter_info);
    s.add_route(
        "audio_timeline",
        crate::endpoints::audio_text::handle_timeline,
    );
    s.add_route(
        "audio_timepoint",
        crate::endpoints::audio_text::handle_timepoint,
    );
}

#[cfg(test)]
mod key_vectors {
    //! key_hex golden vectors from the reference test suite.

    use super::*;
    use serde_json::json;

    fn audio_model(spade_a: &str, kid: &str) -> String {
        format!(
            r#"{{"status":10,"media_type":"audio","video_duration":229.96,"video_list":[{{"main_url":"https://cdn/x","backup_url":"https://cdn/y","encrypt_info":{{"encrypt":true,"kid":"{kid}","spade_a":"{spade_a}","encryption_method":"cenc-aes-ctr"}}}}]}}"#
        )
    }

    #[test]
    fn patches_playinfo_keys_in_the_chapter_shape() {
        let model = audio_model(
            "nbwTwF23F/5phBHDb4Qjz3KBC/lBtzrJcLI4y2qGEc9dhRWhoQ==",
            "677752d8f8818bb4b2223c0e00f7a81a",
        );
        let mut value = json!({
            "code": 0,
            "data": [ { "item_id": "7181453438667096588", "video_model": model } ]
        });
        patch_playinfo_keys(&mut value);

        let raw = value["data"][0]["video_model"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(raw).unwrap();
        assert_eq!(
            parsed["video_list"][0]["encrypt_info"]["key_hex"],
            "0f7a32fda0f04388ba27cf1d0a95b024"
        );
    }
}
