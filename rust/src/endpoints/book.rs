//! Book-family endpoints: `book_detail_legacy`, `directory`,
//! `novel_detail`, `related`, `forum_id` and `book_share`.
//!
//! 全部走国际版（com.dragon.read.oversea.gp 6.8.1.32 googleplay）身份：
//! 参数一律从 `base::oversea_params` 起步，占位符由请求层按设备替换。
//! 原作者遗留的国内版入口（fanqienovel.com 网页目录 + a_bogus、
//! novel.snssdk.com 头条目录/条目、7.0.9.32 国内版 ugc 详情）已移除。

use futures::future::BoxFuture;
use serde_json::{json, Value};

use crate::endpoints::base::{
    dragon_read_headers, dragon_read_json_headers, Upstream, UpstreamMode, UpstreamRequestSpec,
    HOST_FQNOVEL, HOST_FQNOVELC,
};
use crate::endpoints::util::default_val;
use crate::endpoints::{Ctx, Params, Server};
use crate::error::{ApiError, ApiResult};

/// 国际版通用参数包（`base::oversea_params`），`Params` 形式。
fn oversea_params() -> Params {
    Params::from_pairs(crate::endpoints::base::oversea_params())
}

fn oversea_with(extra: &[(&str, String)]) -> Vec<(String, String)> {
    let mut p = oversea_params();
    for (k, v) in extra {
        p.set(k, v.clone());
    }
    p.to_pairs()
}

async fn signed_get(
    ctx: &Ctx,
    host: &str,
    path: &str,
    params: Vec<(String, String)>,
) -> ApiResult<Value> {
    Upstream::new(ctx.up.clone())
        .json(&UpstreamRequestSpec {
            mode: UpstreamMode::DeviceSigned,
            host: host.to_string(),
            path: path.to_string(),
            params,
            headers: dragon_read_headers(),
            ..Default::default()
        })
        .await
}

// -------------------------------------------------------------------------
// book_detail_legacy
// -------------------------------------------------------------------------

const BOOK_DETAIL_LEGACY_PATH: &str = "/reading/bookapi/detail/v";

fn book_detail_legacy<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let book_id = params.get_str("book_id");
        if book_id.is_empty() {
            return Err(ApiError::BadRequest("缺少book_id参数".to_string()));
        }
        signed_get(
            ctx,
            HOST_FQNOVEL,
            BOOK_DETAIL_LEGACY_PATH,
            oversea_with(&[("book_id", book_id)]),
        )
        .await
    })
}

// -------------------------------------------------------------------------
// directory
// -------------------------------------------------------------------------

/// Go `SearchService.DirectoryWithBookType`：国际版目录走搜索域
/// （sinfonlinec）+ 通用参数。听书/漫画目录需 `book_type=1`。
const DIRECTORY_READING_PATH: &str = "/reading/bookapi/directory/all_items/v";

pub(crate) async fn directory_reading(
    ctx: &Ctx,
    book_id: &str,
    book_type: &str,
) -> ApiResult<Value> {
    signed_get(
        ctx,
        HOST_FQNOVELC,
        DIRECTORY_READING_PATH,
        oversea_with(&[
            ("book_id", book_id.to_string()),
            ("book_type", default_val(book_type, "0")),
            ("need_version", "true".to_string()),
        ]),
    )
    .await
}

fn directory<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let book_id = params.get_str("book_id");
        if book_id.is_empty() {
            return Err(ApiError::BadRequest(
                "missing book_id parameter".to_string(),
            ));
        }
        let first = directory_reading(ctx, &book_id, &params.get_str("book_type")).await?;
        // 调用方没指定 book_type 时：书籍目录（0）查不到就按听书/漫画
        // 目录（1）再试一次——同一国际版端点，只是 book_type 不同。
        if params.get_str("book_type").is_empty() && upstream_code(&first) == Some(101104) {
            if let Ok(second) = directory_reading(ctx, &book_id, "1").await {
                if upstream_code(&second) == Some(0) {
                    return Ok(second);
                }
            }
        }
        Ok(first)
    })
}

fn upstream_code(v: &Value) -> Option<i64> {
    v.get("code")
        .and_then(|c| c.as_i64().or_else(|| c.as_f64().map(|f| f as i64)))
}

// -------------------------------------------------------------------------
// novel_detail
// -------------------------------------------------------------------------

/// Go md3 `ShortPost`：国际版短故事详情。
const NOVEL_DETAIL_PATH: &str = "/reading/ugc/postdata/detail/v";

fn novel_detail<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let post_id = params.get_str("post_id");
        if post_id.is_empty() {
            return Err(ApiError::BadRequest("缺少post_id参数".to_string()));
        }
        signed_get(
            ctx,
            HOST_FQNOVEL,
            NOVEL_DETAIL_PATH,
            oversea_with(&[
                ("post_id", post_id),
                (
                    "relative_id",
                    default_val(&params.get_str("relative_id"), "0"),
                ),
                (
                    "relative_type",
                    default_val(&params.get_str("relative_type"), "5"),
                ),
                (
                    "source_type",
                    default_val(&params.get_str("source_type"), "16"),
                ),
            ]),
        )
        .await
    })
}

// -------------------------------------------------------------------------
// related
// -------------------------------------------------------------------------

const RELATED_PATH: &str = "/reading/reader/book/recommend_data_plan/v";

fn related<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let book_id = params.get_str("book_id");
        if book_id.is_empty() {
            return Err(ApiError::BadRequest(
                "missing book_id parameter".to_string(),
            ));
        }

        let mut p = oversea_params();
        p.set("book_id", book_id.clone());
        p.set("source", "5");
        // Resolved per device by the request layer like the other placeholders
        // above; a literal here would pin every install to one identifier.
        p.set("cdid", "{cdid}");

        let plan = Upstream::new(ctx.up.clone())
            .json(&UpstreamRequestSpec {
                mode: UpstreamMode::DeviceSigned,
                host: HOST_FQNOVEL.to_string(),
                path: RELATED_PATH.to_string(),
                params: p.to_pairs(),
                headers: dragon_read_headers(),
                ..Default::default()
            })
            .await?;
        let has_cells = plan
            .pointer("/data/cell_data")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty());
        if has_cells {
            return Ok(plan);
        }

        // The international plan is often empty; gonidbg's RelatedReaders
        // (lastpage recommend) is the international related-books source.
        let mut q = oversea_params();
        q.set("book_id", book_id);
        q.set("limit", "10");
        q.set("change_type", "3");
        q.set("from", "detailpage");
        q.set("type", "0");
        let last = Upstream::new(ctx.up.clone())
            .json(&UpstreamRequestSpec {
                mode: UpstreamMode::DeviceSigned,
                host: HOST_FQNOVEL.to_string(),
                path: LASTPAGE_RECOMMEND_PATH.to_string(),
                params: q.to_pairs(),
                headers: dragon_read_headers(),
                ..Default::default()
            })
            .await?;
        Ok(wrap_lastpage_recommend(last))
    })
}

const LASTPAGE_RECOMMEND_PATH: &str = "/reading/reader/book/lastpage/recommend/v";

/// Reshape a lastpage recommend list into the `cell_data` layout the client
/// reads from `recommend_data_plan`.
fn wrap_lastpage_recommend(mut root: Value) -> Value {
    let books = root.get_mut("data").map(Value::take).unwrap_or(Value::Null);
    let books = if books.is_array() {
        books
    } else {
        Value::Array(vec![])
    };
    if let Some(obj) = root.as_object_mut() {
        obj.insert(
            "data".to_string(),
            serde_json::json!({"cell_data": [{"book_data": books}], "has_more": false}),
        );
    }
    root
}

// -------------------------------------------------------------------------
// forum_id
// -------------------------------------------------------------------------

const FORUM_ID_PATH: &str = "/reading/ugc/item/mix_data/get/v";

/// `json.Marshal` in Go escapes `<`, `>` and `&` as `\u003c`, `\u003e`
/// and `\u0026`; `serde_json` does not, so apply the same escaping to the
/// serialized bytes to match the Go body byte-for-byte.
fn go_escape_json_html(body: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    for b in body {
        match b {
            b'<' => out.extend_from_slice(b"\\u003c"),
            b'>' => out.extend_from_slice(b"\\u003e"),
            b'&' => out.extend_from_slice(b"\\u0026"),
            _ => out.push(b),
        }
    }
    out
}

fn forum_id<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let author_user_id = params.get_str("author_user_id");
        let book_id = params.get_str("book_id");
        let item_id = params.get_str("item_id");
        if author_user_id.is_empty() || book_id.is_empty() || item_id.is_empty() {
            return Err(ApiError::BadRequest(
                "缺少参数: author_user_id / book_id / item_id 均为必填".to_string(),
            ));
        }

        let body = go_escape_json_html(
            serde_json::to_vec(&json!({
                "author_user_id": author_user_id,
                "book_id": book_id,
                "count": 0,
                "include_other_item_data": false,
                "item_id": item_id,
                "should_not_impr": false,
                "source_type": 38,
            }))
            .map_err(|e| ApiError::Internal(e.to_string()))?,
        );

        Upstream::new(ctx.up.clone())
            .json(&UpstreamRequestSpec {
                mode: UpstreamMode::DeviceSigned,
                host: HOST_FQNOVEL.to_string(),
                path: FORUM_ID_PATH.to_string(),
                params: oversea_params().to_pairs(),
                body: Some(body),
                headers: dragon_read_json_headers(),
                ..Default::default()
            })
            .await
    })
}

// -------------------------------------------------------------------------
// book_share
// -------------------------------------------------------------------------

/// 官方短剧分享信息路径（`m0.java:930-957`）。
///
/// 与书籍分享（`share_type=0`）共用同一个上游，只有 `share_type=7`（Video）
/// 和短剧专属的剧集上下文字段不同。官方的 `ShareRequest` 里
/// `share_type`、`album_id`、`current_chapter_id`、`first_chapter_id`、
/// `share_timestamp`、`entrance` 都在 query 上。
fn playlet_share<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let group_id = params.get_str("group_id");
        if group_id.is_empty() {
            // 官方在没有 groupId 时连接口都不打，直接弹「网络错误，请重试」
            // （m0.java:2115-2128）。
            return Err(ApiError::BadRequest("缺少group_id参数".to_string()));
        }

        let mut query = vec![
            ("tone_id".to_string(), "0".to_string()),
            ("share_type".to_string(), "7".to_string()),
            ("group_id".to_string(), group_id.clone()),
            ("only_share_status".to_string(), "false".to_string()),
            ("status".to_string(), "0".to_string()),
        ];
        // 短剧专属字段只有在调用方给了值时才带上：官方把这些放在
        // ShareRequest 里按 model 的实际取值序列化，未取到的字段不下发。
        for (key, value) in [
            ("album_id", params.get_str("album_id")),
            ("current_chapter_id", params.get_str("current_chapter_id")),
            ("first_chapter_id", params.get_str("first_chapter_id")),
            ("share_timestamp", params.get_str("share_timestamp")),
            ("entrance", params.get_str("entrance")),
            ("video_type", params.get_str("video_type")),
        ] {
            if !value.is_empty() {
                query.push((key.to_string(), value));
            }
        }
        query.extend(share_info_device_params());

        Upstream::new(ctx.up.clone())
            .json(&UpstreamRequestSpec {
                mode: UpstreamMode::DeviceSigned,
                host: HOST_FQNOVEL.to_string(),
                path: SHARE_INFO_PATH.to_string(),
                params: query,
                headers: dragon_read_headers(),
                ..Default::default()
            })
            .await
    })
}

/// 复制链接的短链兜底（`LinkShareItem.java:86-104` -> `GET /reading/user/share/short_url/`）。
fn share_short_url<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let target = params.get_str("target");
        if target.is_empty() {
            return Err(ApiError::BadRequest("缺少target参数".to_string()));
        }
        let mut query = vec![("target".to_string(), target.clone())];
        query.extend(share_info_device_params());

        Upstream::new(ctx.up.clone())
            .json(&UpstreamRequestSpec {
                mode: UpstreamMode::DeviceSigned,
                host: HOST_FQNOVEL.to_string(),
                path: SHARE_SHORT_URL_PATH.to_string(),
                params: query,
                headers: dragon_read_headers(),
                ..Default::default()
            })
            .await
    })
}

/// 分享信息/短链两处共用的设备参数，取自官方分享请求的固定 query 集合
/// （`m0.java:973-1016` 的 device 部分，与书籍分享同一套）。
fn share_info_device_params() -> Vec<(String, String)> {
    crate::endpoints::base::oversea_params()
}

/// 分享信息与短链的路径常量（相对 host）。
const SHARE_INFO_PATH: &str = "/reading/user/share/info/v";
const SHARE_SHORT_URL_PATH: &str = "/reading/user/share/short_url/";

fn book_share<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let book_id = params.get_str("book_id");
        if book_id.is_empty() {
            return Err(ApiError::BadRequest("缺少book_id参数".to_string()));
        }
        let mode = default_val(&params.get_str("mode"), "both");
        match mode.as_str() {
            "share" | "both" => {
                signed_get(
                    ctx,
                    HOST_FQNOVEL,
                    SHARE_INFO_PATH,
                    oversea_with(&[
                        ("tone_id", "0".to_string()),
                        ("share_type", "0".to_string()),
                        ("group_id", book_id),
                        ("only_share_status", "false".to_string()),
                        ("status", "0".to_string()),
                    ]),
                )
                .await
            }
            "excerpt" => {
                signed_get(
                    ctx,
                    HOST_FQNOVEL,
                    BOOK_SHARE_EXCERPT_PATH,
                    oversea_with(&[("limit", "0".to_string()), ("book_id", book_id)]),
                )
                .await
            }
            _ => Ok(json!({
                "excerpts": Value::Null,
                "share_info": Value::Null,
            })),
        }
    })
}

const BOOK_SHARE_EXCERPT_PATH: &str = "/reading/bookapi/excerpt/list/v";

pub fn register(s: &mut Server) {
    s.add_route("book_detail_legacy", book_detail_legacy);
    s.add_route("directory", directory);
    s.add_route("novel_detail", novel_detail);
    s.add_route("related", related);
    s.add_route("forum_id", forum_id);
    s.add_route("book_share", book_share);
    s.add_route("playlet_share", playlet_share);
    s.add_route("share_short_url", share_short_url);
}

#[cfg(test)]
mod related_wrap_tests {
    use super::*;

    #[test]
    fn wraps_list_into_cell_data() {
        let v = wrap_lastpage_recommend(serde_json::json!({"code":0,"data":[{"book_id":"1"}]}));
        assert_eq!(v["data"]["cell_data"][0]["book_data"][0]["book_id"], "1");
        assert_eq!(v["code"], 0);
    }
}
