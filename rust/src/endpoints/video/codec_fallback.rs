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
