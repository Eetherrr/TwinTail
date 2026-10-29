//! Sanitized synthetic OneBot boundary fixtures and contract regression tests.

use serde_json::{Value, json};
use twintail_onebot11::{
    ActionRequest, ActionResponse, Event, EventKind, Frame, Message, PROTOCOL, account_id,
    normalize,
};
use twintail_protocol::BotRef;

const MESSAGE: &str = include_str!("fixtures/message.json");
const NOTICE: &str = include_str!("fixtures/notice.json");
const REQUEST: &str = include_str!("fixtures/request.json");
const META: &str = include_str!("fixtures/meta_event.json");
const RESPONSE: &str = include_str!("fixtures/action_response.json");
fn bot() -> BotRef {
    BotRef {
        adapter_instance_id: "adapter.generation.4".into(),
        self_id: "9007199254740993".into(),
    }
}

#[test]
fn four_event_families_normalize_without_losing_raw_fields() {
    for (text, kind, name) in [
        (MESSAGE, EventKind::Message, "message"),
        (NOTICE, EventKind::Notice, "notice"),
        (REQUEST, EventKind::Request, "request"),
        (META, EventKind::MetaEvent, "meta_event"),
    ] {
        let Frame::Event(event) = Frame::parse(text).unwrap() else {
            panic!("event classified as response");
        };
        assert_eq!(event.kind(), kind);
        assert_eq!(event.post_type(), name);
        assert_eq!(event.self_id(), bot().self_id);
        let publish = normalize(&event, &bot()).unwrap();
        assert_eq!(publish.self_id, bot().self_id);
        assert_eq!(publish.protocol, PROTOCOL);
        assert_eq!(publish.event_type, name);
        let original: Value = serde_json::from_str(text).unwrap();
        assert_eq!(publish.payload, original);
        assert_eq!(
            serde_json::from_str::<Value>(&serde_json::to_string(&publish.payload).unwrap())
                .unwrap(),
            original
        );
    }
}
#[test]
fn decimal_ids_keep_precision_beyond_u64_and_canonicalize_strings() {
    let event = Event::parse(MESSAGE).unwrap();
    assert_eq!(
        event.account("group_id").unwrap().as_deref(),
        Some("184467440737095516170")
    );
    assert_eq!(event.account("user_id").unwrap().as_deref(), Some("20002"));
    assert_eq!(event.account("operator_id").unwrap(), None);
    assert_eq!(
        event.raw()["vendor"]["counter"].to_string(),
        "184467440737095516170"
    );
    for (input, expected) in [
        ("0", "0"),
        ("9007199254740993", "9007199254740993"),
        ("184467440737095516170", "184467440737095516170"),
    ] {
        let value = serde_json::from_str::<Value>(input).unwrap();
        assert_eq!(account_id(&value).unwrap(), expected);
        assert_eq!(account_id(&Value::String(input.into())).unwrap(), expected);
    }
    assert_eq!(account_id(&json!("00000")).unwrap(), "0");
    for value in [
        json!(-1),
        json!(1.5),
        json!(true),
        json!(null),
        json!(""),
        json!(" 1"),
        json!("+1"),
        json!("1e3"),
        json!("1.0"),
        json!("1".repeat(129)),
    ] {
        assert!(account_id(&value).is_err(), "accepted invalid ID: {value}");
    }
    assert!(account_id(&serde_json::from_str::<Value>("1e3").unwrap()).is_err());
}
#[test]
fn segments_and_opaque_cq_text_survive_exactly() {
    let event = Event::parse(MESSAGE).unwrap();
    let Message::Segments(segments) = event.message().unwrap() else {
        panic!();
    };
    assert_eq!(segments[0].extra["vendor_flag"], 1);
    assert_eq!(segments[1].kind, "vendor.widget");
    assert_eq!(
        serde_json::to_value(Message::Segments(segments)).unwrap(),
        event.raw()["message"]
    );
    let cq = "literal &#91; [CQ:at,qq=20002] &amp; [CQ:vendor,data=a&#44;b]";
    let event = Event::from_value(
        json!({"post_type":"message","self_id":9007199254740993_u64,"message":cq}),
    )
    .unwrap();
    assert_eq!(event.message(), Some(Message::Text(cq.into())));
    assert_eq!(normalize(&event, &bot()).unwrap().payload["message"], cq);
}
#[test]
fn extension_events_preserve_family_and_do_not_require_standard_fields() {
    let event = Event::parse(
        r#"{"post_type":"vendor.future","self_id":"9007199254740993","future":{"x":[null,true]}}"#,
    )
    .unwrap();
    assert_eq!(event.kind(), EventKind::Unknown);
    assert_eq!(
        normalize(&event, &bot()).unwrap().event_type,
        "vendor.future"
    );
    assert!(event.message().is_none());
    for text in [
        "null",
        "[]",
        "{}",
        r#"{"post_type":"message","self_id":1.5}"#,
        r#"{"post_type":"","self_id":1}"#,
        r#"{"post_type":"message","self_id":1,"message":true}"#,
    ] {
        assert!(Event::parse(text).is_err());
    }
    assert!(Event::from_value(json!({"post_type":"a".repeat(97),"self_id":1})).is_err());
}
#[test]
fn normalization_rejects_wrong_bot_without_rewriting_source_payload() {
    let event = Event::parse(MESSAGE).unwrap();
    let mut wrong = bot();
    wrong.self_id = "9007199254740992".into();
    assert!(normalize(&event, &wrong).is_err());
    wrong = bot();
    wrong.adapter_instance_id.clear();
    assert!(normalize(&event, &wrong).is_err());
    let mut canonical = bot();
    canonical.self_id.insert_str(0, "000");
    assert_eq!(
        normalize(&event, &canonical).unwrap().self_id,
        canonical.self_id
    );
    assert_eq!(
        event.raw(),
        &serde_json::from_str::<Value>(MESSAGE).unwrap()
    );
}
#[test]
fn action_request_preserves_parameters_and_uses_opaque_string_echo() {
    let params = json!({"user_id":"9007199254740993","message":[{"type":"vendor.x","data":{"x":true}}],"future":{"value":null}});
    let request =
        ActionRequest::new("send_private_msg", params.clone(), "instance/4/request-9").unwrap();
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        json!({"action":"send_private_msg","params":params,"echo":"instance/4/request-9"})
    );
    assert!(ActionRequest::new("", json!({}), "echo").is_err());
    assert!(ActionRequest::new("send_msg", json!(null), "echo").is_err());
    assert!(ActionRequest::new("send_msg", json!({}), "").is_err());
    assert!(ActionRequest::new("send_msg", json!({}), "\n").is_err());
    assert!(ActionRequest::new("send_msg", json!({}), "x".repeat(257)).is_err());
}
#[test]
fn remote_error_response_keeps_retcode_data_and_all_extensions() {
    let Frame::Response(response) = Frame::parse(RESPONSE).unwrap() else {
        panic!();
    };
    assert!(!response.is_success());
    assert_eq!(response.retcode, 1404);
    assert_eq!(response.echo_str(), Some("opaque-generation-4-call-7"));
    assert_eq!(
        response.data["vendor_id"].to_string(),
        "184467440737095516170"
    );
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        serde_json::from_str::<Value>(RESPONSE).unwrap()
    );
    for (status, retcode, success) in [
        ("ok", 0, true),
        ("ok", 1, false),
        ("failed", 0, false),
        ("async", 1, false),
    ] {
        let response =
            ActionResponse::from_value(json!({"status":status,"retcode":retcode,"data":null}))
                .unwrap();
        assert_eq!(response.is_success(), success);
    }
}
#[test]
fn frame_classification_uses_structure_and_only_string_echo_can_correlate() {
    assert!(matches!(Frame::parse(MESSAGE).unwrap(), Frame::Event(_)));
    for echo in [json!(null), json!(1), json!({"nested":true}), json!([1, 2])] {
        let original = json!({"status":"ok","retcode":0,"data":{},"echo":echo});
        let response = ActionResponse::from_value(original.clone()).unwrap();
        assert_eq!(response.echo_str(), None);
        assert_eq!(serde_json::to_value(response).unwrap(), original);
    }
    let missing = json!({"status":"ok","retcode":0,"data":null});
    assert_eq!(
        serde_json::to_value(ActionResponse::from_value(missing.clone()).unwrap()).unwrap(),
        missing
    );
    for text in [
        r#"{"echo":"x"}"#,
        r#"{"status":"ok","retcode":0}"#,
        r#"{"status":"ok","data":null}"#,
        r#"{"status":"ok","retcode":"0","data":null}"#,
        r#"{"status":"ok","retcode":0.1,"data":null}"#,
        r#"{"post_type":null,"status":"ok","retcode":0,"data":null}"#,
    ] {
        assert!(Frame::parse(text).is_err(), "accepted invalid frame");
    }
    assert!(ActionResponse::parse(MESSAGE).is_err());
}

#[test]
fn typed_messages_and_reply_extensions_keep_large_numbers_as_numbers() {
    let original:Value=serde_json::from_str(r#"[{"type":"future","data":{"number":184467440737095516170},"huge":184467440737095516170}]"#).unwrap();
    let message = Message::from_value(original.clone()).unwrap();
    assert_eq!(serde_json::to_value(message).unwrap(), original);
    let response =
        r#"{"status":"failed","retcode":1,"data":null,"vendor_number":184467440737095516170}"#;
    assert_eq!(
        serde_json::to_value(ActionResponse::parse(response).unwrap()).unwrap(),
        serde_json::from_str::<Value>(response).unwrap()
    );
}
