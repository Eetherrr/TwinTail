//! Versioned JSON wire types and bounded Tokio framing for TwinTail IPC.
//!
//! Each frame is a four-byte big-endian length followed by one JSON [`Message`].
//! stdout is reserved for these frames; diagnostics belong on stderr. Protocol
//! version negotiation and request authorization are responsibilities of peers.
//!
//! ```
//! use serde_json::json;
//! use twintail_protocol::{Message, MAX_FRAME_BYTES, read_message, write_message};
//!
//! # tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
//! let message = Message::request("r1", "example.echo", json!({"text": "hello"}));
//! let mut wire = Vec::new();
//! write_message(&mut wire, &message, MAX_FRAME_BYTES).await?;
//! let received = read_message(&mut wire.as_slice(), MAX_FRAME_BYTES).await?;
//! assert_eq!(received, message);
//! # Ok::<(), std::io::Error>(())
//! # }).unwrap();
//! ```

use std::{fmt, io};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Absolute maximum JSON payload length, excluding the four-byte prefix (8 MiB).
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Protocol version emitted by message constructors and [`ProtocolVersion::default`].
pub const VERSION: ProtocolVersion = ProtocolVersion { major: 1, minor: 2 };

/// Wire protocol version; peers must negotiate compatibility before activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProtocolVersion {
    /// Major version; differing major versions are incompatible.
    pub major: u16,
    /// Minor version; additions must preserve the major version's compatibility.
    pub minor: u16,
}

impl Default for ProtocolVersion {
    fn default() -> Self {
        VERSION
    }
}

/// A versioned message whose body fields appear at the envelope's top level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Message {
    /// Sender's protocol version, to be checked by the receiving peer.
    pub protocol_version: ProtocolVersion,
    /// Message category and its category-specific fields.
    #[serde(flatten)]
    pub body: Body,
}

impl Message {
    /// Creates a control message using [`VERSION`].
    pub fn control(method: impl Into<String>, payload: Value) -> Self {
        Self {
            protocol_version: VERSION,
            body: Body::Control {
                method: method.into(),
                payload,
            },
        }
    }

    /// Creates a request using [`VERSION`]; callers ensure connection-local ID uniqueness.
    pub fn request(id: impl Into<String>, method: impl Into<String>, payload: Value) -> Self {
        Self {
            protocol_version: VERSION,
            body: Body::Request {
                request_id: id.into(),
                method: method.into(),
                payload,
            },
        }
    }

    /// Creates a response using [`VERSION`] and the originating request's ID.
    pub fn response(id: impl Into<String>, outcome: Outcome) -> Self {
        Self {
            protocol_version: VERSION,
            body: Body::Response {
                request_id: id.into(),
                outcome,
            },
        }
    }
}

/// The four IPC message categories, tagged by the top-level `kind` field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    /// An invocation requiring a response with the same request ID.
    Request {
        /// Connection-local correlation ID.
        request_id: String,
        /// Invoked method name.
        method: String,
        /// Method-specific JSON arguments.
        payload: Value,
    },
    /// Completion of a previous request.
    Response {
        /// Correlation ID copied from the request.
        request_id: String,
        /// Exactly one success value or structured error.
        outcome: Outcome,
    },
    /// A notification that does not require a response.
    Event {
        /// Event method name.
        method: String,
        /// Event-specific JSON data.
        payload: Value,
    },
    /// A lifecycle or connection-control notification.
    Control {
        /// Control method name.
        method: String,
        /// Control-specific JSON data.
        payload: Value,
    },
}

/// A response result, encoded as `{"status":"ok"|"err","value":...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum Outcome {
    /// Successful method result.
    Ok(Value),
    /// Structured failure.
    Err(RpcError),
}

/// A stable error category with a human-readable diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RpcError {
    /// Machine-readable error category.
    pub code: ErrorCode,
    /// Diagnostic text; producers must avoid exposing secrets.
    pub message: String,
}

impl RpcError {
    /// Creates an error with a category and a diagnostic message.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for RpcError {}

/// Stable, snake-case wire error categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Peers have incompatible protocol versions.
    IncompatibleVersion,
    /// The caller lacks an explicitly granted capability.
    PermissionDenied,
    /// The requested target is offline.
    Offline,
    /// The operation's deadline elapsed; side effects may already have occurred.
    Timeout,
    /// A forwarded operation may have executed before its result was lost; never retry automatically.
    UnknownOutcome,
    /// The operation was cancelled.
    Cancelled,
    /// A bounded queue or concurrency budget is exhausted.
    Overloaded,
    /// Configuration validation failed.
    InvalidConfig,
    /// The serving plugin process exited.
    PluginExited,
    /// A plugin replacement failed.
    ReloadFailed,
    /// An upstream service returned an error.
    RemoteError,
    /// The transport frame is invalid.
    InvalidFrame,
    /// The request is malformed or semantically invalid.
    InvalidRequest,
    /// A required dependency is unavailable or failed.
    DependencyFailed,
    /// The requested resource does not exist.
    NotFound,
    /// The operation conflicts with the current state or revision.
    Conflict,
    /// The requested operation or feature is unsupported.
    Unsupported,
}

/// Initial plugin identity and requested capabilities; identity is not authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Hello {
    /// Stable plugin identifier, checked against the host's process binding.
    pub plugin_id: String,
    /// Plugin package version.
    pub version: String,
    /// Capabilities required for activation.
    pub required_capabilities: Vec<String>,
}

/// Host-assigned instance context delivered after negotiation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Initialize {
    /// Host-assigned instance identifier.
    pub instance_id: String,
    /// Host-assigned process generation.
    pub generation: u64,
    /// Capabilities explicitly granted by the host.
    pub granted_capabilities: Vec<String>,
    /// Plugin-private data directory supplied by the host.
    pub data_dir: String,
    /// Configuration revision selected for initialization.
    pub config_revision: u64,
    /// Optional applied configuration snapshot; absent on older hosts/fixtures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<ConfigSnapshot>,
}

/// Reads one length-prefixed JSON message from `reader`.
///
/// `max_frame` is the caller's payload limit, additionally capped at
/// [`MAX_FRAME_BYTES`]. The prefix is validated before allocating the payload.
/// Unknown optional JSON fields are accepted; protocol versions are not negotiated here.
///
/// # Errors
/// Returns [`io::ErrorKind::InvalidData`] for zero/oversized lengths or invalid
/// JSON/messages, [`io::ErrorKind::UnexpectedEof`] for a missing or truncated
/// prefix/payload, and propagates underlying I/O errors.
///
/// # Cancellation
/// Cancellation can consume part of a frame. Discard the connection after
/// cancelling this operation unless the transport is otherwise resynchronized.
pub async fn read_message<R: AsyncRead + Unpin>(
    reader: &mut R,
    max_frame: usize,
) -> io::Result<Message> {
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix).await?;
    let length = u32::from_be_bytes(prefix) as usize;
    validate_length(length, max_frame, io::ErrorKind::InvalidData)?;
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload).await?;
    serde_json::from_slice(&payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Serializes and writes one length-prefixed JSON message to `writer`.
///
/// `max_frame` is capped at [`MAX_FRAME_BYTES`]. Serialization and payload-length
/// validation finish before any bytes are written. This function does not flush;
/// callers control flushing when using buffered writers.
///
/// # Errors
/// Returns [`io::ErrorKind::InvalidInput`] if serialization fails or the encoded
/// payload exceeds either limit; underlying write errors are propagated.
///
/// # Cancellation
/// Cancellation or an I/O error can leave a partial frame on the connection.
/// Discard that connection rather than attempting another frame.
pub async fn write_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: &Message,
    max_frame: usize,
) -> io::Result<()> {
    let payload = serde_json::to_vec(message)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    validate_length(payload.len(), max_frame, io::ErrorKind::InvalidInput)?;
    writer
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await?;
    writer.write_all(&payload).await
}

fn validate_length(length: usize, max_frame: usize, kind: io::ErrorKind) -> io::Result<()> {
    let limit = max_frame.min(MAX_FRAME_BYTES);
    if length == 0 || length > limit {
        return Err(io::Error::new(
            kind,
            format!("frame length {length} is outside 1..={limit}"),
        ));
    }
    Ok(())
}

mod routing;
pub use routing::*;

mod configuration;
pub use configuration::*;

mod management;
pub use management::*;
