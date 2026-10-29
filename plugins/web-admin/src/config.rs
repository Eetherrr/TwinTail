//! Startup-only configuration; resolved administrator credentials never implement Debug.
use serde::Deserialize;
use serde_json::Value;
use std::net::SocketAddr;
use twintail_plugin_sdk::{ErrorCode, Result, RpcError};
/// Loopback HTTP settings. Every configuration change requires exclusive restart.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Numeric loopback socket, default 127.0.0.1:17280. Port zero is for tests.
    #[serde(default = "bind")]
    pub bind: SocketAddr,
    /// Host-resolved administrator secret; 32–256 printable nonspace ASCII bytes.
    /// Supply a randomly generated token using a host environment secret reference.
    pub admin_token: String,
    /// Fixed session lifetime (not sliding), default one hour.
    #[serde(default = "ttl")]
    pub session_ttl_secs: u64,
    /// Maximum concurrent sessions. Full capacity rejects new logins.
    #[serde(default = "sessions")]
    pub max_sessions: usize,
    /// Maximum executing HTTP requests; saturation is rejected immediately.
    #[serde(default = "requests")]
    pub max_concurrent_requests: usize,
    /// Maximum live TCP connections, including slow header readers.
    #[serde(default = "connections")]
    pub max_connections: usize,
    /// Total handler/body/backend deadline in milliseconds.
    #[serde(default = "deadline")]
    pub request_timeout_ms: u64,
    /// JSON body hard limit in bytes, default 64 KiB.
    #[serde(default = "body")]
    pub max_body_bytes: usize,
}
fn bind() -> SocketAddr {
    "127.0.0.1:17280".parse().unwrap()
}
fn ttl() -> u64 {
    3600
}
fn sessions() -> usize {
    32
}
fn requests() -> usize {
    16
}
fn connections() -> usize {
    64
}
fn deadline() -> u64 {
    10000
}
fn body() -> usize {
    65536
}
impl Config {
    /// Parses without reflecting values, field contents or secret data in errors.
    pub fn from_value(value: Value) -> Result<Self> {
        let value: Self = serde_json::from_value(value).map_err(|_| invalid())?;
        value.validate()?;
        Ok(value)
    }
    /// Requires loopback, bounded resources and a nontrivial 32-byte credential.
    /// Length/character checks cannot prove entropy: generate the token randomly.
    pub fn validate(&self) -> Result {
        if !self.bind.ip().is_loopback()
            || !(32..=256).contains(&self.admin_token.len())
            || !self.admin_token.bytes().all(|b| (0x21..=0x7e).contains(&b))
            || self
                .admin_token
                .bytes()
                .all(|b| b == self.admin_token.as_bytes()[0])
            || !(60..=86400).contains(&self.session_ttl_secs)
            || !(1..=256).contains(&self.max_sessions)
            || !(1..=128).contains(&self.max_concurrent_requests)
            || !(1..=512).contains(&self.max_connections)
            || !(100..=30000).contains(&self.request_timeout_ms)
            || !(1024..=1048576).contains(&self.max_body_bytes)
        {
            return Err(invalid());
        }
        Ok(())
    }
}
fn invalid() -> RpcError {
    RpcError::new(
        ErrorCode::InvalidConfig,
        "invalid web administrator configuration",
    )
}
