//! Upstream request construction shared by all endpoints.

use std::sync::Arc;

use serde_json::Value;

use crate::device::Device;
use crate::error::{ApiError, ApiResult};
use crate::upstream::{ClientError, PinnedDevice, UpstreamClient, UpstreamRequest};

pub const HOST_FQNOVEL: &str = "https://api5-normal-sinfonlineb.fqnovel.com";
pub const HOST_FQNOVELA: &str = "https://api5-normal-sinfonlinea.fqnovel.com";
pub const HOST_FQNOVELB: &str = "https://api5-normal-sinfonlineb.fqnovel.com";
/// Go fqclient `DefaultSearchBaseURL`: the drama player family
/// (multi_video_model / video_detail) is signed-POSTed here.
pub const HOST_FQNOVELC: &str = "https://api5-normal-sinfonlinec.fqnovel.com";
pub const HOST_NOVEL_SNSSDK: &str = "https://novel.snssdk.com";
pub const HOST_READING_SNSSDK: &str = "https://api5-normal-sinfonlineb.fqnovel.com";
pub const HOST_FANQIENOVEL: &str = "https://fanqienovel.com";
pub const UA_DRAGON: &str = "com.dragon.read";
/// 国际版番茄小说 UA（Go device generator 同款形态）：包名/版本/机型/构建
/// 号齐全。裸 `com.dragon.read` 只有阅读端点容忍，播放器等火山服务会拒。
pub const UA_OVERSEA: &str = "com.dragon.read.oversea.gp/68132 (Linux; U; Android 13; zh_CN; ANA-AN00; Build/TKQ1.221114.001;tt-ok/3.12.13.4-tiktok)";
pub const UA_ANDROID_BROWSER: &str = "Mozilla/5.0 (Linux; Android 13)";
pub const UA_WINDOWS_BROWSER: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36";
pub const CONTENT_TYPE_JSON: &str = "application/json; charset=utf-8";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamMode {
    DeviceSigned,
    Signed,
    NoSign,
}

#[derive(Debug, Clone)]
pub struct UpstreamRequestSpec {
    pub mode: UpstreamMode,
    pub method: Option<String>,
    pub raw_url: Option<String>,
    pub host: String,
    pub path: String,
    pub params: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub headers: Vec<(String, String)>,
    pub pin_device: String,
}

impl Default for UpstreamRequestSpec {
    fn default() -> Self {
        UpstreamRequestSpec {
            mode: UpstreamMode::Signed,
            method: None,
            raw_url: None,
            host: String::new(),
            path: String::new(),
            params: Vec::new(),
            body: None,
            headers: Vec::new(),
            pin_device: String::new(),
        }
    }
}

pub struct Upstream {
    up: Arc<UpstreamClient>,
}

impl Upstream {
    pub fn new(up: Arc<UpstreamClient>) -> Self {
        Upstream { up }
    }

    fn build(spec: &UpstreamRequestSpec) -> UpstreamRequest {
        let method = match &spec.method {
            Some(m) if !m.is_empty() => Some(m.clone()),
            _ => {
                if spec.body.is_some() {
                    Some("POST".to_string())
                } else {
                    Some("GET".to_string())
                }
            }
        };
        let url = match &spec.raw_url {
            Some(u) if !u.is_empty() => u.clone(),
            _ => crate::endpoints::util::build_upstream_url(&spec.host, &spec.path, &spec.params),
        };
        UpstreamRequest {
            method,
            url,
            body: spec.body.clone(),
            headers: spec.headers.clone(),
            no_sign: spec.mode == UpstreamMode::NoSign,
            pin: if spec.pin_device.is_empty() {
                None
            } else {
                Some(PinnedDevice {
                    device_id: spec.pin_device.clone(),
                })
            },
        }
    }

    pub async fn raw_with_device(
        &self,
        spec: &UpstreamRequestSpec,
    ) -> ApiResult<(Vec<u8>, Device)> {
        let req = Self::build(spec);
        self.up
            .do_with_device_capped(&req, crate::upstream::http::API_BODY_LIMIT_BYTES)
            .await
            .map_err(map_client_error)
    }

    pub async fn raw_with_headers(
        &self,
        spec: &UpstreamRequestSpec,
    ) -> ApiResult<(Vec<u8>, Vec<(String, String)>)> {
        let req = Self::build(spec);
        self.up
            .do_request_capped(&req, crate::upstream::http::API_BODY_LIMIT_BYTES)
            .await
            .map_err(map_client_error)
    }

    pub async fn raw(&self, spec: &UpstreamRequestSpec) -> ApiResult<Vec<u8>> {
        match spec.mode {
            UpstreamMode::DeviceSigned => {
                let (raw, _) = self.raw_with_device(spec).await?;
                Ok(raw)
            }
            UpstreamMode::Signed | UpstreamMode::NoSign => {
                let (raw, _) = self.raw_with_headers(spec).await?;
                Ok(raw)
            }
        }
    }

    pub async fn json(&self, spec: &UpstreamRequestSpec) -> ApiResult<Value> {
        let raw = self.raw(spec).await?;
        crate::endpoints::util::raw_json(&raw)
    }
}

pub fn map_client_error(err: ClientError) -> ApiError {
    match err {
        ClientError::DeviceFailed(m) => ApiError::Internal(format!("device failed: {m}")),
        ClientError::Http(m) => ApiError::Internal(m),
        ClientError::Other(m) => ApiError::Internal(m),
    }
}

/// Go `fqclient.BuildCommonHeaders` 的对齐实现：国际版 UA、store-region
/// cookie（install_id 由 Upstream 在拿到设备后替换）、accept-encoding、
/// x-reading-request 等。**所有**上游请求都应带这套头——此前只发
/// UA+Content-Type，阅读端点能容忍，播放器等火山服务直接 110001。
pub fn common_headers() -> Vec<(String, String)> {
    let now = crate::timeutil::now_millis();
    vec![
        ("User-Agent".to_string(), UA_OVERSEA.to_string()),
        (
            "cookie".to_string(),
            "store-region=cn-zj; store-region-src=did; install_id={install_id};".to_string(),
        ),
        (
            "accept".to_string(),
            "application/json; charset=utf-8,application/x-protobuf".to_string(),
        ),
        ("accept-encoding".to_string(), "gzip".to_string()),
        ("x-xs-from-web".to_string(), "0".to_string()),
        (
            "x-vc-bdturing-sdk-version".to_string(),
            "3.7.2.cn".to_string(),
        ),
        ("lc".to_string(), "101".to_string()),
        ("sdk-version".to_string(), "2".to_string()),
        ("passport-sdk-version".to_string(), "50564".to_string()),
        ("x-tt-store-region".to_string(), "cn-zj".to_string()),
        ("x-tt-store-region-src".to_string(), "did".to_string()),
        // Go: fmt.Sprintf("%d-%d", now, now%2000000000)
        (
            "x-reading-request".to_string(),
            format!("{now}-{}", now % 2_000_000_000),
        ),
    ]
}

/// The common set plus a JSON content type (signed POSTs).
pub fn common_json_headers() -> Vec<(String, String)> {
    let mut headers = common_headers();
    headers.push(("Content-Type".to_string(), CONTENT_TYPE_JSON.to_string()));
    headers
}

pub fn dragon_read_headers() -> Vec<(String, String)> {
    common_headers()
}

pub fn dragon_read_json_headers() -> Vec<(String, String)> {
    common_json_headers()
}

pub fn phoenix_player_headers() -> Vec<(String, String)> {
    vec![
        (
            "User-Agent".to_string(),
            "com.phoenix.read/71332 (Linux; U; Android 16; zh_CN; 25053RT47C; Build/BP2A.250605.031.A3; Cronet/TTNetVersion:04657795 2026-01-23 QuicVersion:c67e9834 2025-09-08)".to_string(),
        ),
        (
            "Accept".to_string(),
            "application/json; charset=utf-8,application/x-protobuf".to_string(),
        ),
        ("Content-Type".to_string(), "application/json".to_string()),
        ("x-xs-from-web".to_string(), "0".to_string()),
        ("sdk-version".to_string(), "2".to_string()),
    ]
}

/// Oversea drama player family base (Go fqclient `SearchBaseURL` +
/// `BuildCommonParams`): the OVERSEA app's own identity — aid=1967
/// novelapp 6.8.1.32 googleplay. The previous phoenix/shortplay URLs were
/// captured from the DOMESTIC app (aid=8662 novelread / api3 host / version
/// 71332): the oversea signature library gets every one of them rejected
/// with 110001 or an empty body, which is why drama playback never worked.
pub fn drama_player_url() -> String {
    format!(
        "{HOST_FQNOVELC}/novel/player/multi_video_model/v1/?iid={{install_id}}\
&device_id={{device_id}}&ac=wifi&channel=googleplay&aid=1967&app_name=novelapp&\
version_code=68132&version_name=6.8.1.32&device_platform=android&os=android&ssmix=a&\
device_type={{device_type}}&device_brand={{device_brand}}&language=zh&\
os_api={{os_api}}&os_version={{os_version}}&manifest_version_code=68132&\
resolution={{resolution}}&dpi={{dpi}}&update_version_code=68132&_rticket={{rticket}}&\
host_abi={{host_abi}}&dragon_device_type=phone&pv_player=68132&compliance_status=0&\
need_personal_recommend=1&player_so_load=1&is_android_pad_screen=0&\
rom_version={{rom_version}}&cdid={{cdid}}"
    )
}

/// Headers for the drama player family: the common set plus
/// `Authorization: Bearer` (the Go reference signs the player endpoints with
/// `withBearer=true`).
pub fn drama_player_headers() -> Vec<(String, String)> {
    let mut headers = common_json_headers();
    headers.push(("Authorization".to_string(), "Bearer".to_string()));
    headers
}

/// The current `_rticket` value (unix millis) for the drama player URL.
pub fn drama_rticket() -> String {
    crate::timeutil::now_millis().to_string()
}

/// The drama TOC (episode list) shares the player base with video_detail.
pub fn drama_video_detail_url() -> String {
    drama_player_url().replacen(
        "/novel/player/multi_video_model/v1/",
        "/novel/player/video_detail/v1/",
        1,
    )
}
