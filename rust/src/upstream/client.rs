//! Signed upstream requests with device-pool rotation.

use std::sync::Arc;
use std::time::Duration;

use crate::device::{Device, DevicePool};
use crate::sign::Manager;
use crate::upstream::http::{HttpTransport, TransportError};

pub const DEFAULT_USER_AGENT: &str = "com.dragon.read/66732 (Linux; U; Android 10; zh_CN; Pixel 4 XL; Build/QD1A.190821.007;tt-ok/3.12.13.4-tiktok)";

/// HTTP codes treated as device-auth failures.
fn is_auth_fail_status(status: u16) -> bool {
    matches!(status, 401 | 403 | 10001 | 10002)
}

#[derive(Debug, Clone)]
pub enum ClientError {
    /// Network failure or auth-class status: triggers device replacement.
    DeviceFailed(String),
    Http(String),
    Other(String),
}

impl ClientError {
    pub fn is_device_failed(&self) -> bool {
        matches!(self, ClientError::DeviceFailed(_))
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::DeviceFailed(m) => write!(f, "device failed: {m}"),
            ClientError::Http(m) => f.write_str(m),
            ClientError::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ClientError {}

/// Reuses a specific pooled device (identified by device_id) instead of drawing
/// the next one from the rotation.
#[derive(Debug, Clone)]
pub struct PinnedDevice {
    pub device_id: String,
}

/// One signed upstream call. `body = None` means GET.
#[derive(Debug, Clone, Default)]
pub struct UpstreamRequest {
    pub method: Option<String>,
    pub url: String,
    pub body: Option<Vec<u8>>,
    pub headers: Vec<(String, String)>,
    pub no_sign: bool,
    pub pin: Option<PinnedDevice>,
}

pub struct UpstreamClient {
    http: HttpTransport,
    sign: Manager,
    pool: Arc<DevicePool>,
    max_retries: usize,
    /// Offline test seam: when set, every absolute upstream URL has its origin
    /// (scheme://host[:port]) replaced by this one. Path and query - and
    /// therefore every signature input - are untouched, so a local mock server
    /// can answer real signed requests without touching the network.
    mock_origin: Option<String>,
}

impl UpstreamClient {
    pub fn new(pool: Arc<DevicePool>, sign: Manager) -> Result<Self, String> {
        Self::build(pool, sign, None)
    }

    /// Builds a client whose upstream calls are redirected to `origin`.
    pub fn with_mock_origin(
        pool: Arc<DevicePool>,
        sign: Manager,
        origin: impl Into<String>,
    ) -> Result<Self, String> {
        Self::build(pool, sign, Some(origin.into()))
    }

    fn build(
        pool: Arc<DevicePool>,
        sign: Manager,
        mock_origin: Option<String>,
    ) -> Result<Self, String> {
        Ok(UpstreamClient {
            http: HttpTransport::new(15)?,
            sign,
            pool,
            max_retries: 3,
            mock_origin,
        })
    }

    /// Applies the mock-origin rewrite, keeping path and query intact.
    /// Shared by the request path and the streaming video download, which
    /// bypasses `do_request` with its own client.
    pub(crate) fn effective_url(&self, url: &str) -> String {
        let Some(origin) = self.mock_origin.as_deref() else {
            return url.to_string();
        };
        match url::Url::parse(url) {
            Ok(parsed) => {
                let origin = origin.trim_end_matches('/');
                let mut out = format!("{origin}{}", parsed.path());
                if let Some(q) = parsed.query() {
                    out.push('?');
                    out.push_str(q);
                }
                out
            }
            Err(_) => url.to_string(),
        }
    }

    /// Performs a request with signature injection but no device retry.
    ///
    /// Uncapped: only the video download path uses this. API callers must
    /// use [UpstreamClient::do_request_capped].
    pub async fn do_request(
        &self,
        req: &UpstreamRequest,
    ) -> Result<(Vec<u8>, Vec<(String, String)>), ClientError> {
        self.do_resp(req, None, None).await
    }

    /// Capped variant of [UpstreamClient::do_request] for JSON/API calls.
    pub async fn do_request_capped(
        &self,
        req: &UpstreamRequest,
        body_limit: usize,
    ) -> Result<(Vec<u8>, Vec<(String, String)>), ClientError> {
        self.do_resp(req, Some(body_limit), None).await
    }

    async fn do_resp(
        &self,
        req: &UpstreamRequest,
        body_limit: Option<usize>,
        device_serial: Option<&str>,
    ) -> Result<(Vec<u8>, Vec<(String, String)>), ClientError> {
        let target = self.effective_url(&req.url);
        url::Url::parse(&target).map_err(|e| ClientError::Other(format!("parse url: {e}")))?;

        let method = match &req.method {
            Some(m) if !m.is_empty() => m.clone(),
            _ => {
                if req.body.is_some() {
                    "POST".to_string()
                } else {
                    "GET".to_string()
                }
            }
        };

        let mut headers: Vec<(String, String)> =
            vec![("User-Agent".to_string(), DEFAULT_USER_AGENT.to_string())];
        for (k, v) in &req.headers {
            headers.retain(|(hk, _)| !hk.eq_ignore_ascii_case(k));
            headers.push((k.clone(), v.clone()));
        }

        // Go `fqclient.jsonSigningHeaders`: every signed POST carries the
        // body digest and a charset-qualified content type. Without
        // `x-ss-stub` the TTNet/Luckycat gateway answers the player family with
        // `{"Code":110001,"Message":"未知异常"}` before any business logic runs
        // (drama playback), which is why it looked like a params mismatch.
        if let Some(body) = req.body.as_deref() {
            if !body.is_empty() {
                set_header(
                    &mut headers,
                    "content-type",
                    "application/json; charset=utf-8",
                );
            }
        }

        if !req.no_sign {
            let sig = self.sign.generate_headers_for_request(
                &target,
                &headers,
                req.body.as_deref(),
                device_serial,
            );
            for (k, v) in sig {
                headers.retain(|(hk, _)| !hk.eq_ignore_ascii_case(&k));
                headers.push((k, v));
            }
        }

        let resp = self
            .http
            .request_with_limit(
                &method,
                &target,
                &headers,
                req.body.clone(),
                None,
                body_limit,
            )
            .await
            .map_err(|e| match e {
                TransportError::Network(m) => ClientError::DeviceFailed(m),
                // A local size limit is not a device fault: retiring the
                // device here would rotate identities over a response that was
                // merely too large.
                TransportError::TooLarge(m) => ClientError::Other(m),
            })?;

        if resp.status != 200 {
            if is_auth_fail_status(resp.status) {
                return Err(ClientError::DeviceFailed(format!("HTTP {}", resp.status)));
            }
            return Err(ClientError::Http(format!("HTTP {}", resp.status)));
        }
        Ok((resp.body, resp.headers))
    }

    /// Fetches a device, substitutes placeholders, signs, and retries with a
    /// fresh device on device-class failures.
    ///
    /// Uncapped: only the video download path uses this. API callers must
    /// use [UpstreamClient::do_with_device_capped].
    pub async fn do_with_device(
        &self,
        req: &UpstreamRequest,
    ) -> Result<(Vec<u8>, Device), ClientError> {
        self.do_with_device_inner(req, None).await
    }

    /// Capped variant of [UpstreamClient::do_with_device] for JSON/API calls.
    pub async fn do_with_device_capped(
        &self,
        req: &UpstreamRequest,
        body_limit: usize,
    ) -> Result<(Vec<u8>, Device), ClientError> {
        self.do_with_device_inner(req, Some(body_limit)).await
    }

    async fn do_with_device_inner(
        &self,
        req: &UpstreamRequest,
        body_limit: Option<usize>,
    ) -> Result<(Vec<u8>, Device), ClientError> {
        let mut last_err: Option<ClientError> = None;
        let mut pin = req.pin.clone();

        for _attempt in 0..self.max_retries {
            let mut next_pin = pin.clone();
            let dev = match &next_pin {
                Some(p) => match self.pool.get_by_id(&p.device_id).await {
                    Ok(d) => d,
                    Err(_) => {
                        // A pin miss (device rotated out) falls back to the
                        // pool rotation on the next attempt.
                        next_pin = None;
                        self.pool
                            .get()
                            .await
                            .map_err(|e| ClientError::Other(format!("get device: {e}")))?
                    }
                },
                None => self
                    .pool
                    .get()
                    .await
                    .map_err(|e| ClientError::Other(format!("get device: {e}")))?,
            };

            let mut attempt = req.clone();
            attempt.url = substitute_device_url(&req.url, &dev);
            // Header values may carry device placeholders (the store-region
            // cookie embeds {install_id}); expand them with the same device
            // as the URL.
            attempt.headers = req
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), substitute_device(v, &dev)))
                .collect();
            // Go `BuildCommonHeaders` 用的就是设备自己的 UA（型号 + Build 号
            // 与 query 里的 device_type/rom_version 一致）。播放器一族会交叉
            // 校验 UA 型号，硬编码 UA 与设备档案不符时直接被拒成 110001。
            set_header(&mut attempt.headers, "user-agent", &dev.user_agent());
            attempt.pin = None;

            match self
                .do_resp(&attempt, body_limit, dev.serialno.as_str().into())
                .await
            {
                Ok((data, _)) => return Ok((data, dev)),
                Err(err) => {
                    last_err = Some(err.clone());
                    if err.is_device_failed() {
                        let _ = self.pool.replace_failed(&dev.device_id).await;
                        pin = None;
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                    return Err(err);
                }
            }
        }
        Err(ClientError::Other(format!(
            "device request failed after {} retries: {}",
            self.max_retries,
            last_err.map(|e| e.to_string()).unwrap_or_default()
        )))
    }
}

/// Substitutes device values into a URL, percent-encoding each injected value.
///
/// The URL is built and escaped *before* the device is known, so placeholders
/// survive as `{rom_version}` etc. Injecting the raw value afterwards skips
/// `encode_query` — and Go escapes at encode time, so a `rom_version` of
/// `V394IR+release-keys` must travel as `V394IR%2Brelease-keys`. Left raw, the
/// server reads `+` as a space, the player family rejects the request with
/// `{"Code":110001}` (the reading endpoints tolerate it).
pub fn substitute_device_url(url: &str, dev: &Device) -> String {
    let escaped = |v: &str| crate::endpoints::util::go_query_escape(v);
    let openudid = if dev.openudid.is_empty() {
        &dev.device_id
    } else {
        &dev.openudid
    };
    url.replace("{device_id}", &escaped(&dev.device_id))
        .replace("{install_id}", &escaped(&dev.install_id))
        .replace("{secret_key}", &escaped(&dev.secret_key))
        .replace("{cdid}", &escaped(&dev.cdid))
        .replace("{openudid}", &escaped(openudid))
        .replace("{rticket}", &crate::timeutil::now_millis().to_string())
        .replace("{device_type}", &escaped(&dev.device_type))
        .replace("{device_brand}", &escaped(&dev.device_brand))
        .replace("{resolution}", &escaped(&dev.resolution))
        .replace("{dpi}", &escaped(&dev.dpi))
        .replace("{os_version}", &escaped(&dev.os_version))
        .replace("{os_api}", &escaped(&dev.os_api))
        .replace("{rom_version}", &escaped(&dev.rom_version))
        .replace("{host_abi}", &escaped(&dev.host_abi))
}

pub fn substitute_device(raw_url: &str, dev: &Device) -> String {
    // Pools written before `openudid` was tracked have an empty value, so fall
    // back to this device's own id rather than emitting an empty query
    // parameter, which upstream would read as a malformed request.
    let openudid = if dev.openudid.is_empty() {
        &dev.device_id
    } else {
        &dev.openudid
    };
    raw_url
        .replace("{device_id}", &dev.device_id)
        .replace("{install_id}", &dev.install_id)
        .replace("{secret_key}", &dev.secret_key)
        .replace("{cdid}", &dev.cdid)
        .replace("{openudid}", openudid)
        .replace("{rticket}", &crate::timeutil::now_millis().to_string())
        // 注册时的机型档案（上游会与注册身份交叉校验，见 device::Device
        // 的字段注释）。空值保持为空字符串——旧设备池没有这些字段时行为
        // 与之前一致。
        .replace("{device_type}", &dev.device_type)
        .replace("{device_brand}", &dev.device_brand)
        .replace("{resolution}", &dev.resolution)
        .replace("{dpi}", &dev.dpi)
        .replace("{os_version}", &dev.os_version)
        .replace("{os_api}", &dev.os_api)
        .replace("{rom_version}", &dev.rom_version)
        .replace("{host_abi}", &dev.host_abi)
}

/// Case-insensitive header upsert (the header list keeps the caller's order).
fn set_header(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    headers.push((name.to_string(), value.to_string()));
}

#[cfg(test)]
mod tests {
    use super::{substitute_device, substitute_device_url};
    use crate::device::Device;

    fn device(openudid: &str) -> Device {
        Device {
            device_type: String::new(),
            device_brand: String::new(),
            resolution: String::new(),
            dpi: String::new(),
            os_version: String::new(),
            os_api: String::new(),
            rom_version: String::new(),
            host_abi: String::new(),
            device_id: "0123456789abcdef".to_string(),
            install_id: "9876543210".to_string(),
            secret_key: "abcd".to_string(),
            platform: "android".to_string(),
            status: "active".to_string(),
            created_time: "2026-01-01 00:00:00".to_string(),
            last_used: "2026-01-01 00:00:00".to_string(),
            use_count: 0,
            cdid: "cdid-from-pool".to_string(),
            openudid: openudid.to_string(),
            serialno: String::new(),
        }
    }

    /// Go escapes at `url.Values.Encode()` time, so a `rom_version` carrying
    /// `+release-keys` travels as `%2B`. Injecting the raw value after encoding
    /// sends `+`, which the server decodes as a space — the player family
    /// answered that with `{"Code":110001}` while the reading endpoints were
    /// tolerant. The same applies to the `*` in `resolution`.
    #[test]
    fn url_substitution_percent_encodes_device_values() {
        let mut dev = device("");
        dev.rom_version = "V433IR+release-keys".to_string();
        dev.resolution = "2400*1080".to_string();
        dev.device_type = "SM-S9210".to_string();
        let url = substitute_device_url(
            "https://example.invalid/?rom_version={rom_version}&resolution={resolution}\
&device_type={device_type}",
            &dev,
        );
        assert_eq!(
            url,
            "https://example.invalid/?rom_version=V433IR%2Brelease-keys\
&resolution=2400%2A1080&device_type=SM-S9210"
        );
    }

    /// The UA has to describe the same device the query parameters describe:
    /// the player service cross-checks the model in the UA against
    /// `device_type`, and a hard-coded UA with another model reads as a
    /// spoofed client.
    #[test]
    fn user_agent_follows_the_device_profile() {
        let mut dev = device("");
        dev.device_type = "PGT-AN10".to_string();
        dev.os_version = "13".to_string();
        dev.rom_version = "V451IR+release-keys".to_string();
        assert_eq!(
            dev.user_agent(),
            "com.dragon.read.oversea.gp/68132 (Linux; U; Android 13; zh_CN; PGT-AN10; \
Build/V451IR;tt-ok/3.12.13.4-tiktok)"
        );
        // Pools written before the model profile existed keep the constant.
        dev.device_type = String::new();
        assert_eq!(dev.user_agent(), crate::endpoints::base::UA_OVERSEA);
    }

    // Every device scoped identifier has to come from the pool entry the
    // request layer picked. A literal left in a URL template would pin every
    // install of the app to the same value, so these tests pin the
    // substitution behaviour rather than any particular identifier format.
    #[test]
    fn substitutes_every_device_scoped_placeholder() {
        let url = substitute_device(
            "https://example.invalid/?iid={install_id}&device_id={device_id}\
&cdid={cdid}&openudid={openudid}&secret_key={secret_key}",
            &device("openudid-from-pool"),
        );
        assert_eq!(
            url,
            "https://example.invalid/?iid=9876543210&device_id=0123456789abcdef\
&cdid=cdid-from-pool&openudid=openudid-from-pool&secret_key=abcd"
        );
        assert!(
            !url.contains('{'),
            "no placeholder may survive substitution"
        );
    }

    #[test]
    fn a_pool_entry_without_openudid_falls_back_to_its_own_device_id() {
        // Pools written before `openudid` was recorded deserialize with an
        // empty value. Substituting that empty string would send upstream a
        // malformed empty parameter, so the device's own id stands in.
        let url = substitute_device("https://example.invalid/?openudid={openudid}", &device(""));
        assert_eq!(url, "https://example.invalid/?openudid=0123456789abcdef");
    }

    #[test]
    fn a_pool_entry_without_openudid_does_not_leak_another_devices_identifier() {
        let legacy_url =
            substitute_device("https://example.invalid/?openudid={openudid}", &device(""));
        let fresh_url = substitute_device(
            "https://example.invalid/?openudid={openudid}",
            &device("openudid-from-pool"),
        );
        assert_ne!(legacy_url, fresh_url);
        assert!(!legacy_url.contains("openudid-from-pool"));
    }
}
