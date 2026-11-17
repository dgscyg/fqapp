//! VVC-only fallback lists: `fallback_api_url` must ask for H.265
//! (`codec_type=1`) instead of bytevc2 (`codec_type=4`).

use super::*;
use serde_json::json;

fn model(fallback: &str) -> Map<String, Value> {
    json!({ "fallback_api": { "fallback_api": fallback } })
        .as_object()
        .unwrap()
        .clone()
}

#[test]
fn fallback_rewrites_vvc_codec_to_h265() {
    let url = fallback_api_url(&model(
        "https://vod.example/play?vid=v1&codec_type=4&format_type=mp4&stream_type=audio_encrypt",
    ));
    assert_eq!(
        url,
        "https://vod.example/play?vid=v1&codec_type=1&format_type=mp4"
    );
}

#[test]
fn fallback_rewrites_codec_when_it_is_the_last_or_first_param() {
    assert_eq!(
        fallback_api_url(&model("https://vod.example/play?vid=v1&codec_type=4")),
        "https://vod.example/play?vid=v1&codec_type=1"
    );
    assert_eq!(
        fallback_api_url(&model("https://vod.example/play?codec_type=4&vid=v1")),
        "https://vod.example/play?codec_type=1&vid=v1"
    );
}

#[test]
fn fallback_leaves_other_codecs_and_lookalikes_alone() {
    for raw in [
        "https://vod.example/play?vid=v1&codec_type=1",
        "https://vod.example/play?vid=v1&codec_type=0",
        "https://vod.example/play?vid=v1&codec_type=40",
        "https://vod.example/play?vid=v1&xcodec_type=4",
        "https://vod.example/play?vid=v1",
    ] {
        assert_eq!(fallback_api_url(&model(raw)), raw);
    }
}

#[test]
fn fallback_rewrite_also_applies_to_the_string_form() {
    let mut m = Map::new();
    m.insert(
        "fallback_api".into(),
        Value::String(r#"{"fallback_api":"https://vod.example/p?codec_type=4&vid=v2"}"#.into()),
    );
    assert_eq!(
        fallback_api_url(&m),
        "https://vod.example/p?codec_type=1&vid=v2"
    );
    assert_eq!(fallback_api_url(&Map::new()), "");
}

/// `codec=vvc` (libmpv proxy client): the fallback keeps `codec_type=4`.
#[tokio::test]
async fn vvc_mode_keeps_vvc_codec_type() {
    let url = "https://vod.example/play?vid=v1&codec_type=4";
    let kept = ALLOW_VVC
        .scope(true, async { fallback_api_url(&model(url)) })
        .await;
    assert_eq!(kept, url);
    let rewritten = ALLOW_VVC
        .scope(false, async { fallback_api_url(&model(url)) })
        .await;
    assert_eq!(rewritten, "https://vod.example/play?vid=v1&codec_type=1");
}

/// bytevc2 variants are rejected unless VVC mode is on; then they win ties.
#[tokio::test]
async fn vvc_mode_selects_bytevc2_variants() {
    let vvc = json!({"vwidth": 1080, "vheight": 1920, "bitrate": 900_000, "codec_type": "bytevc2"});
    let hevc = json!({"vwidth": 1080, "vheight": 1920, "bitrate": 2_400_000, "codec_type": "h265"});
    let smaller =
        json!({"vwidth": 720, "vheight": 1280, "bitrate": 900_000, "codec_type": "bytevc2"});
    let score = |v: &Value| video_quality_score(v.as_object().unwrap());
    assert_eq!(score(&vvc), None);
    let (vvc_score, hevc_score, smaller_score) = ALLOW_VVC
        .scope(true, async { (score(&vvc), score(&hevc), score(&smaller)) })
        .await;
    assert!(vvc_score.unwrap() > hevc_score.unwrap());
    assert!(hevc_score.unwrap() > smaller_score.unwrap());
}

/// Rows whose scores tie (oversea list, size only in video_meta) still list
/// the highest tier first; 1080P must not end up last.
#[test]
fn merged_variants_are_ordered_by_size() {
    let row = |name: &str, w: i64, h: i64| json!({"name": name, "width": w, "height": h});
    let rows = merge_variant_rows(vec![
        (5, row("720P", 720, 1280)),
        (5, row("360P", 360, 640)),
        (1, row("1080P", 1080, 1920)),
        (5, row("480P", 480, 854)),
    ]);
    let names: Vec<_> = rows.iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["1080P", "720P", "480P", "360P"]);
}

/// The client routes only bytevc2 streams to the libmpv VVC player; every
/// other codec keeps the normal player, so each row carries its codec.
#[test]
fn variants_and_source_carry_their_codec() {
    let label = |v: Value| variant_codec_label(v.as_object().unwrap());
    assert_eq!(label(json!({"codec_type": "bytevc2"})), "vvc");
    assert_eq!(
        label(json!({"video_meta": {"codec_type": "ByteVC2"}})),
        "vvc"
    );
    assert_eq!(
        label(json!({"video_meta": {"definition": "bytevc2"}})),
        "vvc"
    );
    assert_eq!(label(json!({"codec_type": "h265"})), "other");
    assert_eq!(
        label(json!({"video_meta": {"definition": "h264"}})),
        "other"
    );
    assert_eq!(label(json!({})), "other");

    let variants = json!([
        {"url": "https://cdn.invalid/vvc.mp4", "codec": "vvc"},
        {"url": "https://cdn.invalid/hevc.mp4", "codec": "other"},
    ]);
    assert_eq!(
        source_codec("https://cdn.invalid/vvc.mp4", &variants),
        "vvc"
    );
    assert_eq!(
        source_codec("https://cdn.invalid/hevc.mp4", &variants),
        "other"
    );
    assert_eq!(
        source_codec("https://cdn.invalid/missing.mp4", &variants),
        "other"
    );
    assert_eq!(
        source_codec("https://cdn.invalid/vvc.mp4", &json!([])),
        "other"
    );
}

/// The main URL comes from the `fallback_api` answer and never equals a
/// variant row's signed URL; the entry it was resolved from decides (1.0.217
/// labelled every VVC main stream `other`).
#[test]
fn main_stream_codec_comes_from_the_resolved_entry() {
    let variants = json!([
        {"url": "https://cdn.invalid/a.mp4?sig=1", "codec": "vvc"},
        {"url": "https://cdn.invalid/hevc.mp4", "codec": "other"},
    ]);
    assert_eq!(
        stream_codec("vvc", "https://cdn.invalid/a.mp4?sig=2", &variants),
        "vvc"
    );
    assert_eq!(
        stream_codec("other", "https://cdn.invalid/a.mp4?sig=2", &variants),
        "other"
    );
    assert_eq!(
        stream_codec("other", "https://cdn.invalid/a.mp4?sig=1", &variants),
        "vvc"
    );
}

/// select_video_source labels the entry it picked (VVC mode).
#[tokio::test]
async fn resolved_source_carries_its_codec() {
    let list = [
        json!({"vwidth": 1080, "vheight": 1920, "bitrate": 900_000, "codec_type": "bytevc2", "main_url": "https://cdn.invalid/v.mp4"}),
        json!({"vwidth": 720, "vheight": 1280, "bitrate": 900_000, "codec_type": "h265", "main_url": "https://cdn.invalid/h.mp4"}),
    ];
    let pick = || {
        select_video_source(list.iter(), |info| {
            Some((info["main_url"].as_str()?.to_string(), vec![1]))
        })
    };
    let VideoSourceResolution::Found(url, _, codec) = ALLOW_VVC.scope(true, async { pick() }).await
    else {
        panic!("expected a source");
    };
    assert_eq!((url.as_str(), codec), ("https://cdn.invalid/v.mp4", "vvc"));
    let VideoSourceResolution::Found(url, _, codec) = pick() else {
        panic!("expected a source");
    };
    assert_eq!(
        (url.as_str(), codec),
        ("https://cdn.invalid/h.mp4", "other")
    );
}
