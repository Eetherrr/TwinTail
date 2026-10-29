//! Validated startup configuration. Credentials are never included in diagnostics.
use serde::Deserialize;
use serde_json::Value;
use twintail_plugin_sdk::{ErrorCode, Result, RpcError};
use url::Url;

/// Universal WebSocket transport mode.
#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransportMode {
    /// Connect to a remote OneBot server (legacy default).
    #[default]
    ForwardWs,
    /// Accept a single authenticated Universal connection.
    ReverseWs,
}

/// Universal WebSocket configuration. All changes require instance restart.
///
/// `access_token` is an already resolved host secret reference. This type deliberately
/// does not implement Debug or Serialize; never log raw configuration snapshots.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Transport direction; omitted values preserve forward mode.
    #[serde(default)]
    pub transport: TransportMode,
    /// Numeric listener address, default 127.0.0.1:17281.
    #[serde(default = "reverse_bind")]
    pub reverse_bind: String,
    /// Exact upgrade path without query/fragment, default /onebot/v11.
    #[serde(default = "reverse_path")]
    pub reverse_path: String,
    /// ws/wss endpoint without credentials, query string or fragment.
    #[serde(default)]
    pub url: String,
    /// Canonical decimal account ID; events from other accounts are rejected.
    pub self_id: String,
    /// Optional Bearer credential, injected into Authorization only.
    #[serde(default)]
    pub access_token: Option<String>,
    /// Deadline from action admission through remote reply, milliseconds.
    #[serde(default = "action_timeout")]
    pub action_timeout_ms: u64,
    /// Connection/TLS/HTTP handshake and individual write deadline, milliseconds.
    #[serde(default = "connect_timeout")]
    pub connect_timeout_ms: u64,
    /// Initial reconnect backoff, milliseconds.
    #[serde(default = "reconnect_initial")]
    pub reconnect_initial_ms: u64,
    /// Maximum exponential backoff, milliseconds.
    #[serde(default = "reconnect_max")]
    pub reconnect_max_ms: u64,
    /// WebSocket ping interval, independent of OneBot heartbeat, milliseconds.
    #[serde(default = "ping_interval")]
    pub ping_interval_ms: u64,
    /// Deadline for the matching WebSocket pong, milliseconds.
    #[serde(default = "pong_timeout")]
    pub pong_timeout_ms: u64,
    /// Total queued and in-flight actions; admission fails immediately when full.
    #[serde(default = "max_pending")]
    pub max_pending: usize,
    /// Best-effort publication queue capacity, independent of action replies.
    #[serde(default = "event_queue")]
    pub event_queue: usize,
    /// Maximum incoming/outgoing WebSocket message size in bytes.
    #[serde(default = "max_frame")]
    pub max_frame_bytes: usize,
}
fn reverse_bind() -> String {
    "127.0.0.1:17281".into()
}
fn reverse_path() -> String {
    "/onebot/v11".into()
}
fn action_timeout() -> u64 {
    3000
}
fn connect_timeout() -> u64 {
    3000
}
fn reconnect_initial() -> u64 {
    100
}
fn reconnect_max() -> u64 {
    10000
}
fn ping_interval() -> u64 {
    15000
}
fn pong_timeout() -> u64 {
    10000
}
fn max_pending() -> usize {
    64
}
fn event_queue() -> usize {
    64
}
fn max_frame() -> usize {
    1048576
}
impl Config {
    /// Parses and validates without returning raw input, endpoints or token values.
    pub fn from_value(value: Value) -> Result<Self> {
        let config: Self = serde_json::from_value(value).map_err(|_| invalid())?;
        config.validate()?;
        Ok(config)
    }
    /// Validates bounds and rejects URL authentication or query/fragment credentials.
    pub fn validate(&self) -> Result {
        let transport_valid = match self.transport {
            TransportMode::ForwardWs => Url::parse(&self.url).is_ok_and(|url| {
                matches!(url.scheme(), "ws" | "wss")
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && self.url.len() <= 4096
                    && !self
                        .url
                        .chars()
                        .any(|c| c.is_whitespace() || c.is_control())
            }),
            TransportMode::ReverseWs => {
                self.url.is_empty()
                    && self
                        .reverse_bind
                        .parse::<std::net::SocketAddr>()
                        .is_ok_and(|address| address.ip().is_loopback())
                    && self.reverse_path.starts_with('/')
                    && self.reverse_path.len() <= 1024
                    && !self
                        .reverse_path
                        .bytes()
                        .any(|b| b <= 0x20 || b >= 0x7f || matches!(b, b'?' | b'#'))
            }
        };
        if !transport_valid
            || self.self_id.is_empty()
            || self.self_id.len() > 128
            || !self.self_id.bytes().all(|b| b.is_ascii_digit())
            || (self.self_id.len() > 1 && self.self_id.starts_with('0'))
            || self.access_token.as_ref().is_some_and(|token| {
                token.is_empty()
                    || token.len() > 4096
                    || token.bytes().any(|b| !(0x21..=0x7e).contains(&b))
            })
            || !(1..=1024).contains(&self.max_pending)
            || !(1..=4096).contains(&self.event_queue)
            || !(1024..=4 * 1024 * 1024).contains(&self.max_frame_bytes)
            || !(10..=30000).contains(&self.action_timeout_ms)
            || !(10..=10000).contains(&self.connect_timeout_ms)
            || !(10..=60000).contains(&self.reconnect_initial_ms)
            || !(self.reconnect_initial_ms..=60000).contains(&self.reconnect_max_ms)
            || !(10..=60000).contains(&self.ping_interval_ms)
            || !(10..=60000).contains(&self.pong_timeout_ms)
        {
            return Err(invalid());
        }
        Ok(())
    }
}
fn invalid() -> RpcError {
    RpcError::new(
        ErrorCode::InvalidConfig,
        "invalid Universal WebSocket configuration",
    )
}
