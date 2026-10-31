//! `homepage_recommend` and `series_feed` endpoints.
//! The private `reading724Params` builder below duplicates logic that the
//! Rust `content_util` module does not expose.

use futures::future::BoxFuture;
use serde_json::Value;

use crate::endpoints::base::{dragon_read_headers, Upstream, UpstreamMode, UpstreamRequestSpec};
use crate::endpoints::base::{HOST_FQNOVEL, HOST_FQNOVELC};
use crate::endpoints::session::{record_device_session, session_id_from_response};
use crate::endpoints::util::{default_val, raw_json};
use crate::endpoints::{Ctx, Params, Server, INTERNAL_DEVICE_PIN_KEY};
use crate::error::{ApiError, ApiResult};

const HOMEPAGE_RECOMMEND_PATH: &str = "/reading/bookapi/bookmall/tab/v";
const SERIES_FEED_PATH: &str = "/reading/bookapi/bookmall/cell/change/v";
const SERIES_RELATED_PATH: &str = "/reading/bookapi/plan/v";

/// Go's `reading724Params` (content_util.go).
/// Go `fqclient.BuildCommonParams` 的对齐实现：国际版身份（aid=1967
/// novelapp 6.8.1.32 googleplay）+ **注册设备自己的机型档案**（占位符由
/// Upstream 按设备替换）+ `_rticket`。
///
/// 这里曾经是国内版 7.2.4.32 的固定抓包（device_type=25053RT47C/
/// rom_version=miui_...），与注册设备的档案不一致——阅读端点能容忍，
/// 书城/搜索的分类流（`bookmall/tab`）会因此拿不到各分类的区块
/// （实测 2026-10-31：听书 tab 返回 0 个 cell）。
pub(crate) fn reading724_params() -> Params {
    let mut p = Params::new();
    p.set("iid", "{install_id}");
    p.set("device_id", "{device_id}");
    p.set("ac", "wifi");
    p.set("channel", "googleplay");
    p.set("aid", "1967");
    p.set("app_name", "novelapp");
    p.set("version_code", "68132");
    p.set("version_name", "6.8.1.32");
    p.set("device_platform", "android");
    p.set("os", "android");
    p.set("ssmix", "a");
    p.set("device_type", "{device_type}");
    p.set("device_brand", "{device_brand}");
    p.set("language", "zh");
    p.set("os_api", "{os_api}");
    p.set("os_version", "{os_version}");
    p.set("manifest_version_code", "68132");
    p.set("resolution", "{resolution}");
    p.set("dpi", "{dpi}");
    p.set("update_version_code", "68132");
    p.set("host_abi", "{host_abi}");
    p.set("dragon_device_type", "phone");
    p.set("pv_player", "68132");
    p.set("compliance_status", "0");
    p.set("need_personal_recommend", "1");
    p.set("player_so_load", "1");
    p.set("is_android_pad_screen", "0");
    p.set("rom_version", "{rom_version}");
    p.set("cdid", "{cdid}");
    p.set("_rticket", "{rticket}");
    p
}
fn homepage_recommend<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let tab_type = default_val(&params.get_str("tab_type"), "2");
        let offset = default_val(&params.get_str("offset"), "0");
        let session_id = params.get_str("session_id");

        // Go `BookmallService.ExploreTab` 的形状：BuildCommonParams +
        // tab_type + offset + client_req_type=1（+ session_uuid），走搜索域
        // （c）并带 Bearer。此前照国内版抓包塞了约 55 个参数（tab_index /
        // last_tab_index / screen_width_px …）且版本号与机型都是国内 7.2.4.32
        // 的固定值——上游因此只回答「推荐」，听书/看剧/漫画的书城流全空
        // （实测 2026-10-31：极简形状下四类都返回 cell_data）。
        let mut p = reading724_params();
        p.set("tab_type", tab_type);
        p.set("offset", offset);
        p.set("client_req_type", "1");
        if !session_id.is_empty() {
            p.set("session_uuid", session_id.clone());
        }

        let mut headers = dragon_read_headers();
        headers.push(("Authorization".to_string(), "Bearer".to_string()));

        let spec = UpstreamRequestSpec {
            mode: UpstreamMode::DeviceSigned,
            host: HOST_FQNOVELC.to_string(),
            path: HOMEPAGE_RECOMMEND_PATH.to_string(),
            params: p.to_pairs(),
            headers,
            pin_device: params.get_str(INTERNAL_DEVICE_PIN_KEY),
            ..Default::default()
        };

        let (raw, dev) = Upstream::new(ctx.up.clone()).raw_with_device(&spec).await?;
        if !dev.device_id.is_empty() {
            let opened = session_id_from_response(&raw);
            if !opened.is_empty() {
                record_device_session(&opened, &dev.device_id);
            }
            if !session_id.is_empty() {
                record_device_session(&session_id, &dev.device_id);
            }
        }
        raw_json(&raw)
    })
}

fn series_feed<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let cell_id = params.get_str("cell_id");
        if cell_id.is_empty() {
            return Err(ApiError::BadRequest("缺少cell_id参数".to_string()));
        }

        let tab_type = default_val(&params.get_str("tab_type"), "16");
        let offset = default_val(&params.get_str("offset"), "0");
        let limit = default_val(&params.get_str("limit"), "10");
        // 首屏 ChangeFilter(2)、翻页 GetMore(1)：官方 `c.java` 的 q()/m() 分支。
        let change_type = default_val(&params.get_str("unlimited_selector_change_type"), "2");
        let client_req_type = default_val(&params.get_str("client_req_type"), "2");
        let session_id = params.get_str("session_id");
        let selected_items = params.get_str("selected_items");
        // 短剧/漫剧 tab 走 `video_feed_refactor` 形态：`algo_type` 默认不下发，
        // `client_template` 默认 12（Go `ExploreCell` 的 ModeDrama 分支）。
        let drish = matches!(tab_type.as_str(), "8" | "24");
        let client_template = params.get_str("client_template");
        let version_tag = params.get_str("version_tag");

        // Go `BookmallService.ExploreCell` 的形状：BuildCommonParams + cell_id /
        // tab_type / offset / limit / change_type / client_req_type /
        // algo_type / session_uuid（+ 短剧的 version_tag / client_template）。
        // 此前这里塞了约 50 个国内 7.2.4.32 抓包参数（screen_width_px /
        // cold_start_session / network_type …），会话翻页被上游判成 101116。
        let mut p = reading724_params();
        p.set("cell_id", cell_id);
        p.set("tab_type", tab_type);
        p.set("offset", offset);
        p.set("limit", limit);
        p.set("change_type", change_type);
        p.set("client_req_type", client_req_type);
        if !drish {
            p.set("algo_type", "101");
        }
        if !session_id.is_empty() {
            p.set("session_uuid", session_id.clone());
        }
        if !selected_items.is_empty() {
            p.set("selected_items", selected_items);
        }
        if drish || !version_tag.is_empty() {
            p.set(
                "version_tag",
                if version_tag.is_empty() {
                    "video_feed_refactor".to_string()
                } else {
                    version_tag
                },
            );
            p.set(
                "client_template",
                if client_template.is_empty() {
                    "12".to_string()
                } else {
                    client_template
                },
            );
        } else if !client_template.is_empty() {
            p.set("client_template", client_template);
        }

        let mut headers = dragon_read_headers();
        headers.push(("Authorization".to_string(), "Bearer".to_string()));

        let spec = UpstreamRequestSpec {
            mode: UpstreamMode::DeviceSigned,
            host: HOST_FQNOVELC.to_string(),
            path: SERIES_FEED_PATH.to_string(),
            params: p.to_pairs(),
            headers,
            pin_device: params.get_str(INTERNAL_DEVICE_PIN_KEY),
            ..Default::default()
        };

        let (raw, dev) = Upstream::new(ctx.up.clone()).raw_with_device(&spec).await?;
        if !dev.device_id.is_empty() {
            let opened = session_id_from_response(&raw);
            if !opened.is_empty() {
                record_device_session(&opened, &dev.device_id);
            }
            if !session_id.is_empty() {
                record_device_session(&session_id, &dev.device_id);
            }
        }
        raw_json(&raw)
    })
}

/// 频道表（官方 `GET /reading/bookapi/bookmall/tab/v`，`BookstoreTabResponse`
/// 的 `data: TabDataList`，其 `tab_item` 是 `Vec<BookstoreTabData>`）。
///
/// 官方**不用静态频道表**：`m0.java:3051-3089` 把 `tab_item` 逐条转成
/// `BookMallTabData`，名字直接取 `bookstoreTabData.title`（`:3058`），
/// 类型取 `tab_type`（`:3086`）。本路由把这两段原样透传，客户端的频道条
/// 就由服务端配置驱动。
///
/// 参数是 `tab_type`（当前选中的频道）与 `last_tab_type`（上次选中的频道，
/// 官方存 SP `last_tab_type`，无值 -1）：与 `homepage_recommend` 不同的地方
/// 是这里**必须**原样返回整个 `tab_item`，不能只挑一个 tab。
///
/// `bottom_tab_type` 必须是 7（官方 `BottomTabBarItemType.VideoSeriesFeedTab`
/// = 7，书城才是 0）：服务端靠它区分语境，0 会拿到**书城**频道条
///（书城把 video_feed=16 那条叫「视频」，且没有最近/收藏），短剧页的频道条
/// 就会塌成「看剧/视频」两条。`client_req_type` 同理对齐官方进页语义
/// `ClientReqType.Open` = 3（我们此前发 1 = Refresh）。
/// 证据：J:seriesmall/SeriesMallVM.java:141-176、J:rpc/model/BottomTabBarItemType.java、
/// J:rpc/model/ClientReqType.java。
fn channel_tabs<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let tab_type = default_val(&params.get_str("tab_type"), "16");
        let last_tab_type = default_val(&params.get_str("last_tab_type"), "-1");
        let mut p = reading724_params();
        p.set("tab_type", tab_type);
        p.set("last_tab_type", last_tab_type);
        p.set("client_template", "0");
        p.set("bottom_tab_type", "7");
        p.set("landing_bottom_tab_type", "0");
        p.set("client_req_type", "3");
        p.set("app_mode", "0");
        p.set("classic_tab_style", "v3");
        p.set("lore_tab_style", "v5");
        p.set("migration_top_tab_enable", "false");
        p.set("auth_aweme", "false");
        p.set("auth_backward", "true");
        p.set("cold_start_is_double_gd", "false");
        p.set("after_genre_preference_popup", "0");
        p.set("first_use_category_select", "false");
        p.set("current_name", "");
        p.set("book_id", "0");
        p.set("last_tab_index", "0");
        p.set("enable_search_box_collapse", "false");
        p.set("top_tab_extra", "");

        let spec = UpstreamRequestSpec {
            mode: UpstreamMode::DeviceSigned,
            host: HOST_FQNOVEL.to_string(),
            path: HOMEPAGE_RECOMMEND_PATH.to_string(),
            params: p.to_pairs(),
            headers: dragon_read_headers(),
            pin_device: params.get_str(INTERNAL_DEVICE_PIN_KEY),
            ..Default::default()
        };
        let (raw, dev) = Upstream::new(ctx.up.clone()).raw_with_device(&spec).await?;
        if !dev.device_id.is_empty() {
            let opened = session_id_from_response(&raw);
            if !opened.is_empty() {
                record_device_session(&opened, &dev.device_id);
            }
        }
        raw_json(&raw)
    })
}

// Official lo3/f.java:281-285 uses scene=10, not a general recommendation.
// Note: .agents/notes/implemented/feature/2026-10-15-related-series.md
fn series_related<'a>(ctx: &'a Ctx, params: &'a Params) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let series_id = params.get_str("series_id");
        if series_id.is_empty() || !series_id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ApiError::BadRequest("缺少有效的series_id参数".to_string()));
        }
        let mut p = reading724_params();
        p.set("scene", "10");
        p.set("from", "series_short_play_info");
        p.set("book_id", series_id.clone());
        p.set("filter_ids", series_id);
        Upstream::new(ctx.up.clone())
            .json(&UpstreamRequestSpec {
                mode: UpstreamMode::DeviceSigned,
                host: HOST_FQNOVEL.to_string(),
                path: SERIES_RELATED_PATH.to_string(),
                params: p.to_pairs(),
                headers: dragon_read_headers(),
                ..Default::default()
            })
            .await
    })
}

pub fn register(s: &mut Server) {
    s.add_route("homepage_recommend", homepage_recommend);
    s.add_route("channel_tabs", channel_tabs);
    s.add_route("series_feed", series_feed);
    s.add_route("series_related", series_related);
}
