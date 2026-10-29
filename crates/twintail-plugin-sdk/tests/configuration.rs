//! Configuration transaction and lifecycle-drain duplex contracts.
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{DuplexStream, ReadHalf, WriteHalf},
    sync::Notify,
    time::timeout,
};
use twintail_plugin_sdk::*;
use twintail_protocol::{Body, MAX_FRAME_BYTES, Message, Outcome, read_message, write_message};
type Host = (ReadHalf<DuplexStream>, WriteHalf<DuplexStream>);
async fn send(host: &mut Host, message: Message) {
    write_message(&mut host.1, &message, MAX_FRAME_BYTES)
        .await
        .unwrap();
}
async fn recv(host: &mut Host) -> Message {
    timeout(
        Duration::from_secs(2),
        read_message(&mut host.0, MAX_FRAME_BYTES),
    )
    .await
    .unwrap()
    .unwrap()
}
async fn call(host: &mut Host, id: &str, method: &str, payload: Value) -> Result<Value> {
    send(host, Message::request(id, method, payload)).await;
    response(recv(host).await, id)
}
fn response(message: Message, id: &str) -> Result<Value> {
    match message.body {
        Body::Response {
            request_id,
            outcome,
        } => {
            assert_eq!(request_id, id);
            match outcome {
                Outcome::Ok(v) => Ok(v),
                Outcome::Err(e) => Err(e),
            }
        }
        other => panic!("expected response, got {other:?}"),
    }
}
fn snapshot(revision: u64) -> Value {
    json!({"config_version":1,"revision":revision,"values":{"label":format!("revision-{revision}")}})
}
async fn launch<P: Plugin>(
    plugin: P,
    config: Value,
    options: RuntimeOptions,
) -> (Host, tokio::task::JoinHandle<Result>) {
    let (client, host) = tokio::io::duplex(65536);
    let (r, w) = tokio::io::split(client);
    let mut host = tokio::io::split(host);
    let task = tokio::spawn(run_io(
        plugin,
        Hello {
            plugin_id: "config-test".into(),
            version: "1.0.0".into(),
            required_capabilities: vec![],
        },
        r,
        w,
        options,
    ));
    assert!(matches!(recv(&mut host).await.body,Body::Control{method,..} if method=="hello"));
    send(
        &mut host,
        Message::control("negotiate", json!({"major":1,"minor":2})),
    )
    .await;
    send(&mut host,Message::control("initialize",json!({"instance_id":"config-1","generation":1,"granted_capabilities":[],"data_dir":"/tmp/test","config_revision":config.get("revision").and_then(Value::as_u64).unwrap_or(0),"config":config}))).await;
    assert!(matches!(recv(&mut host).await.body,Body::Control{method,..} if method=="ready"));
    send(&mut host, Message::control("activate", json!({}))).await;
    (host, task)
}
async fn stop(host: &mut Host, task: tokio::task::JoinHandle<Result>) {
    send(host, Message::control("shutdown", json!({}))).await;
    timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
struct Configurable;
impl Plugin for Configurable {
    async fn prepare_config(&self, context: Context, snapshot: ConfigSnapshot) -> Result {
        if snapshot.values.get("reject") == Some(&json!(true)) {
            return Err(RpcError::new(
                ErrorCode::InvalidConfig,
                "candidate rejected",
            ));
        }
        if snapshot.values.get("nested") == Some(&json!(true)) {
            context.health().await?;
        }
        Ok(())
    }
    async fn commit_config(&self, _: Context, snapshot: ConfigSnapshot) -> Result {
        if snapshot.values.get("fail") == Some(&json!(true)) {
            Err(RpcError::new(ErrorCode::InvalidConfig, "apply failed"))
        } else {
            Ok(())
        }
    }
    async fn request(&self, context: Context, method: String, _: Value) -> Result<Value> {
        match method.as_str() {
            "observe" => {
                Ok(json!({"current":context.config(),"initial":context.initialize().config}))
            }
            "hold" => context.health().await,
            _ => Err(RpcError::new(ErrorCode::Unsupported, "unknown")),
        }
    }
}
#[tokio::test]
async fn prepared_configuration_is_single_revisioned_and_failure_keeps_applied() {
    let (mut host, task) = launch(Configurable, snapshot(7), RuntimeOptions::default()).await;
    assert_eq!(
        call(&mut host, "status", "config.status", json!({}))
            .await
            .unwrap(),
        json!({"applied_revision":7,"config_version":1})
    );
    assert_eq!(
        call(
            &mut host,
            "stale",
            "config.prepare",
            json!({"expected_revision":6,"snapshot":snapshot(8)})
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );
    call(
        &mut host,
        "prepare",
        "config.prepare",
        json!({"expected_revision":7,"snapshot":snapshot(8)}),
    )
    .await
    .unwrap();
    assert_eq!(
        call(
            &mut host,
            "repeat",
            "config.prepare",
            json!({"expected_revision":7,"snapshot":snapshot(9)})
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        call(&mut host, "wrong", "config.commit", json!({"revision":9}))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    call(&mut host, "commit", "config.commit", json!({"revision":8}))
        .await
        .unwrap();
    let state = call(&mut host, "observe", "observe", json!({}))
        .await
        .unwrap();
    assert_eq!(state["current"], snapshot(8));
    assert_eq!(state["initial"], snapshot(7));
    let mut candidate = snapshot(9);
    candidate["values"]["fail"] = json!(true);
    call(
        &mut host,
        "prepare-fail",
        "config.prepare",
        json!({"expected_revision":8,"snapshot":candidate}),
    )
    .await
    .unwrap();
    assert_eq!(
        call(
            &mut host,
            "commit-fail",
            "config.commit",
            json!({"revision":9})
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::InvalidConfig
    );
    assert_eq!(
        call(
            &mut host,
            "commit-again",
            "config.commit",
            json!({"revision":9})
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        call(&mut host, "status-after", "config.status", json!({}))
            .await
            .unwrap(),
        json!({"applied_revision":8,"config_version":1})
    );
    let mut rejected = snapshot(9);
    rejected["values"]["reject"] = json!(true);
    assert_eq!(
        call(
            &mut host,
            "reject",
            "config.prepare",
            json!({"expected_revision":8,"snapshot":rejected})
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::InvalidConfig
    );
    call(
        &mut host,
        "prepare-retry",
        "config.prepare",
        json!({"expected_revision":8,"snapshot":snapshot(9)}),
    )
    .await
    .unwrap();
    assert_eq!(
        call(
            &mut host,
            "abort-wrong",
            "config.abort",
            json!({"revision":10})
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );
    call(&mut host, "abort", "config.abort", json!({"revision":9}))
        .await
        .unwrap();
    call(
        &mut host,
        "abort-idempotent",
        "config.abort",
        json!({"revision":9}),
    )
    .await
    .unwrap();
    call(
        &mut host,
        "prepare-after-abort",
        "config.prepare",
        json!({"expected_revision":8,"snapshot":snapshot(9)}),
    )
    .await
    .unwrap();
    stop(&mut host, task).await;
}
#[tokio::test]
async fn maintenance_serializes_prepare_commit_without_blocking_nested_response() {
    let (mut host, task) = launch(
        Configurable,
        Value::Null,
        RuntimeOptions {
            max_handlers: 1,
            ..Default::default()
        },
    )
    .await;
    let mut candidate = snapshot(1);
    candidate["values"]["nested"] = json!(true);
    send(
        &mut host,
        Message::request(
            "prepare",
            "config.prepare",
            json!({"expected_revision":0,"snapshot":candidate}),
        ),
    )
    .await;
    let nested = recv(&mut host).await;
    let Body::Request {
        request_id, method, ..
    } = nested.body
    else {
        panic!()
    };
    assert_eq!(method, "host.health");
    send(
        &mut host,
        Message::request("commit", "config.commit", json!({"revision":1})),
    )
    .await;
    assert!(
        timeout(
            Duration::from_millis(20),
            read_message(&mut host.0, MAX_FRAME_BYTES)
        )
        .await
        .is_err()
    );
    send(
        &mut host,
        Message::response(request_id, Outcome::Ok(json!({}))),
    )
    .await;
    response(recv(&mut host).await, "prepare").unwrap();
    response(recv(&mut host).await, "commit").unwrap();
    assert_eq!(
        call(&mut host, "status", "config.status", json!({}))
            .await
            .unwrap()["applied_revision"],
        1
    );
    stop(&mut host, task).await;
}
struct DefaultPlugin;
impl Plugin for DefaultPlugin {}
#[tokio::test]
async fn default_config_callbacks_are_unsupported_and_status_has_no_values() {
    let (mut host, task) = launch(DefaultPlugin, Value::Null, RuntimeOptions::default()).await;
    assert_eq!(
        call(&mut host, "status", "config.status", json!({}))
            .await
            .unwrap(),
        json!({"applied_revision":null,"config_version":null})
    );
    assert_eq!(
        call(
            &mut host,
            "prepare",
            "config.prepare",
            json!({"expected_revision":0,"snapshot":snapshot(1)})
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::Unsupported
    );
    assert_eq!(
        call(&mut host, "commit", "config.commit", json!({"revision":1}))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    stop(&mut host, task).await;
}
struct Startup {
    gate: Arc<Notify>,
    entered: Arc<Notify>,
}
impl Plugin for Startup {
    async fn start(&self, _: Context) -> Result {
        self.entered.notify_one();
        self.gate.notified().await;
        Ok(())
    }
}
#[tokio::test]
async fn config_status_waits_for_successful_start() {
    let gate = Arc::new(Notify::new());
    let entered = Arc::new(Notify::new());
    let (mut host, task) = launch(
        Startup {
            gate: gate.clone(),
            entered: entered.clone(),
        },
        snapshot(2),
        RuntimeOptions::default(),
    )
    .await;
    entered.notified().await;
    send(
        &mut host,
        Message::request("status", "config.status", json!({})),
    )
    .await;
    assert!(
        timeout(
            Duration::from_millis(20),
            read_message(&mut host.0, MAX_FRAME_BYTES)
        )
        .await
        .is_err()
    );
    send(
        &mut host,
        Message::request("started", "lifecycle.started", json!({})),
    )
    .await;
    gate.notify_one();
    assert_eq!(
        response(recv(&mut host).await, "status").unwrap()["applied_revision"],
        2
    );
    assert_eq!(
        response(recv(&mut host).await, "started").unwrap(),
        json!({"started":true,"config_revision":2})
    );
    stop(&mut host, task).await;
}
struct Draining {
    events: Arc<AtomicUsize>,
}
impl Plugin for Draining {
    async fn event(&self, _: Context, _: EventEnvelope) -> Result {
        self.events.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn request(&self, context: Context, _: String, _: Value) -> Result<Value> {
        context.health().await
    }
}
#[tokio::test]
async fn drain_stops_admission_but_resolves_running_nested_rpc() {
    let events = Arc::new(AtomicUsize::new(0));
    let (mut host, task) = launch(
        Draining {
            events: events.clone(),
        },
        Value::Null,
        RuntimeOptions {
            max_handlers: 1,
            ..Default::default()
        },
    )
    .await;
    send(&mut host, Message::request("hold", "hold", json!({}))).await;
    let Body::Request { request_id, .. } = recv(&mut host).await.body else {
        panic!()
    };
    send(
        &mut host,
        Message::request("drain", "lifecycle.drain", json!({})),
    )
    .await;
    send(&mut host, Message::request("new", "hold", json!({}))).await;
    assert_eq!(
        response(recv(&mut host).await, "new").unwrap_err().code,
        ErrorCode::Cancelled
    );
    send(&mut host,Message{protocol_version:twintail_protocol::VERSION,body:Body::Event{method:"events.deliver".into(),payload:json!({"event_id":"e","bot":{"adapter_instance_id":"a-1","self_id":"1"},"received_at_ms":0,"protocol":"mock","event_type":"message","payload":{}})}}).await;
    assert!(
        timeout(
            Duration::from_millis(20),
            read_message(&mut host.0, MAX_FRAME_BYTES)
        )
        .await
        .is_err()
    );
    send(
        &mut host,
        Message::response(request_id, Outcome::Ok(json!({"ok":true}))),
    )
    .await;
    assert_eq!(
        response(recv(&mut host).await, "hold").unwrap(),
        json!({"ok":true})
    );
    assert_eq!(
        response(recv(&mut host).await, "drain").unwrap(),
        json!({"drained":true})
    );
    assert_eq!(events.load(Ordering::SeqCst), 0);
    assert_eq!(
        call(&mut host, "health", "host.health", json!({}))
            .await
            .unwrap(),
        json!({"status":"ok"})
    );
    stop(&mut host, task).await;
}
#[tokio::test]
async fn drain_timeout_is_bounded_and_keeps_new_business_closed() {
    let (mut host, task) = launch(
        Configurable,
        Value::Null,
        RuntimeOptions {
            max_handlers: 1,
            drain_timeout: Duration::from_millis(20),
            ..Default::default()
        },
    )
    .await;
    send(&mut host, Message::request("hold", "hold", json!({}))).await;
    let Body::Request { request_id, .. } = recv(&mut host).await.body else {
        panic!()
    };
    assert_eq!(
        call(&mut host, "drain", "lifecycle.drain", json!({}))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Timeout
    );
    assert_eq!(
        call(&mut host, "new", "observe", json!({}))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    send(
        &mut host,
        Message::response(request_id, Outcome::Ok(json!({}))),
    )
    .await;
    response(recv(&mut host).await, "hold").unwrap();
    assert_eq!(
        call(&mut host, "drain-again", "lifecycle.drain", json!({}))
            .await
            .unwrap(),
        json!({"drained":true})
    );
    stop(&mut host, task).await;
}
