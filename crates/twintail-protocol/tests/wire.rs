//! Wire compatibility, schema drift, and bounded framing contract tests.

use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use schemars::schema_for;
use serde_json::json;
use tokio::io::{AsyncRead, ReadBuf};
use twintail_protocol::{
    ActionCall, Body, BotRef, ConfigCommit, ConfigPrepare, ConfigSnapshot, DeliveryReport,
    ErrorCode, EventEnvelope, Hello, Initialize, MAX_FRAME_BYTES, Message, Outcome,
    ProtocolVersion, PublishEvent, RegisterBot, RpcError, ServiceCall, ServiceRegistration,
    Subscribe, VERSION, read_message, write_message,
};

#[test]
fn version_and_flattened_envelope() {
    assert_eq!(ProtocolVersion::default(), VERSION);
    assert_eq!(VERSION, ProtocolVersion { major: 1, minor: 2 });
    let message = Message::request("r1", "example.call", json!({"x": 1}));
    assert_eq!(
        serde_json::to_value(message).unwrap(),
        json!({
            "protocol_version": {"major": 1, "minor": 2},
            "kind": "request", "request_id": "r1", "method": "example.call", "payload": {"x": 1}
        })
    );
    assert_eq!(
        Message::control("ready", json!({})).protocol_version,
        VERSION
    );
}

#[test]
fn response_roundtrips_and_errors_are_structured() {
    for outcome in [
        Outcome::Ok(json!({"answer": 42})),
        Outcome::Err(RpcError::new(ErrorCode::PermissionDenied, "denied")),
    ] {
        let message = Message::response("r1", outcome);
        let value = serde_json::to_value(&message).unwrap();
        assert_eq!(
            serde_json::from_value::<Message>(value.clone()).unwrap(),
            message
        );
        assert!(value.get("body").is_none());
        assert!(matches!(
            value["outcome"]["status"].as_str(),
            Some("ok" | "err")
        ));
    }
    let error = RpcError::new(ErrorCode::InvalidFrame, "bad frame");
    assert_eq!(error.to_string(), "InvalidFrame: bad frame");
    let _: &dyn std::error::Error = &error;
    assert_eq!(
        serde_json::to_value(error).unwrap()["code"],
        "invalid_frame"
    );
}

#[test]
fn unknown_optional_fields_and_explicit_versions_survive() {
    let value = json!({
        "protocol_version": {"major": 2, "minor": 9}, "kind": "event",
        "method": "future", "payload": null, "optional_future_field": true
    });
    let message: Message = serde_json::from_value(value).unwrap();
    assert_eq!(
        message.protocol_version,
        ProtocolVersion { major: 2, minor: 9 }
    );
    assert!(matches!(message.body, Body::Event { .. }));
}

#[tokio::test]
async fn framing_roundtrip_and_big_endian_prefix() {
    let message = Message::response("request-1", Outcome::Ok(json!([1, 2, 3])));
    let mut bytes = Vec::new();
    write_message(&mut bytes, &message, MAX_FRAME_BYTES)
        .await
        .unwrap();
    assert_eq!(
        u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize,
        bytes.len() - 4
    );
    let mut input = bytes.as_slice();
    assert_eq!(
        read_message(&mut input, MAX_FRAME_BYTES).await.unwrap(),
        message
    );
    assert!(input.is_empty());
}

// Delivering one byte per poll exercises read_exact across short successful reads.
struct ShortReader<'a>(&'a [u8]);
impl AsyncRead for ShortReader<'_> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if !self.0.is_empty() && buf.remaining() > 0 {
            buf.put_slice(&self.0[..1]);
            self.0 = &self.0[1..];
        }
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn short_reads_form_a_complete_frame() {
    let message = Message::control("ready", json!({}));
    let mut bytes = Vec::new();
    write_message(&mut bytes, &message, MAX_FRAME_BYTES)
        .await
        .unwrap();
    assert_eq!(
        read_message(&mut ShortReader(&bytes), MAX_FRAME_BYTES)
            .await
            .unwrap(),
        message
    );
}

#[tokio::test]
async fn invalid_lengths_are_rejected_without_reading_a_payload() {
    for (length, limit) in [
        (0, MAX_FRAME_BYTES),
        (11, 10),
        ((MAX_FRAME_BYTES + 1) as u32, usize::MAX),
        (u32::MAX, usize::MAX),
        (1, 0),
    ] {
        let prefix = length.to_be_bytes();
        let error = read_message(&mut prefix.as_slice(), limit)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}

#[tokio::test]
async fn truncated_prefix_and_payload_return_unexpected_eof() {
    for bytes in [&[][..], &[0][..], &[0, 0, 0][..], &[0, 0, 0, 4, b'{'][..]] {
        assert_eq!(
            read_message(&mut &*bytes, MAX_FRAME_BYTES)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}

#[tokio::test]
async fn malformed_json_or_message_is_invalid_data() {
    for payload in [&b"{"[..], &b"{}"[..], &b"\xff"[..], &b"null"[..]] {
        let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(payload);
        assert_eq!(
            read_message(&mut bytes.as_slice(), MAX_FRAME_BYTES)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
}

#[tokio::test]
async fn write_validates_before_writing_and_accepts_exact_limit() {
    let message = Message::control("ready", json!({}));
    let length = serde_json::to_vec(&message).unwrap().len();
    let mut bytes = Vec::new();
    for limit in [0, length - 1] {
        assert_eq!(
            write_message(&mut bytes, &message, limit)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(bytes.is_empty());
    }
    write_message(&mut bytes, &message, length).await.unwrap();
    assert_eq!(
        read_message(&mut bytes.as_slice(), length).await.unwrap(),
        message
    );
    let huge = Message::control("huge", json!("x".repeat(MAX_FRAME_BYTES)));
    let mut output = Vec::new();
    assert_eq!(
        write_message(&mut output, &huge, usize::MAX)
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(output.is_empty());
}

#[test]
fn committed_schemas_do_not_drift() {
    for (name, schema) in [
        ("protocol-v1.json", schema_for!(Message)),
        ("hello-v1.json", schema_for!(Hello)),
        ("initialize-v1.json", schema_for!(Initialize)),
        ("config-snapshot-v1.json", schema_for!(ConfigSnapshot)),
        ("config-prepare-v1.json", schema_for!(ConfigPrepare)),
        ("config-commit-v1.json", schema_for!(ConfigCommit)),
        ("bot-ref-v1.json", schema_for!(BotRef)),
        ("register-bot-v1.json", schema_for!(RegisterBot)),
        ("subscribe-v1.json", schema_for!(Subscribe)),
        ("publish-event-v1.json", schema_for!(PublishEvent)),
        ("event-envelope-v1.json", schema_for!(EventEnvelope)),
        ("delivery-report-v1.json", schema_for!(DeliveryReport)),
        ("action-call-v1.json", schema_for!(ActionCall)),
        (
            "service-registration-v1.json",
            schema_for!(ServiceRegistration),
        ),
        ("service-call-v1.json", schema_for!(ServiceCall)),
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../schemas")
            .join(name);
        let expected = format!("{}\n", serde_json::to_string_pretty(&schema).unwrap());
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            expected,
            "schema drift: {name}"
        );
    }
}

#[test]
fn p2_routing_preserves_extensions_and_account_precision() {
    let payload = json!({"message":[{"type":"future.segment","data":{"id":"9007199254740993","extension":[1,null,true]}}], "vendor":{"x":1}});
    let event = EventEnvelope {
        event_id: "e:1".into(),
        bot: BotRef {
            adapter_instance_id: "a:1".into(),
            self_id: "9007199254740993".into(),
        },
        received_at_ms: 0,
        protocol: "mock.v1".into(),
        event_type: "message".into(),
        payload: payload.clone(),
    };
    let round: EventEnvelope =
        serde_json::from_value(serde_json::to_value(&event).unwrap()).unwrap();
    assert_eq!(event, round);
    assert_eq!(round.payload, payload);
    assert_eq!(
        serde_json::to_value(RpcError::new(ErrorCode::UnknownOutcome, "result lost")).unwrap()["code"],
        "unknown_outcome"
    );
}

#[test]
fn legacy_initialize_and_configuration_payloads_are_compatible() {
    let legacy = json!({"instance_id":"old-1","generation":1,"granted_capabilities":[],"data_dir":"/tmp/old","config_revision":0});
    let initialize: Initialize = serde_json::from_value(legacy.clone()).unwrap();
    assert!(initialize.config.is_none());
    assert_eq!(serde_json::to_value(&initialize).unwrap(), legacy);
    let snapshot = ConfigSnapshot {
        config_version: 2,
        revision: 9,
        values: json!({"message":"hello","optional":{"future":true}}),
    };
    let prepare = ConfigPrepare {
        expected_revision: 8,
        snapshot: snapshot.clone(),
    };
    assert_eq!(
        serde_json::from_value::<ConfigPrepare>(serde_json::to_value(&prepare).unwrap()).unwrap(),
        prepare
    );
    let initialized = Initialize {
        config: Some(snapshot),
        ..initialize
    };
    assert_eq!(
        serde_json::from_value::<Initialize>(serde_json::to_value(&initialized).unwrap()).unwrap(),
        initialized
    );
    assert_eq!(
        serde_json::from_value::<ConfigCommit>(json!({"revision":9,"future_field":true})).unwrap(),
        ConfigCommit { revision: 9 }
    );
}
