//! Signed upstream HTTP access and device rotation.

pub(crate) mod budget;
pub mod client;
pub mod http;
pub(crate) mod playback_trace;
pub mod shuqi;

pub use client::{ClientError, PinnedDevice, UpstreamClient, UpstreamRequest};
pub use http::{HttpTransport, RawResponse, TransportError};
