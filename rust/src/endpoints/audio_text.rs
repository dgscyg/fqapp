//! 听书文本同步端点：字幕时间轴与朗读位置映射。
//!
//! 两个端点服务于「边听边读」：
//! * `timeline`  —— 播报文本 + 起始毫秒，给字幕面板用；
//! * `timepoint` —— 音频毫秒 → 正文段落/字符偏移，给正文朗读高亮用。
//!
//! 它们的共同点是都以音频时间为基准去对齐文本，且都需要真实的 `tone_id`
//! 才返回内容，所以放在一起。

use futures::future::BoxFuture;
use serde_json::Value;

use crate::endpoints::audio::{oversea_params, set_param};
use crate::endpoints::base::{
    dragon_read_headers, Upstream, UpstreamMode, UpstreamRequestSpec, HOST_FQNOVEL,
};
use crate::endpoints::util::default_val;
use crate::endpoints::{Ctx, Params};
use crate::error::{ApiError, ApiResult};

/// 听书字幕（边听边读字幕面板）。
///
/// 原实现调用的是国内版「番茄 SDK」字幕接口（api-sinfonlinea.fanqiesdk.com
/// `/api/novel/audio/speech/text/v1/`，aid=6589 gold_browser）。国际版没有
/// 对应接口：实测 sinfonlinea/b/c 三个国际版域名上同路径及
/// `audio/subtitle`、`speech_text` 等候选路径全部 404，playinfo 里也不带字幕。
/// 国内版接口已移除，这里固定回「无字幕」（1301008），客户端按空轨处理；
/// 正文朗读高亮仍由国际版 `audio/timepoint` 提供。
pub(crate) fn handle_timeline<'a>(
    _ctx: &'a Ctx,
    params: &'a Params,
) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let item_id = default_val(&params.get_str("item_id"), &params.get_str("item_ids"));
        if item_id.is_empty() {
            return Err(ApiError::BadRequest("缺少item_id参数".to_string()));
        }
        Ok(serde_json::json!({
            "code": 1301008,
            "message": "no available speech text (international API has no subtitle endpoint)",
            "data": {},
        }))
    })
}

/// Audio time points: GET → `/reading/reader/audio/timepoint/`.
///
/// Mirrors the official `AudioTimePointRequest` (`ro4/f.java` `$GET
/// /reading/reader/audio/timepoint/`). The answer is the mapping that makes
/// listen-along highlighting possible without any text matching: each entry
/// carries `start_para` / `start_para_off` / `end_para` / `end_para_off`
/// (paragraph index plus character offset inside it) against an audio window
/// `start_time` / `end_time` in milliseconds.
///
/// `req_type` selects which timeline is asked for (`AudioTimePointReqType`):
/// 1 = tts (智能朗读), 2 = audio (真人讲书), 3 = para_match_audio,
/// 4 = para_match_text, 5 = local book tts.
///
/// Measured against the live upstream (2026-10), **3 is the one that answers
/// with usable offsets**: 1 replies `101049 TTS_MAY_HASNT_PRODUCT` for books
/// without generated speech, 2 replies `101002 NOVEL_DATA_GET_ERROR`, and 4
/// answers `code:0` but with every offset zero and `positionV2: null`. Only 3
/// carries the paragraph ranges the reader needs, so it is the default here.
const AUDIO_TIMEPOINT_PATH: &str = "/reading/reader/audio/timepoint/";

pub(crate) fn handle_timepoint<'a>(
    ctx: &'a Ctx,
    params: &'a Params,
) -> BoxFuture<'a, ApiResult<Value>> {
    Box::pin(async move {
        let book_id = params.get_str("book_id");
        let item_id = default_val(&params.get_str("item_id"), &params.get_str("chapter_id"));
        if book_id.is_empty() || item_id.is_empty() {
            return Err(ApiError::BadRequest(
                "缺少参数: book_id / item_id 均为必填".to_string(),
            ));
        }
        let tone_id = default_val(&params.get_str("tone_id"), "0");
        let req_type = default_val(&params.get_str("req_type"), "3");

        let mut p = oversea_params();
        set_param(&mut p, "book_id", book_id);
        set_param(&mut p, "item_id", item_id);
        set_param(&mut p, "tone_id", tone_id);
        set_param(&mut p, "req_type", req_type);

        Upstream::new(ctx.up.clone())
            .json(&UpstreamRequestSpec {
                mode: UpstreamMode::DeviceSigned,
                host: HOST_FQNOVEL.to_string(),
                path: AUDIO_TIMEPOINT_PATH.to_string(),
                params: p,
                headers: dragon_read_headers(),
                ..Default::default()
            })
            .await
    })
}
