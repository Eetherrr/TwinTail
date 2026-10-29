//! Lossless OneBot v11 boundary types, independent of transports and the host.
//!
//! [`Frame`] distinguishes events from action replies by their structure, never by
//! the presence of `echo`. [`normalize`] maps events onto the TwinTail publish
//! contract while leaving the original JSON payload intact. Account identifiers
//! are read as decimal strings without an intermediate floating-point conversion.
//! CQ strings are opaque text; this crate does not parse or generate CQ codes.
//!
//! This library performs no I/O, retries, authentication or account registration.
//! Transports must bound input size, generate opaque string echoes and correlate
//! responses with their own bounded pending table. A timed-out or disconnected
//! action may have executed and must not be automatically retried.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{error, fmt};
use twintail_protocol::{BotRef, PublishEvent};

/// Protocol namespace used by normalized publications.
pub const PROTOCOL: &str = "onebot.v11";

/// A malformed OneBot document or mismatched account binding.
///
/// Messages identify only the invalid field; raw events, action parameters and
/// credentials are deliberately excluded from diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(&'static str);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl error::Error for Error {}

/// Standard OneBot event families, with unknown families preserved for forwarding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// Incoming private or group message.
    Message,
    /// Membership, recall and other notifications.
    Notice,
    /// Friend or group request.
    Request,
    /// Lifecycle and heartbeat events.
    MetaEvent,
    /// Extension family identified by the event's original `post_type`.
    Unknown,
}

/// A validated event together with its untouched JSON representation.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    raw: Value,
    self_id: String,
    post_type: String,
    kind: EventKind,
}
impl Event {
    /// Parses a JSON event with exact integer precision.
    ///
    /// Rejects nonobjects, missing/invalid `self_id` and invalid `post_type`.
    /// Unknown event families and all extension fields are retained. The caller
    /// must enforce its transport's frame-size bound before parsing.
    pub fn parse(text: &str) -> Result<Self, Error> {
        Self::from_value(serde_json::from_str(text).map_err(|_| Error("invalid JSON"))?)
    }
    /// Validates an existing JSON value without removing or rewriting fields.
    ///
    /// Use [`Self::parse`] at the transport boundary. Precision already lost by
    /// another JSON parser cannot be restored here.
    pub fn from_value(raw: Value) -> Result<Self, Error> {
        let object = raw.as_object().ok_or(Error("event must be an object"))?;
        let self_id = account_id(object.get("self_id").ok_or(Error("missing self_id"))?)?;
        let post_type = object
            .get("post_type")
            .and_then(Value::as_str)
            .ok_or(Error("missing or invalid post_type"))?;
        if !valid_name(post_type) {
            return Err(Error("invalid post_type"));
        }
        let kind = match post_type {
            "message" => EventKind::Message,
            "notice" => EventKind::Notice,
            "request" => EventKind::Request,
            "meta_event" => EventKind::MetaEvent,
            _ => EventKind::Unknown,
        };
        if kind == EventKind::Message
            && let Some(value) = object.get("message")
        {
            Message::from_value(value.clone())?;
        }
        let post_type = post_type.to_owned();
        Ok(Self {
            raw,
            self_id,
            post_type,
            kind,
        })
    }
    /// Returns the normalized decimal account identifier.
    pub fn self_id(&self) -> &str {
        &self.self_id
    }
    /// Returns the event family; extensions map to [`EventKind::Unknown`].
    pub fn kind(&self) -> EventKind {
        self.kind
    }
    /// Returns the original family string, including unknown extensions.
    pub fn post_type(&self) -> &str {
        &self.post_type
    }
    /// Returns the original payload, retaining every known and unknown field.
    pub fn raw(&self) -> &Value {
        &self.raw
    }
    /// Returns a normalized identifier field such as `user_id` or `group_id`.
    ///
    /// Missing fields return `Ok(None)`; present malformed identifiers return an error.
    /// The original payload remains unchanged.
    pub fn account(&self, field: &str) -> Result<Option<String>, Error> {
        self.raw.get(field).map(account_id).transpose()
    }
    /// Returns message content for a message event, leaving CQ text opaque.
    pub fn message(&self) -> Option<Message> {
        (self.kind == EventKind::Message)
            .then(|| self.raw.get("message").cloned())
            .flatten()
            .and_then(|v| Message::from_value(v).ok())
    }
}

/// Converts a nonnegative decimal integer number or string to canonical text.
///
/// Rejects signs, exponent/fraction syntax, empty values and identifiers over 128
/// digits. Leading zeros in strings are removed; zero is retained as `"0"`.
/// Arbitrarily large integer JSON literals remain exact with this crate's
/// `serde_json/arbitrary_precision` feature. Never pass through `f64` first.
pub fn account_id(value: &Value) -> Result<String, Error> {
    let digits = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Err(Error("invalid account identifier")),
    };
    if digits.is_empty() || digits.len() > 128 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error("invalid account identifier"));
    }
    let canonical = digits.trim_start_matches('0');
    Ok(if canonical.is_empty() {
        "0".into()
    } else {
        canonical.into()
    })
}

/// Maps an event onto the host publication contract for a registered bot.
///
/// The account must match `bot.self_id` after decimal canonicalization and the
/// host-assigned adapter instance must be nonempty. No host event ID is minted
/// here: the router stamps IDs and receive times when it accepts publication.
/// Exact subscriptions use `message`, `notice`, `request`, `meta_event`, or
/// the unknown family's original name. Payload JSON is preserved.
pub fn normalize(event: &Event, bot: &BotRef) -> Result<PublishEvent, Error> {
    if bot.adapter_instance_id.is_empty()
        || account_id(&Value::String(bot.self_id.clone()))? != event.self_id
    {
        return Err(Error("event account does not match registered bot"));
    }
    Ok(PublishEvent {
        self_id: bot.self_id.clone(),
        protocol: PROTOCOL.into(),
        event_type: event.post_type.clone(),
        payload: event.raw.clone(),
    })
}

/// OneBot message content: either an array of segments or untouched CQ text.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Message {
    /// Original CQ-compatible string; no parsing, escaping or joining is performed.
    Text(String),
    /// Ordered message segments, including unknown segment kinds and fields.
    Segments(Vec<MessageSegment>),
}
impl Message {
    /// Validates message content without converting strings into segments.
    pub fn from_value(value: Value) -> Result<Self, Error> {
        match value {
            Value::String(text) => Ok(Self::Text(text)),
            Value::Array(values) => values
                .into_iter()
                .map(|value| {
                    serde_json::from_value(value).map_err(|_| Error("invalid message segment"))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Self::Segments),
            _ => Err(Error("invalid message content")),
        }
    }
}

// Serde's untagged intermediate representation cannot hold arbitrary-precision
// numbers. Decode a Value first, then inspect the actual JSON shape instead.
impl<'de> Deserialize<'de> for Message {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_value(Value::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// An extensible OneBot message segment.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MessageSegment {
    /// Segment type, including unknown implementation-specific types.
    #[serde(rename = "type")]
    pub kind: String,
    /// Original segment data with arbitrary nested extension values.
    pub data: Map<String, Value>,
    /// Additional fields beside `type` and `data`.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// An outbound OneBot action with a transport-generated opaque string echo.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ActionRequest {
    /// Standard or extension action name.
    pub action: String,
    /// Parameters, preserving all JSON extension fields.
    pub params: Value,
    /// Unique opaque correlation value; never a numeric counter on the wire.
    pub echo: String,
}
impl ActionRequest {
    /// Builds an action request; parameters must be an object and echo nonempty.
    ///
    /// Action names and echoes are bounded to 96 and 256 bytes respectively.
    /// No retry policy is attached; transports must preserve uncertain outcomes.
    pub fn new(
        action: impl Into<String>,
        params: Value,
        echo: impl Into<String>,
    ) -> Result<Self, Error> {
        let action = action.into();
        let echo = echo.into();
        if !valid_name(&action) {
            return Err(Error("invalid action name"));
        }
        if !params.is_object() {
            return Err(Error("action params must be an object"));
        }
        if echo.is_empty() || echo.len() > 256 || echo.chars().any(char::is_control) {
            return Err(Error("invalid action echo"));
        }
        Ok(Self {
            action,
            params,
            echo,
        })
    }
}

/// An action response retaining remote errors and all implementation extensions.
///
/// Nonzero `retcode` and non-`ok` statuses are valid responses, not parse errors.
/// The transport should return their complete serialized value to the caller;
/// this crate does not discard `data` or translate remote failure into success.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ActionResponse {
    /// Remote status (`ok`, `failed`, `async`, or an extension).
    pub status: String,
    /// Original signed OneBot return code.
    pub retcode: i64,
    /// Original response data, including structured data on failure.
    pub data: Value,
    /// Original correlation value; only string echoes correlate our outbound calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub echo: Option<Value>,
    /// Additional response fields such as `message` or `wording`.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
impl ActionResponse {
    /// Parses a response with exact JSON precision and validates its structure.
    pub fn parse(text: &str) -> Result<Self, Error> {
        Self::from_value(serde_json::from_str(text).map_err(|_| Error("invalid JSON"))?)
    }
    /// Parses a response value, rejecting event/response ambiguity.
    pub fn from_value(value: Value) -> Result<Self, Error> {
        if value.get("post_type").is_some() {
            return Err(Error("event cannot be parsed as action response"));
        }
        let object = value
            .as_object()
            .ok_or(Error("action response must be an object"))?;
        if !object.contains_key("data")
            || object
                .get("status")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        {
            return Err(Error("missing or invalid action response field"));
        }
        let Value::Object(mut object) = value else {
            unreachable!()
        };
        let status = object
            .remove("status")
            .and_then(|v| v.as_str().map(str::to_owned))
            .ok_or(Error("invalid response status"))?;
        let retcode = object
            .remove("retcode")
            .and_then(|v| v.as_i64())
            .ok_or(Error("invalid response retcode"))?;
        let data = object
            .remove("data")
            .ok_or(Error("missing response data"))?;
        let echo = object.remove("echo");
        Ok(Self {
            status,
            retcode,
            data,
            echo,
            extra: object,
        })
    }
    /// Returns the opaque string echo if supplied with the expected type.
    pub fn echo_str(&self) -> Option<&str> {
        self.echo.as_ref().and_then(Value::as_str)
    }
    /// Reports success only for `status == "ok"` together with `retcode == 0`.
    pub fn is_success(&self) -> bool {
        self.status == "ok" && self.retcode == 0
    }
}

/// A structural classification of an incoming universal WebSocket JSON frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Frame {
    /// An event, including unknown event families and events containing `echo`.
    Event(Event),
    /// An action response identified by its status/retcode/data fields.
    Response(ActionResponse),
}
impl Frame {
    /// Parses an inbound frame. Presence of `post_type` selects event parsing;
    /// otherwise a complete action-response structure is required.
    ///
    /// An event with an extension field called `echo` remains an event. Malformed
    /// documents are rejected; transports should record bounded diagnostics.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let value: Value = serde_json::from_str(text).map_err(|_| Error("invalid JSON"))?;
        Self::from_value(value)
    }
    /// Classifies an existing value without inspecting echo for dispatch.
    pub fn from_value(value: Value) -> Result<Self, Error> {
        if value.get("post_type").is_some() {
            Event::from_value(value).map(Self::Event)
        } else {
            ActionResponse::from_value(value).map(Self::Response)
        }
    }
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 96
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-/".contains(&b))
}

// Decode extension-bearing objects directly as Value rather than Serde's
// flatten intermediate representation, which cannot hold integers over u64.
impl<'de> Deserialize<'de> for MessageSegment {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let Value::Object(mut object) = Value::deserialize(deserializer)? else {
            return Err(serde::de::Error::custom("segment must be an object"));
        };
        let kind = object
            .remove("type")
            .and_then(|v| v.as_str().map(str::to_owned))
            .ok_or_else(|| serde::de::Error::custom("invalid segment type"))?;
        let data = match object.remove("data") {
            Some(Value::Object(data)) => data,
            _ => return Err(serde::de::Error::custom("invalid segment data")),
        };
        Ok(Self {
            kind,
            data,
            extra: object,
        })
    }
}
impl<'de> Deserialize<'de> for ActionResponse {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_value(Value::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
