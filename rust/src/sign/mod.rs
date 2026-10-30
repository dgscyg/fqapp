//! Upstream request signing — the emulated libmetasec library ONLY.
//!
//! The previous author's pure-algorithm implementations (argus/ladon/gorgon/
//! simon/sm3) were removed: they signed the DOMESTIC endpoints this app no
//! longer talks to, and the shipped obfuscated `libfqsign` artifact is the
//! single source of signatures. `abogus` stays — it signs the
//! fanqienovel.com WEB fallback (a different, web anti-crawler mechanism),
//! not the device API.

pub mod abogus;
pub mod emu;

use crate::timeutil;

/// The single signing path: every request goes through the emulated
/// libmetasec artifact. A boot/sign failure surfaces as an empty header set
/// (the request then fails upstream) — the artifact error is logged by the
/// emu layer with its `fqsign_last_error` detail.
#[derive(Debug, Clone, Default)]
pub struct Manager {}

impl Manager {
    pub fn new() -> Self {
        Manager {}
    }

    /// Signs one request through the emulated library. `full_url` is the
    /// request target including the query; `request_headers` are the headers
    /// the request will carry (the emulated SDK reads the ones it needs,
    /// e.g. user-agent and cookie, from this set); `post_data` is hashed
    /// into x-ss-stub for POSTs.
    pub fn generate_headers_for_request(
        &self,
        full_url: &str,
        request_headers: &[(String, String)],
        post_data: Option<&[u8]>,
        device_serial: Option<&str>,
    ) -> Vec<(String, String)> {
        let now_ms = timeutil::now_millis();
        emu::sign_request(full_url, request_headers, post_data, now_ms, device_serial)
            .unwrap_or_default()
    }
}
