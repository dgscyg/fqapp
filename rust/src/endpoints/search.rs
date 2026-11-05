//! `search` endpoint (international reading search). Port of `search.go`.

use futures::future::BoxFuture;
use serde_json::Value;

use crate::endpoints::base::{
    dragon_read_headers, Upstream, UpstreamMode, UpstreamRequestSpec, HOST_FQNOVELC,
};
use crate::endpoints::{Ctx, Params, Server};
use crate::error::{ApiError, ApiResult};

/// Go `SearchService.SearchTab`：国际版搜索走搜索域（sinfonlinec）+
/// 通用参数包。原作者遗留的 api.fanqiesdk.com 头条（news_article aid=13）
/// 搜索已移除。
const READING_SEARCH_PATH: &str = "/reading/bookapi/search/tab/v";

async fn search_reading(ctx: &Ctx, query: &str, offset: &str, q: &Params) -> ApiResult<Value> {
    let dv = crate::endpoints::util::default_val;
    let count = dv(&q.get_str("count"), "10");
    let tab_type = dv(&q.get_str("tab_type"), "1");
    let passback = dv(&q.get_str("passback"), offset);
    let mut p = Params::from_pairs(crate::endpoints::base::oversea_params());
    for (k, v) in [
        ("query", query.to_string()),
        ("offset", offset.to_string()),
        ("passback", passback),
        ("count", count),
        ("tab_type", tab_type),
        ("bookshelf_search_plan", "4".to_string()),
        ("user_is_login", "0".to_string()),
        ("bookstore_tab", "2".to_string()),
        ("search_source", "1".to_string()),
        ("clicked_content", "page_search_button".to_string()),
        ("search_source_id", "click_banner".to_string()),
        ("use_lynx", "false".to_string()),
        ("use_correct", "true".to_string()),
        ("tab_name", "store".to_string()),
        ("last_consume_interval", "-1".to_string()),
        ("pad_column_cover", "0".to_string()),
        ("is_first_enter_search", "false".to_string()),
        ("last_search_page_interval", "1500".to_string()),
    ] {
        p.set(k, v);
    }
    let search_id = q.get_str("search_id");
    if !search_id.is_empty() {
        p.set("search_id", search_id);
    }
    Upstream::new(ctx.up.clone())
        .json(&UpstreamRequestSpec {
            mode: UpstreamMode::DeviceSigned,
            host: HOST_FQNOVELC.to_string(),
            path: READING_SEARCH_PATH.to_string(),
            params: p.to_pairs(),
            headers: dragon_read_headers(),
            ..Default::default()
        })
        .await
}

fn handle<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let query = {
            let primary = params.get_str("query");
            if primary.is_empty() {
                params.get_str("q")
            } else {
                primary
            }
        };
        if query.is_empty() {
            return Err(ApiError::BadRequest("missing query parameter".to_string()));
        }
        let offset = crate::endpoints::util::default_val(&params.get_str("offset"), "0");
        search_reading(ctx, &query, &offset, params).await
    })
}

/// The bookstore search-bar cue words (`/reading/bookapi/search/cue/v`).
///
/// The official bar rotates `data_v1[]` with the response's `rotate_interval_ms`
/// (`SearchWordDisplayView.m` → `VerticalFlipper.setFlipInterval`). The request
/// shape follows `y.java R()`/`j0()`: `search_source=1` (BOOKSTORE),
/// `bookstore_tab` is the tab's `tab_type`, `cue_type` rides the query.
fn search_cue<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let cue_type = crate::endpoints::util::default_val(&params.get_str("cue_type"), "0");
        let bookstore_tab = crate::endpoints::util::default_val(&params.get_str("tab_type"), "2");
        let tab_name = params.get_str("tab_name");
        let mut p = crate::endpoints::home::oversea_params();
        p.set("cue_type", cue_type);
        p.set("bookstore_tab", bookstore_tab);
        p.set("user_is_login", "0");
        p.set("search_source", "1");
        if !tab_name.is_empty() {
            p.set("tab_name", tab_name.clone());
        }
        let spec = UpstreamRequestSpec {
            mode: UpstreamMode::DeviceSigned,
            host: crate::endpoints::base::HOST_FQNOVEL.to_string(),
            path: "/reading/bookapi/search/cue/v".to_string(),
            params: p.to_pairs(),
            headers: dragon_read_headers(),
            ..Default::default()
        };
        Upstream::new(ctx.up.clone()).json(&spec).await
    })
}

/// The hot search board for the empty search screen.
///
/// Upstream is the 热搜榜 rank list (`/reading/bookapi/search/rank_list/v:version/`,
/// `show_type=472`), passed through unchanged. A 2026-10-30 HAR capture of the
/// official client shows this endpoint returns one cell with 50 tags
/// (`tag_title` plus `tag_attached` 热搜值 and `display_tag` 巅峰榜/热聊…),
/// while the search landing plan used before (`/reading/bookapi/plan/v`,
/// scene 10) returns only two `tag_title`s — which is why the board looked
/// truncated. `:version` is the literal path segment the official client
/// sends, not a placeholder.
fn hot_search<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let show_type = crate::endpoints::util::default_val(&params.get_str("show_type"), "472");
        let mut p = crate::endpoints::home::oversea_params();
        p.set("show_type", show_type);
        let spec = UpstreamRequestSpec {
            mode: UpstreamMode::DeviceSigned,
            host: crate::endpoints::base::HOST_FQNOVELC.to_string(),
            path: "/reading/bookapi/search/rank_list/v:version/".to_string(),
            params: p.to_pairs(),
            headers: dragon_read_headers(),
            ..Default::default()
        };
        Upstream::new(ctx.up.clone()).json(&spec).await
    })
}

/// The category front page (`/reading/bookapi/new_category/front/v`).
///
/// One GET returns the whole page: `data.category_tab_config.tab_list[]`,
/// each with `category_tab`/`tab_name` plus `cell_data[].atom_data[]
/// .category_data{name, pic_url, category_id}` tiles. Request shape from
/// `GetNewCategoryFrontPageRequest` (decompiled): `category_tab` is the
/// NewCategoryTabType (1=男频 by default), the rest are scene bookkeeping.
fn category_front<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let category_tab =
            crate::endpoints::util::default_val(&params.get_str("category_tab"), "1");
        let mut p = crate::endpoints::home::oversea_params();
        p.set("category_tab", category_tab);
        p.set("need_hot", "0");
        p.set("last_chose_tab", "1");
        p.set("source", "0");
        p.set("distinct_style", "0");
        p.set("new_category_tab", "1");
        p.set("sub_category_tab", "0");
        p.set("req_scene", "0");
        // v2 参数：官方 7.0.9.32 的分类页（同端点）拿到的是带 tab_list/
        // sub_tabs 的新 schema，缺这几个字段时服务端回退老结构
        // （rule.categoy_tab_dic + down_category 平铺）。
        p.set("category_tab_v2", "1");
        p.set("last_choose_tab_v2", "1");
        let spec = UpstreamRequestSpec {
            mode: UpstreamMode::DeviceSigned,
            host: crate::endpoints::base::HOST_FQNOVEL.to_string(),
            path: "/reading/bookapi/new_category/front/v".to_string(),
            params: p.to_pairs(),
            headers: dragon_read_headers(),
            ..Default::default()
        };
        Upstream::new(ctx.up.clone()).json(&spec).await
    })
}

/// 搜索联想词：国际版 `/reading/bookapi/search/suggest/v`（搜索域）。
/// gonidbg 未覆盖，实测确认：返回 `data.query_result[]`（+ `query_result_v2`）。
fn search_predict<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let keyword = crate::endpoints::util::default_val(
            &params.get_str("keyword"),
            &params.get_str("query"),
        );
        if keyword.is_empty() {
            return Err(ApiError::BadRequest("missing query parameter".to_string()));
        }
        let mut p = Params::from_pairs(crate::endpoints::base::oversea_params());
        p.set("query", keyword);
        Upstream::new(ctx.up.clone())
            .json(&UpstreamRequestSpec {
                mode: UpstreamMode::DeviceSigned,
                host: HOST_FQNOVELC.to_string(),
                path: "/reading/bookapi/search/suggest/v".to_string(),
                params: p.to_pairs(),
                headers: dragon_read_headers(),
                ..Default::default()
            })
            .await
    })
}

pub fn register(s: &mut Server) {
    s.add_route("search_predict", search_predict);
    s.add_route("search", handle);
    s.add_route("search_cue", search_cue);
    s.add_route("hot_search", hot_search);
    s.add_route("category_front", category_front);
}
