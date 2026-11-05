//! Per-request fqsign `device_json` (device-bound Neptune identity).

/// International app id used for Neptune binding. Must stay 1967 even when a
/// request URL carries a different `aid` (the speech/text exception uses
/// aid=6589 in the query bag). gonidbg binds Neptune to the app's own id, not
/// the per-request query aid; feeding 6589 into Neptune crashes the engine
/// boot (`unmapped access`) and leaves the signer cold.
const NEPTUNE_AID: &str = "1967";

/// Builds the fqsign `device_json` for one request. Besides the persisted
/// `serialno`, it carries the request's own `device_id` / `iid` so the engine
/// performs gonidbg's device-bound Neptune init
/// (`internal/sign/idlefq/engine.go::initializeNeptune`). Without it the
/// signature is not bound to the request identity and strict endpoints
/// (`/novel/player/multi_video_model/v1/`) answer `{"Code":110001}`.
/// The engine reboots whenever this JSON changes (device rotation).
///
/// `aid` and `fetched_did` are fixed to the international app identity
/// (1967 / device_id), matching gonidbg's `factory_engine.go`.
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
                m.insert("device_id".into(), did.clone().into());
                m.insert("install_id".into(), iid.into());
                m.insert("aid".into(), NEPTUNE_AID.into());
                // gonidbg sets FetchedDID = device_id; rustnidbg defaults the
                // same way when empty, but spelling it out keeps the binding
                // explicit across signer versions.
                m.insert("fetched_did".into(), did.into());
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
        assert_eq!(j["fetched_did"], "111");
        assert_eq!(j["serialno"], "0123456789abcdef");
        assert!(device_json_for("https://h/p?x=1", None).is_none());
    }

    #[test]
    fn device_json_keeps_neptune_aid_international_even_when_url_aid_differs() {
        // speech/text exception: URL carries aid=6589, Neptune must stay 1967.
        let u = "https://api-sinfonlinea.fanqiesdk.com/api/novel/audio/speech/text/v1/?aid=6589&device_id=111&iid=222";
        let j: serde_json::Value =
            serde_json::from_str(&device_json_for(u, Some("0123456789abcdef")).unwrap()).unwrap();
        assert_eq!(j["aid"], "1967");
        assert_eq!(j["device_id"], "111");
        assert_eq!(j["install_id"], "222");
        assert_eq!(j["fetched_did"], "111");
    }
}
