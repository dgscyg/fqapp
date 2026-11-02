//! Per-request fqsign `device_json` (device-bound Neptune identity).

/// Builds the fqsign `device_json` for one request. Besides the persisted
/// `serialno`, it carries the request's own `device_id` / `iid` / `aid`
/// so the engine performs gonidbg's device-bound Neptune init
/// (`internal/sign/idlefq/engine.go::initializeNeptune`). Without it the
/// signature is not bound to the request identity and strict endpoints
/// (`/novel/player/multi_video_model/v1/`) answer `{"Code":110001}`.
/// The engine reboots whenever this JSON changes (device rotation).
pub(crate) fn device_json_for(full_url: &str, device_serial: Option<&str>) -> Option<String> {
    let mut m = serde_json::Map::new();
    if let Some(s) = device_serial.filter(|s| !s.is_empty()) {
        m.insert("serialno".into(), s.into());
    }
    if let Ok(u) = url::Url::parse(full_url) {
        let q = |k: &str| {
            u.query_pairs()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.into_owned())
        };
        if let (Some(did), Some(iid)) = (q("device_id"), q("iid")) {
            if !did.is_empty() && !iid.is_empty() && !did.starts_with('{') {
                m.insert("device_id".into(), did.into());
                m.insert("install_id".into(), iid.into());
                m.insert(
                    "aid".into(),
                    q("aid").unwrap_or_else(|| "1967".into()).into(),
                );
            }
        }
    }
    (!m.is_empty()).then(|| serde_json::Value::Object(m).to_string())
}

#[cfg(test)]
mod device_json_tests {
    use super::device_json_for;

    #[test]
    fn device_json_binds_request_identity() {
        let u = "https://h/p?aid=1967&device_id=111&iid=222&x=1";
        let j: serde_json::Value =
            serde_json::from_str(&device_json_for(u, Some("0123456789abcdef")).unwrap()).unwrap();
        assert_eq!(j["device_id"], "111");
        assert_eq!(j["install_id"], "222");
        assert_eq!(j["aid"], "1967");
        assert_eq!(j["serialno"], "0123456789abcdef");
        assert!(device_json_for("https://h/p?x=1", None).is_none());
    }
}
