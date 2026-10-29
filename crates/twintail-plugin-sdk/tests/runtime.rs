//! Duplex wire tests independent of process supervisor routing.
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{DuplexStream, ReadHalf, WriteHalf},
    sync::oneshot,
    time::timeout,
};
use twintail_plugin_sdk::*;
use twintail_protocol::{
    Body, MAX_FRAME_BYTES, Message, Outcome, VERSION, read_message, write_message,
};

type Host = (ReadHalf<DuplexStream>, WriteHalf<DuplexStream>);
fn hello() -> Hello {
    Hello {
        plugin_id: "test".into(),
        version: "0.1.0".into(),
        required_capabilities: vec![],
    }
}
async fn send(host: &mut Host, message: Message) {
    write_message(&mut host.1, &message, MAX_FRAME_BYTES)
        .await
        .unwrap();
}
async fn receive(host: &mut Host) -> Message {
    timeout(
        Duration::from_secs(2),
        read_message(&mut host.0, MAX_FRAME_BYTES),
    )
    .await
    .unwrap()
    .unwrap()
}
async fn handshake(host: &mut Host) {
    assert!(matches!(receive(host).await.body,Body::Control{method,..} if method=="hello"));
    send(
        host,
        Message::control("negotiate", json!({"major":1,"minor":1})),
    )
    .await;
    send(host,Message::control("initialize",json!({"instance_id":"test-1","generation":1,"granted_capabilities":[],"data_dir":"/tmp/test","config_revision":1}))).await;
    assert!(matches!(receive(host).await.body,Body::Control{method,..} if method=="ready"));
    send(host, Message::control("activate", json!({}))).await;
}
struct Capture(Mutex<Option<oneshot::Sender<Context>>>);
impl Plugin for Capture {
    async fn start(&self, context: Context) -> Result {
        self.0.lock().unwrap().take().unwrap().send(context).ok();
        Ok(())
    }
}
async fn captured(options: RuntimeOptions) -> (Host, Context, tokio::task::JoinHandle<Result>) {
    let (client, host) = tokio::io::duplex(65536);
    let (r, w) = tokio::io::split(client);
    let mut host = tokio::io::split(host);
    let (tx, rx) = oneshot::channel();
    let task = tokio::spawn(run_io(
        Capture(Mutex::new(Some(tx))),
        hello(),
        r,
        w,
        options,
    ));
    handshake(&mut host).await;
    (host, rx.await.unwrap(), task)
}
async fn stop(host: &mut Host, task: tokio::task::JoinHandle<Result>) {
    send(host, Message::control("shutdown", json!({}))).await;
    assert!(
        timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
}
#[tokio::test]
async fn host_health_and_new_optional_minor_are_supported() {
    let (mut host, context, task) = captured(RuntimeOptions::default()).await;
    let call = tokio::spawn(async move { context.health().await });
    let Body::Request {
        request_id, method, ..
    } = receive(&mut host).await.body
    else {
        panic!("request expected")
    };
    assert_eq!(method, "host.health");
    let mut response = Message::response(request_id, Outcome::Ok(json!({"status":"ok"})));
    response.protocol_version.minor = 3;
    send(&mut host, response).await;
    assert_eq!(call.await.unwrap().unwrap()["status"], "ok");
    stop(&mut host, task).await;
}
#[tokio::test]
async fn handshake_version_and_deadline_are_enforced() {
    let (client, host) = tokio::io::duplex(4096);
    let (r, w) = tokio::io::split(client);
    let mut host = tokio::io::split(host);
    let task = tokio::spawn(run_io(
        Capture(Mutex::new(None)),
        hello(),
        r,
        w,
        RuntimeOptions::default(),
    ));
    receive(&mut host).await;
    let mut message = Message::control("negotiate", json!({"major":2,"minor":0}));
    message.protocol_version.major = 2;
    send(&mut host, message).await;
    assert_eq!(
        task.await.unwrap().unwrap_err().code,
        ErrorCode::IncompatibleVersion
    );
    let (client, _host) = tokio::io::duplex(4096);
    let (r, w) = tokio::io::split(client);
    let options = RuntimeOptions {
        handshake_timeout: Duration::from_millis(20),
        ..Default::default()
    };
    assert_eq!(
        run_io(Capture(Mutex::new(None)), hello(), r, w, options)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Timeout
    );
}
fn action() -> ActionCall {
    ActionCall {
        bot: BotRef {
            adapter_instance_id: "adapter-1".into(),
            self_id: "10001".into(),
        },
        action: "send_message".into(),
        params: json!({"message":[]}),
    }
}
#[tokio::test]
async fn pending_timeout_and_explicit_refusal_do_not_leak_or_retry() {
    let options = RuntimeOptions {
        request_timeout: Duration::from_millis(30),
        max_pending: 1,
        ..Default::default()
    };
    let (mut host, context, task) = captured(options).await;
    let call_context = context.clone();
    let call = tokio::spawn(async move { call_context.call_action(action()).await });
    receive(&mut host).await;
    assert_eq!(
        context.health().await.unwrap_err().code,
        ErrorCode::Overloaded
    );
    assert_eq!(
        call.await.unwrap().unwrap_err().code,
        ErrorCode::UnknownOutcome
    );
    let call = tokio::spawn(async move { context.call_action(action()).await });
    let Body::Request { request_id, .. } = receive(&mut host).await.body else {
        panic!()
    };
    send(
        &mut host,
        Message::response(
            request_id,
            Outcome::Err(RpcError::new(ErrorCode::PermissionDenied, "denied")),
        ),
    )
    .await;
    assert_eq!(
        call.await.unwrap().unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    stop(&mut host, task).await;
}
#[tokio::test]
async fn disconnect_after_action_admission_reports_unknown_outcome() {
    let (mut host, context, task) = captured(RuntimeOptions::default()).await;
    let call = tokio::spawn(async move { context.call_action(action()).await });
    receive(&mut host).await;
    drop(host);
    assert_eq!(
        call.await.unwrap().unwrap_err().code,
        ErrorCode::UnknownOutcome
    );
    assert!(task.await.unwrap().is_err());
}
struct Nested;
impl Plugin for Nested {
    async fn event(&self, context: Context, event: EventEnvelope) -> Result {
        context
            .call_action(ActionCall {
                bot: event.bot,
                action: "send_message".into(),
                params: event.payload,
            })
            .await?;
        Ok(())
    }
    async fn request(&self, _: Context, _: String, _: Value) -> Result<Value> {
        Ok(json!("alive"))
    }
}
#[tokio::test]
async fn event_handler_can_await_nested_rpc_while_reader_resolves_response() {
    let (client, host) = tokio::io::duplex(65536);
    let (r, w) = tokio::io::split(client);
    let mut host = tokio::io::split(host);
    let task = tokio::spawn(run_io(
        Nested,
        hello(),
        r,
        w,
        RuntimeOptions {
            max_handlers: 1,
            ..Default::default()
        },
    ));
    handshake(&mut host).await;
    send(
        &mut host,
        Message {
            protocol_version: VERSION,
            body: Body::Event {
                method: "events.deliver".into(),
                payload: serde_json::to_value(EventEnvelope {
                    event_id: "event1".into(),
                    bot: action().bot,
                    received_at_ms: 1,
                    protocol: "mock.v1".into(),
                    event_type: "message".into(),
                    payload: json!({"message":[{"type":"text","data":{"text":"hi"}}]}),
                })
                .unwrap(),
            },
        },
    )
    .await;
    let Body::Request {
        request_id,
        method,
        payload,
    } = receive(&mut host).await.body
    else {
        panic!()
    };
    assert_eq!(method, "actions.call");
    assert_eq!(payload["params"]["message"][0]["data"]["text"], "hi");
    send(
        &mut host,
        Message::response(request_id, Outcome::Ok(json!({}))),
    )
    .await;
    send(&mut host, Message::request("probe", "test", json!({}))).await;
    assert!(matches!(
        receive(&mut host).await.body,
        Body::Response {
            outcome: Outcome::Ok(_),
            ..
        }
    ));
    stop(&mut host, task).await;
}
struct Slow {
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    stopped: Arc<AtomicUsize>,
}
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Plugin for Slow {
    async fn request(&self, _: Context, _: String, _: Value) -> Result<Value> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        let _guard = Active(self.active.clone());
        std::future::pending().await
    }
    async fn stop(&self, _: Context) -> Result {
        assert_eq!(self.active.load(Ordering::SeqCst), 0);
        self.stopped.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
#[tokio::test]
async fn concurrency_is_bounded_and_shutdown_cancels_joins_handlers() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let stopped = Arc::new(AtomicUsize::new(0));
    let (client, host) = tokio::io::duplex(65536);
    let (r, w) = tokio::io::split(client);
    let mut host = tokio::io::split(host);
    let task = tokio::spawn(run_io(
        Slow {
            active: active.clone(),
            peak: peak.clone(),
            stopped: stopped.clone(),
        },
        hello(),
        r,
        w,
        RuntimeOptions {
            max_handlers: 2,
            ..Default::default()
        },
    ));
    handshake(&mut host).await;
    for i in 0..8 {
        send(
            &mut host,
            Message::request(format!("slow-{i}"), "slow", json!({})),
        )
        .await;
    }
    timeout(Duration::from_secs(2), async {
        while active.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await
        }
    })
    .await
    .unwrap();
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    stop(&mut host, task).await;
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert_eq!(stopped.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn duplicate_request_ids_retire_runtime() {
    let (mut host, _context, task) = captured(RuntimeOptions::default()).await;
    send(&mut host, Message::request("dup", "custom", json!({}))).await;
    send(&mut host, Message::request("dup", "custom", json!({}))).await;
    assert_eq!(
        timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .code,
        ErrorCode::InvalidFrame
    );
}

#[tokio::test]
async fn full_inbound_queue_retires_without_leaking_handlers() {
    let active = Arc::new(AtomicUsize::new(0));
    let stopped = Arc::new(AtomicUsize::new(0));
    let (client, host) = tokio::io::duplex(65536);
    let (r, w) = tokio::io::split(client);
    let mut host = tokio::io::split(host);
    let task = tokio::spawn(run_io(
        Slow {
            active: active.clone(),
            peak: Arc::new(AtomicUsize::new(0)),
            stopped: stopped.clone(),
        },
        hello(),
        r,
        w,
        RuntimeOptions {
            max_handlers: 1,
            queue_capacity: 1,
            ..Default::default()
        },
    ));
    handshake(&mut host).await;
    send(&mut host, Message::request("active", "slow", json!({}))).await;
    timeout(Duration::from_secs(2), async {
        while active.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    send(&mut host, Message::request("queued", "slow", json!({}))).await;
    send(&mut host, Message::request("overflow", "slow", json!({}))).await;
    assert_eq!(
        timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .code,
        ErrorCode::Overloaded
    );
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert_eq!(stopped.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn typed_management_bridge_preserves_operation_ack_and_permission_errors() {
    let (mut host, context, task) = captured(RuntimeOptions::default()).await;
    let call_context = context.clone();
    let call = tokio::spawn(async move {
        call_context
            .management(ManagementCall {
                method: "plugins.reload".into(),
                payload: json!({"id":"test","version":"0.2.0"}),
            })
            .await
    });
    let Body::Request {
        request_id,
        method,
        payload,
    } = receive(&mut host).await.body
    else {
        panic!("request expected")
    };
    assert_eq!(method, "management.call");
    assert_eq!(
        payload,
        json!({"method":"plugins.reload","payload":{"id":"test","version":"0.2.0"}})
    );
    send(
        &mut host,
        Message::response(
            request_id,
            Outcome::Ok(json!({"operation_id":"op-1","state":"running"})),
        ),
    )
    .await;
    assert_eq!(call.await.unwrap().unwrap()["operation_id"], "op-1");
    let denied = tokio::spawn(async move {
        context
            .management(ManagementCall {
                method: "plugins.list".into(),
                payload: json!({}),
            })
            .await
    });
    let Body::Request { request_id, .. } = receive(&mut host).await.body else {
        panic!()
    };
    send(
        &mut host,
        Message::response(
            request_id,
            Outcome::Err(RpcError::new(
                ErrorCode::PermissionDenied,
                "read capability absent",
            )),
        ),
    )
    .await;
    assert_eq!(
        denied.await.unwrap().unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    stop(&mut host, task).await;
}

#[tokio::test]
async fn four_pending_management_calls_leave_ping_and_health_responsive() {
    let (mut host, context, task) = captured(RuntimeOptions::default()).await;
    let mut calls = Vec::new();
    for method in [
        "plugins.list",
        "plugins.installed",
        "operations.list",
        "management.overview",
    ] {
        let context = context.clone();
        calls.push(tokio::spawn(async move {
            context
                .management(ManagementCall {
                    method: method.into(),
                    payload: json!({}),
                })
                .await
        }));
    }
    let mut pending = Vec::new();
    for _ in 0..4 {
        let Body::Request {
            request_id,
            method,
            payload,
        } = receive(&mut host).await.body
        else {
            panic!("expected management request")
        };
        assert_eq!(method, "management.call");
        pending.push((request_id, payload["method"].clone()));
    }
    send(&mut host, Message::control("ping", json!({"sequence":1}))).await;
    assert!(matches!(receive(&mut host).await.body,Body::Control{method,..} if method=="pong"));
    send(
        &mut host,
        Message::request("health-control", "host.health", json!({})),
    )
    .await;
    assert!(
        matches!(receive(&mut host).await.body,Body::Response{request_id,outcome:Outcome::Ok(value)} if request_id=="health-control" && value["status"]=="ok")
    );
    // Resolve out of order while all callers remain pending; direct reader dispatch
    // must correlate responses rather than await a management or business handler.
    for (id, method) in pending.into_iter().rev() {
        send(
            &mut host,
            Message::response(id, Outcome::Ok(json!({"method":method}))),
        )
        .await;
    }
    for call in calls {
        assert!(call.await.unwrap().unwrap()["method"].is_string());
    }
    stop(&mut host, task).await;
}
