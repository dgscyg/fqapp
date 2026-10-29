//! Network side of device registration.

use md5::{Digest, Md5};
use serde_json::{json, Value};

use crate::crypto::gzip;
use crate::device::crypto::*;
use crate::device::Device;
use crate::sign::Manager;
use crate::upstream::http::{HttpTransport, API_BODY_LIMIT_BYTES};

const PKG_NAME: &str = "com.dragon.read";

/// Go's `url.Values.Encode()`: sorted keys, query-component escaping with '+'
/// for spaces and the unreserved set `A-Za-z0-9-_.~`.
pub fn go_query_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn sorted_query(params: &[(String, String)]) -> String {
    let mut v = params.to_vec();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v.iter()
        .map(|(k, val)| format!("{}={}", go_query_escape(k), go_query_escape(val)))
        .collect::<Vec<_>>()
        .join("&")
}

#[derive(Debug, Clone)]
pub struct DevInfo {
    /// 16-hex serial; persisted with the device (fqsign device_json input).
    pub serialno: String,
    pub device_type: String,
    pub os_api: String,
    pub os_version: String,
    pub openudid: String,
    pub resolution: String,
    pub dpi: String,
    pub cdid: String,
    pub uuid: String,
    pub clientudid: String,
    pub rom: String,
    pub rom_version: String,
    pub device_brand: String,
    pub channel: String,
    pub version_code: String,
    pub version_name: String,
    pub manifest_version_code: String,
    pub update_version_code: String,
    pub user_agent: String,
    // Register-payload identity fields (reference: device.Info).
    pub device_manufacturer: String,
    pub cpu_abi: String,
    pub release_build: String,
    pub display_density: String,
    pub sig_hash: String,
    pub ipv6_address: String,
    pub req_id: String,
    pub apk_first_install_ms: i64,
    pub host_abi: String,
    /// Register cookie (store-region; install_id filled after register).
    pub cookie: String,
}

pub fn new_dev_info() -> DevInfo {
    // Random handset surface from the reference's proven pools (models.rs).
    let s = crate::device::models::random_surface();
    let install_id = random_hex_lower(16);
    DevInfo {
        serialno: crate::device::models::random_serial(),
        device_type: s.model,
        os_api: s.os_api.to_string(),
        os_version: s.os_version,
        openudid: s.openudid,
        resolution: s.resolution,
        dpi: s.dpi,
        cdid: uuid_v4(),
        uuid: uuid_v4(),
        clientudid: uuid_v4(),
        rom: s.rom,
        rom_version: s.rom_version,
        device_brand: s.brand,
        channel: "googleplay".into(),
        version_code: "68132".into(),
        version_name: "6.8.1.32".to_string(),
        manifest_version_code: "68132".into(),
        update_version_code: "68132".into(),
        user_agent: s.user_agent,
        device_manufacturer: s.manufacturer,
        cpu_abi: s.cpu_abi,
        release_build: s.release_build,
        display_density: s.display_density,
        sig_hash: s.sig_hash,
        ipv6_address: s.ipv6_address,
        req_id: s.req_id,
        apk_first_install_ms: s.apk_first_install_ms,
        host_abi: s.host_abi,
        cookie: format!("store-region=cn-zj; store-region-src=did; install_id={install_id};"),
    }
}

impl DevInfo {
    pub fn url_params(&self, extra: &[(String, String)]) -> Vec<(String, String)> {
        let now = crate::timeutil::now_secs().to_string();
        let mut p: Vec<(String, String)> = vec![
            ("tt_data".into(), "a".into()),
            ("os_api".into(), self.os_api.clone()),
            ("device_type".into(), self.device_type.clone()),
            ("ssmix".into(), "a".into()),
            (
                "manifest_version_code".into(),
                self.manifest_version_code.clone(),
            ),
            ("dpi".into(), self.dpi.clone()),
            ("version_name".into(), self.version_name.clone()),
            ("ts".into(), now),
            ("cpu_support64".into(), "true".into()),
            ("app_type".into(), "normal".into()),
            ("appTheme".into(), "light".into()),
            ("ac".into(), "wifi".into()),
            ("host_abi".into(), "arm64-v8a".into()),
            (
                "update_version_code".into(),
                self.update_version_code.clone(),
            ),
            ("channel".into(), self.channel.clone()),
            ("_rticket".into(), crate::timeutil::now_millis().to_string()),
            ("device_platform".into(), "android".into()),
            ("version_code".into(), self.version_code.clone()),
            ("cdid".into(), self.cdid.clone()),
            ("os".into(), "android".into()),
            ("is_android_pad".into(), "0".into()),
            ("openudid".into(), self.openudid.clone()),
            ("package".into(), PKG_NAME.into()),
            ("resolution".into(), self.resolution.clone()),
            ("os_version".into(), self.os_version.clone()),
            ("language".into(), "zh".into()),
            ("device_brand".into(), self.device_brand.clone()),
            ("need_personal_recommend".into(), "1".into()),
            ("aid".into(), "1967".into()),
            ("minor_status".into(), "0".into()),
            ("app_name".into(), "novel".into()),
            ("mcc_mnc".into(), "46007".into()),
        ];
        for (k, v) in extra {
            p.retain(|(pk, _)| pk != k);
            p.push((k.clone(), v.clone()));
        }
        p
    }
}

pub struct Registrar {
    http: HttpTransport,
    sign: Manager,
    /// Offline test seam, see `UpstreamClient::with_mock_origin`.
    mock_origin: Option<String>,
}

impl Registrar {
    pub fn new() -> Result<Self, String> {
        Self::build(None)
    }

    pub fn with_mock_origin(origin: impl Into<String>) -> Result<Self, String> {
        Self::build(Some(origin.into()))
    }

    fn build(mock_origin: Option<String>) -> Result<Self, String> {
        let mut sign = Manager::new();
        // registerKey signs with x-gorgon enabled.
        sign.enable_gorgon = true;
        Ok(Registrar {
            http: HttpTransport::new(30)?,
            sign,
            mock_origin,
        })
    }

    fn target(&self, url: &str) -> String {
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

    /// Full registration: device_register -> app_alert_check -> privilege
    /// activation -> registerkey.
    pub async fn register(&self) -> Result<Device, String> {
        let di = new_dev_info();

        let (device_id, install_id) = self
            .device_register(&di)
            .await
            .map_err(|e| format!("device_register: {e}"))?;

        let _ = self.app_alert_check(&di, &device_id, &install_id).await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let _ = self
            .activate_premium(&device_id, &install_id, &di.device_type)
            .await;

        let secret_key = match self.register_key(&device_id, &install_id).await {
            Ok(k) => k,
            Err(e) => {
                // The device is still returned, but with an empty key it can
                // decrypt nothing. The reason must reach the log: this used to
                // go to a no-op `log_line`, which made the degradation
                // invisible in `rust.log` and `logcat` alike.
                log::warn!("registerKey failed (secret_key will be empty): {e}");
                String::new()
            }
        };

        let now = crate::timeutil::now_string();
        Ok(Device {
            device_id,
            install_id,
            secret_key,
            platform: "android".into(),
            status: "active".into(),
            created_time: now.clone(),
            last_used: now,
            use_count: 0,
            cdid: di.cdid,
            openudid: di.openudid,
            serialno: di.serialno,
        })
    }

    async fn device_register(&self, di: &DevInfo) -> Result<(String, String), String> {
        // Ported 1:1 from the proven fqnovel-gonidbg reference: the
        // fqnovel.com register host (log.snssdk.com gets TLS-reset on some
        // networks), a minimal query, a PLAIN JSON body (no tt_encrypt) and
        // the reference's header set.
        let gtime = crate::timeutil::now_millis();
        let header = json!({
            "display_name": "番茄小说",
            "aid": 1967,
            "channel": "googleplay",
            "package": "com.dragon.read.oversea.gp",
            "app_version": di.version_name,
            "version_code": 68132,
            "update_version_code": 68132,
            "manifest_version_code": 68132,
            "app_version_minor": di.version_name,
            "sdk_version": "3.7.0-rc.25-fanqie-xiaoshuo-opt",
            "sdk_target_version": 29,
            "git_hash": "5b6a0d3",
            "sdk_flavor": "china",
            "guest_mode": 0,
            "is_system_app": 0,
            "pre_installed_channel": "",
            "not_request_sender": 0,
            "os": "Android",
            "os_version": di.os_version,
            "os_api": di.os_api,
            "device_model": di.device_type,
            "device_brand": di.device_brand,
            "device_manufacturer": di.device_manufacturer,
            "cpu_abi": di.cpu_abi,
            "release_build": di.release_build,
            "density_dpi": di.dpi,
            "display_density": di.display_density,
            "resolution": di.resolution.replace('*', "x"),
            "language": "zh",
            "timezone": 8,
            "access": "wifi",
            "rom": di.rom,
            "rom_version": di.rom_version.replace('+', " "),
            "cdid": di.cdid,
            "sig_hash": di.sig_hash,
            "openudid": di.openudid,
            "clientudid": di.clientudid,
            "ipv6_list": [
                {"type": "client_anpi", "value": di.ipv6_address}
            ],
            "region": "CN",
            "tz_name": "Asia/Shanghai",
            "tz_offset": 28800,
            "sim_serial_number": [],
            "oaid_may_support": false,
            "req_id": di.req_id,
            "device_platform": "android",
            "custom": {
                "host_bit": 64,
                "account_region": "cn",
                "dragon_device_type": "phone",
            },
            "apk_first_install_time": di.apk_first_install_ms,
        });
        let body = json!({
            "magic_tag": "ss_app_log",
            "header": header,
            "_gen_time": gtime,
        });
        let json_data = serde_json::to_vec(&body).map_err(|e| e.to_string())?;

        let req_url = format!(
            "https://log5-applog.fqnovel.com/service/2/device_register/?\
aid=1967&version_code=68132&channel=googleplay\
&package=com.dragon.read.oversea.gp&_rticket={gtime}\
&use_store_region_cookie=1&okhttp_version=4.2.137.76-fanqie"
        );

        let headers = vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("accept".to_string(), "application/json".to_string()),
            ("accept-encoding".to_string(), "gzip".to_string()),
            ("log-encode-type".to_string(), "gzip".to_string()),
            ("x-ss-req-ticket".to_string(), gtime.to_string()),
            (
                "x-vc-bdturing-sdk-version".to_string(),
                "3.7.2.cn".to_string(),
            ),
            ("user-agent".to_string(), di.user_agent.clone()),
            ("cookie".to_string(), di.cookie.clone()),
        ];

        let resp = self
            .http
            .request_with_limit(
                "POST",
                &self.target(&req_url),
                &headers,
                Some(json_data),
                None,
                Some(API_BODY_LIMIT_BYTES),
            )
            .await
            .map_err(|e| format!("{e:?}"))?;
        if resp.status != 200 {
            return Err(format!("HTTP {}", resp.status));
        }
        let raw = maybe_gunzip(&resp.body, &resp.headers);
        let rd: Value = serde_json::from_slice(&raw).map_err(|e| format!("json: {e}"))?;
        let device_id = pick_str(&rd, &["device_id_str", "device_id"]);
        let install_id = pick_str(&rd, &["install_id_str", "install_id"]);
        if device_id.is_empty() || install_id.is_empty() {
            return Err(format!("bad response: {}", body_head(&raw)));
        }
        Ok((device_id, install_id))
    }

    async fn app_alert_check(
        &self,
        di: &DevInfo,
        device_id: &str,
        install_id: &str,
    ) -> Result<(), String> {
        let params = di.url_params(&[
            ("device_id".to_string(), device_id.to_string()),
            ("iid".to_string(), install_id.to_string()),
        ]);
        let query = sorted_query(&params);
        let req_url = format!("https://ichannel.snssdk.com/service/2/app_alert_check/?{query}");

        let mut headers = vec![
            (
                "content-type".to_string(),
                "application/octet-stream;tt-data=a".to_string(),
            ),
            ("accept-encoding".to_string(), "gzip".to_string()),
            ("log-encode-type".to_string(), "gzip".to_string()),
            ("x-tt-request-tag".to_string(), "t=0;n=1".to_string()),
            (
                "x-ss-req-ticket".to_string(),
                crate::timeutil::now_millis().to_string(),
            ),
            ("sdk-version".to_string(), "2".to_string()),
            ("passport-sdk-version".to_string(), di.version_code.clone()),
            (
                "x-vc-bdturing-sdk-version".to_string(),
                "3.7.4.cn".to_string(),
            ),
            ("user-agent".to_string(), di.user_agent.clone()),
            ("host".to_string(), "ichannel.snssdk.com".to_string()),
            ("connection".to_string(), "Keep-Alive".to_string()),
        ];
        for (k, v) in self.sign.generate_headers(&query, None) {
            headers.retain(|(hk, _)| !hk.eq_ignore_ascii_case(&k));
            headers.push((k, v));
        }

        let _ = self
            .http
            .request_with_limit(
                "GET",
                &self.target(&req_url),
                &headers,
                None,
                None,
                Some(API_BODY_LIMIT_BYTES),
            )
            .await;
        Ok(())
    }

    async fn activate_premium(
        &self,
        device_id: &str,
        install_id: &str,
        device_type: &str,
    ) -> Result<(), String> {
        let mut parts = vec![
            "aid=1967".to_string(),
            "app_name=novelapp".to_string(),
            "channel=0".to_string(),
            format!("device_id={device_id}"),
            "device_platform=android".to_string(),
            format!("device_type={device_type}"),
            format!("iid={install_id}"),
            "os_version=0".to_string(),
            "version_code=61532".to_string(),
            "version_name=6.1.5.32".to_string(),
            "manifest_version_code=61532".to_string(),
            "update_version_code=61532".to_string(),
        ];
        parts.sort();
        let req_url = format!(
            "https://api5-normal-sinfonlinea.fqnovel.com/reading/user/privilege/add/v/?{}",
            parts.join("&")
        );

        let body = json!({
            "add_count_daily": 0,
            "amount": 2592000,
            "privilege_id": 7210376203117531962i64,
            "from": 8,
            "unique_key": crate::timeutil::now_millis().to_string(),
        });
        let body_json = serde_json::to_vec(&body).map_err(|e| e.to_string())?;

        let _ = self
            .http
            .request_with_limit(
                "POST",
                &self.target(&req_url),
                &[],
                Some(body_json),
                None,
                Some(API_BODY_LIMIT_BYTES),
            )
            .await;
        Ok(())
    }

    async fn register_key(&self, device_id: &str, install_id: &str) -> Result<String, String> {
        let content = encrypt_register_key_request(device_id, None)?;

        let mut parts = vec![
            "aid=1967".to_string(),
            "app_name=novelapp".to_string(),
            "channel=googleplay".to_string(),
            format!("device_id={device_id}"),
            "device_platform=android".to_string(),
            format!("iid={install_id}"),
            "os_version=0".to_string(),
            "version_code=68132".to_string(),
            "version_name=6.8.1.32".to_string(),
        ];
        parts.sort();
        let param_string = parts.join("&");
        let req_url = format!(
            "https://api5-normal-sinfonlineb.fqnovel.com/reading/crypt/registerkey?{param_string}"
        );

        let post_json =
            serde_json::to_vec(&json!({ "content": content })).map_err(|e| e.to_string())?;
        let compressed = gzip(&post_json);
        let sig_headers = self.sign.generate_headers(&param_string, Some(&compressed));

        let mut last_err = String::new();
        for _attempt in 0..3 {
            let mut headers = vec![
                ("User-Agent".to_string(), "com.dragon.read".to_string()),
                ("Content-Encoding".to_string(), "gzip".to_string()),
            ];
            for (k, v) in &sig_headers {
                headers.retain(|(hk, _)| !hk.eq_ignore_ascii_case(k));
                headers.push((k.clone(), v.clone()));
            }

            let resp = match self
                .http
                .request_with_limit(
                    "POST",
                    &self.target(&req_url),
                    &headers,
                    Some(compressed.clone()),
                    None,
                    Some(API_BODY_LIMIT_BYTES),
                )
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    last_err = format!("{e:?}");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };
            if resp.status != 200 {
                last_err = format!("HTTP {} body={}", resp.status, body_head(&resp.body));
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }

            let rd: Value = match serde_json::from_slice(&resp.body) {
                Ok(v) => v,
                Err(e) => {
                    last_err = format!("json: {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };
            let key = rd
                .get("data")
                .and_then(|d| d.get("key"))
                .and_then(|k| k.as_str())
                .unwrap_or("")
                .to_string();
            if key.is_empty() {
                last_err = "missing key field".to_string();
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
            match decrypt_register_key(&key) {
                Ok(dec) if !dec.is_empty() => return Ok(hex::encode_upper(dec)),
                Ok(_) => {
                    last_err = "decrypt key: empty".to_string();
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
                Err(e) => {
                    last_err = format!("decrypt key: {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            }
        }
        Err(last_err)
    }
}

pub fn md5_hex(s: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(s.as_bytes());
    hex::encode(hasher.finalize())
}

fn pick_str(v: &Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(val) = v.get(*k) {
            if let Some(s) = val.as_str() {
                if !s.is_empty() {
                    return s.to_string();
                }
                continue;
            }
            if let Some(s) = crate::json::value_to_string(val) {
                if !s.is_empty() {
                    return s;
                }
            }
        }
    }
    String::new()
}

/// A diagnostic prefix of a response body.
///
/// Bodies here can be up to `API_BODY_LIMIT_BYTES` (64 MiB) and the strings
/// built from them end up in `rust.log`, which rotates at 1 MiB - interpolating
/// a whole body would evict the log's history in one line. 256 bytes is enough
/// to recognise an error envelope.
/// Gunzip when the response says so (or the body carries the gzip magic),
/// mirroring the reference's decodeMaybeGzip.
fn maybe_gunzip(raw: &[u8], headers: &[(String, String)]) -> Vec<u8> {
    let says_gzip = headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("content-encoding") && v.to_lowercase().contains("gzip")
    });
    let magic = raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b;
    if says_gzip || magic {
        crate::crypto::gunzip(raw).unwrap_or_else(|| raw.to_vec())
    } else {
        raw.to_vec()
    }
}

fn body_head(bytes: &[u8]) -> String {
    let head = &bytes[..bytes.len().min(256)];
    String::from_utf8_lossy(head).into_owned()
}
