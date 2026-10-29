use super::*;
use std::time::Duration;

struct Fixture {
    identity: Identity,
    _registration: Registration,
    commands: mpsc::Receiver<InstanceCommand>,
    events: mpsc::Receiver<Message>,
}
fn peer(router: &SharedRouter, id: &str, generation: u64, capacity: usize) -> Fixture {
    let identity = Identity {
        id: id.into(),
        instance_id: format!("{id}-{generation}"),
        generation,
    };
    let (tx, commands) = mpsc::channel(capacity);
    let (tx_events, events) = mpsc::channel(capacity);
    let registration = Router::activate(router, identity.clone(), tx, tx_events);
    Fixture {
        identity,
        _registration: registration,
        commands,
        events,
    }
}
fn dispatch(
    router: &SharedRouter,
    source: &Identity,
    method: &str,
    payload: Value,
) -> Result<Dispatch, RpcError> {
    router
        .lock()
        .unwrap()
        .dispatch(source, method, payload, Duration::from_millis(20))
}
fn ready(result: Result<Dispatch, RpcError>) -> Value {
    match result.unwrap() {
        Dispatch::Ready(value) => value,
        _ => panic!("expected immediate result"),
    }
}
fn failure(result: Result<Dispatch, RpcError>) -> ErrorCode {
    match result {
        Err(e) => e.code,
        _ => panic!("expected failure"),
    }
}
fn action(
    router: &SharedRouter,
    source: &Identity,
    target: &Identity,
) -> Result<Dispatch, RpcError> {
    dispatch(
        router,
        source,
        "actions.call",
        json!({"bot":{"adapter_instance_id":target.instance_id,"self_id":"42"},"action":"send_message","params":[null,{"extension":true}]}),
    )
}
fn publish(router: &SharedRouter, source: &Identity) -> Result<Dispatch, RpcError> {
    dispatch(
        router,
        source,
        "events.publish",
        json!({"self_id":"42","protocol":"mock.v1","event_type":"message","payload":{"segments":[{"type":"unknown","data":{"x":[1,null,true]}}],"extension":42}}),
    )
}
fn register(router: &SharedRouter, source: &Identity) {
    let value = ready(dispatch(
        router,
        source,
        "bots.register",
        json!({"self_id":"42"}),
    ));
    assert_eq!(
        value,
        json!({"adapter_instance_id":source.instance_id,"self_id":"42"})
    );
}

#[test]
fn event_order_payload_and_slow_subscriber_accounting() {
    let router = SharedRouter::default();
    let adapter = peer(&router, "adapter", 1, 2);
    let mut fast = peer(&router, "fast", 1, 2);
    let mut slow = peer(&router, "slow", 1, 1);
    for source in [&fast.identity, &slow.identity] {
        ready(dispatch(
            &router,
            source,
            "events.subscribe",
            json!({"event_types":["message","message"]}),
        ));
    }
    assert_eq!(
        failure(publish(&router, &adapter.identity)),
        ErrorCode::NotFound
    );
    register(&router, &adapter.identity);
    assert_eq!(
        ready(publish(&router, &adapter.identity)),
        json!({"delivered":2,"dropped":0})
    );
    register(&router, &adapter.identity);
    assert_eq!(
        ready(publish(&router, &adapter.identity)),
        json!({"delivered":1,"dropped":1})
    );
    let a = fast.events.try_recv().unwrap();
    let b = fast.events.try_recv().unwrap();
    let Body::Event { method, payload } = a.body else {
        panic!()
    };
    assert_eq!(method, "events.deliver");
    let first: EventEnvelope = serde_json::from_value(payload).unwrap();
    let Body::Event { payload, .. } = b.body else {
        panic!()
    };
    let second: EventEnvelope = serde_json::from_value(payload).unwrap();
    assert_eq!(first.event_id, "adapter-1:42:1");
    assert_eq!(second.event_id, "adapter-1:42:2");
    assert_eq!(
        first.payload["segments"][0]["data"]["x"],
        json!([1, null, true])
    );
    assert_eq!(first.payload["extension"], 42);
    assert_eq!(
        slow.events.try_recv().unwrap().body,
        Message {
            protocol_version: VERSION,
            body: Body::Event {
                method,
                payload: serde_json::to_value(first).unwrap()
            }
        }
        .body
    );
    ready(dispatch(
        &router,
        &fast.identity,
        "events.subscribe",
        json!({"event_types":[]}),
    ));
    assert_eq!(
        ready(publish(&router, &adapter.identity)),
        json!({"delivered":1,"dropped":0})
    );
}

#[tokio::test]
async fn target_generation_is_fixed_and_source_retirement_rejects_operations() {
    let router = SharedRouter::default();
    let caller = peer(&router, "caller", 1, 2);
    let mut old = peer(&router, "adapter", 1, 2);
    register(&router, &old.identity);
    let Dispatch::Forward(call) = action(&router, &caller.identity, &old.identity).unwrap() else {
        panic!()
    };
    let InstanceCommand::Call {
        reply,
        method,
        payload,
        ..
    } = old.commands.try_recv().unwrap()
    else {
        panic!()
    };
    assert_eq!(method, "actions.handle");
    assert_eq!(payload["params"], json!([null,{"extension":true}]));
    let new = peer(&router, "adapter", 2, 2);
    register(&router, &new.identity);
    assert_eq!(failure(publish(&router, &old.identity)), ErrorCode::Offline);
    assert_eq!(
        failure(action(&router, &old.identity, &new.identity)),
        ErrorCode::Offline
    );
    drop(old);
    // The old guard must never remove the replacement registration.
    assert!(action(&router, &caller.identity, &new.identity).is_ok());
    assert_eq!(
        failure(action(
            &router,
            &caller.identity,
            &Identity {
                instance_id: "adapter-1".into(),
                ..new.identity.clone()
            }
        )),
        ErrorCode::Offline
    );
    reply.send(Ok(json!({"from":"old"}))).unwrap();
    assert_eq!(
        call.complete(Duration::from_secs(1)).await.unwrap(),
        json!({"from":"old"})
    );
}

#[tokio::test]
async fn forwarding_queue_admission_disconnect_and_timeout_are_explicit() {
    let router = SharedRouter::default();
    let caller = peer(&router, "caller", 1, 1);
    let mut adapter = peer(&router, "adapter", 1, 1);
    register(&router, &adapter.identity);
    let Dispatch::Forward(call) = action(&router, &caller.identity, &adapter.identity).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        failure(action(&router, &caller.identity, &adapter.identity)),
        ErrorCode::Overloaded
    );
    drop(adapter.commands.try_recv().unwrap());
    assert_eq!(
        call.complete(Duration::from_secs(1))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UnknownOutcome
    );
    let Dispatch::Forward(call) = action(&router, &caller.identity, &adapter.identity).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        call.complete(Duration::from_millis(1))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UnknownOutcome
    );
    drop(adapter);
    assert_eq!(
        failure(action(
            &router,
            &caller.identity,
            &Identity {
                id: "adapter".into(),
                instance_id: "adapter-1".into(),
                generation: 1
            }
        )),
        ErrorCode::Offline
    );
}

#[tokio::test]
async fn services_require_explicit_provider_and_matching_version() {
    let router = SharedRouter::default();
    let caller = peer(&router, "caller", 1, 2);
    let mut provider = peer(&router, "provider", 1, 2);
    ready(dispatch(
        &router,
        &provider.identity,
        "services.register",
        json!({"name":"example.echo","version":"1.2.3"}),
    ));
    let mut payload = json!({"provider":"provider","service":"example.echo","version_req":"^1.0","method":"echo","params":[1,{"arbitrary":null}]});
    let Dispatch::Forward(call) =
        dispatch(&router, &caller.identity, "services.call", payload.clone()).unwrap()
    else {
        panic!()
    };
    let InstanceCommand::Call {
        reply,
        method,
        payload: forwarded,
        ..
    } = provider.commands.try_recv().unwrap()
    else {
        panic!()
    };
    assert_eq!(method, "services.handle");
    assert_eq!(forwarded, payload);
    reply.send(Ok(forwarded["params"].clone())).unwrap();
    assert_eq!(
        call.complete(Duration::from_secs(1)).await.unwrap(),
        payload["params"]
    );
    for (requirement, expected) in [
        ("^2", ErrorCode::IncompatibleVersion),
        ("bogus", ErrorCode::InvalidRequest),
    ] {
        payload["version_req"] = requirement.into();
        assert_eq!(
            failure(dispatch(
                &router,
                &caller.identity,
                "services.call",
                payload.clone()
            )),
            expected
        );
    }
    payload["version_req"] = "*".into();
    payload["provider"] = "absent".into();
    assert_eq!(
        failure(dispatch(
            &router,
            &caller.identity,
            "services.call",
            payload
        )),
        ErrorCode::Offline
    );
    assert_eq!(
        failure(dispatch(
            &router,
            &provider.identity,
            "services.register",
            json!({"name":"x","version":"1"})
        )),
        ErrorCode::InvalidRequest
    );
}

#[test]
fn malformed_names_lengths_and_registration_budgets_are_bounded() {
    let router = SharedRouter::default();
    let adapter = peer(&router, "adapter", 1, 1);
    for value in ["".to_string(), "x".repeat(129), "bad\nvalue".into()] {
        assert_eq!(
            failure(dispatch(
                &router,
                &adapter.identity,
                "bots.register",
                json!({"self_id":value})
            )),
            ErrorCode::InvalidRequest
        );
    }
    for index in 0..REGISTRATIONS {
        ready(dispatch(
            &router,
            &adapter.identity,
            "bots.register",
            json!({"self_id":index.to_string()}),
        ));
    }
    assert_eq!(
        failure(dispatch(
            &router,
            &adapter.identity,
            "bots.register",
            json!({"self_id":"overflow"})
        )),
        ErrorCode::Overloaded
    );
    ready(dispatch(
        &router,
        &adapter.identity,
        "bots.register",
        json!({"self_id":"0"}),
    ));
    assert_eq!(
        failure(dispatch(
            &router,
            &adapter.identity,
            "events.subscribe",
            json!({"event_types":vec!["message";REGISTRATIONS+1]})
        )),
        ErrorCode::Overloaded
    );
    assert_eq!(
        failure(dispatch(
            &router,
            &adapter.identity,
            "events.subscribe",
            json!({"event_types":["bad name"]})
        )),
        ErrorCode::InvalidRequest
    );
    assert_eq!(capability("services.register"), Some("services.provide"));
    assert_eq!(capability("events.publish"), Some("events.publish"));
}

#[test]
fn retirement_guard_removes_all_registration_types() {
    let router = SharedRouter::default();
    let adapter = peer(&router, "adapter", 1, 1);
    register(&router, &adapter.identity);
    ready(dispatch(
        &router,
        &adapter.identity,
        "events.subscribe",
        json!({"event_types":["message"]}),
    ));
    ready(dispatch(
        &router,
        &adapter.identity,
        "services.register",
        json!({"name":"x","version":"1.0.0"}),
    ));
    drop(adapter);
    assert!(router.lock().unwrap().peers.is_empty());
}

#[tokio::test]
async fn cancellation_before_first_task_poll_revokes_registration() {
    let router = SharedRouter::default();
    let identity = Identity {
        id: "cancelled".into(),
        instance_id: "cancelled-1".into(),
        generation: 1,
    };
    let (commands, _rx) = mpsc::channel(1);
    let (events, _rx_events) = mpsc::channel(1);
    let registration = Router::activate(&router, identity.clone(), commands, events);
    let task = tokio::spawn(async move {
        let _registration = registration;
        std::future::pending::<()>().await;
    });
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(router.lock().unwrap().peers.is_empty());
    assert_eq!(
        failure(dispatch(
            &router,
            &identity,
            "bots.register",
            json!({"self_id":"42"})
        )),
        ErrorCode::Offline
    );
}

#[test]
fn quiescing_rejects_new_target_calls_but_preserves_source_nested_rpc() {
    let router = SharedRouter::default();
    let adapter = peer(&router, "adapter", 1, 4);
    let caller = peer(&router, "caller", 1, 4);
    register(&router, &adapter.identity);
    register(&router, &caller.identity);
    ready(dispatch(
        &router,
        &adapter.identity,
        "events.subscribe",
        json!({"event_types":["message"]}),
    ));
    router.lock().unwrap().quiesce("adapter");
    assert_eq!(
        failure(action(&router, &caller.identity, &adapter.identity)),
        ErrorCode::Offline
    );
    assert!(action(&router, &adapter.identity, &caller.identity).is_ok());
    assert_eq!(
        ready(publish(&router, &caller.identity)),
        json!({"delivered":0,"dropped":0})
    );
    router.lock().unwrap().retire(&adapter.identity);
    assert_eq!(
        failure(action(&router, &adapter.identity, &caller.identity)),
        ErrorCode::Offline
    );
}

fn manage(
    router: &SharedRouter,
    source: &Identity,
    grants: &[&str],
    method: &str,
    payload: Value,
) -> Result<Dispatch, RpcError> {
    router.lock().unwrap().management(
        source,
        &grants.iter().map(|g| (*g).to_owned()).collect::<Vec<_>>(),
        json!({"method":method,"payload":payload}),
        Duration::from_millis(100),
    )
}
#[tokio::test]
async fn management_bridge_authenticates_source_and_requires_exact_read_write_grants() {
    let router = Arc::new(Mutex::new(Router::default()));
    let source = peer(&router, "web", 7, 4);
    let (tx, mut rx) = mpsc::channel(1);
    router.lock().unwrap().management = Some(tx);
    assert_eq!(
        failure(manage(
            &router,
            &source.identity,
            &[],
            "plugins.list",
            json!({})
        )),
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        failure(manage(
            &router,
            &source.identity,
            &["management.write"],
            "plugins.list",
            json!({})
        )),
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        failure(manage(
            &router,
            &source.identity,
            &["management.read"],
            "plugins.stop",
            json!({"id":"web"})
        )),
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        failure(manage(
            &router,
            &source.identity,
            &["management.*"],
            "plugins.list",
            json!({})
        )),
        ErrorCode::PermissionDenied
    );
    let Dispatch::Forward(call) = manage(
        &router,
        &source.identity,
        &["management.read"],
        "plugins.list",
        json!({"plugin_id":"spoofed","generation":1}),
    )
    .unwrap() else {
        panic!()
    };
    let request = rx.recv().await.unwrap();
    assert_eq!(request.plugin_id, "web");
    assert_eq!(request.instance_id, "web-7");
    assert_eq!(request.generation, 7);
    assert!(request.is_active());
    request.reply.send(Ok(json!({"plugins":[]}))).unwrap();
    assert_eq!(
        call.complete(Duration::from_secs(1)).await.unwrap(),
        json!({"plugins":[]})
    );
}
#[tokio::test]
async fn management_bridge_denies_arbitrary_rpc_and_bounds_admission() {
    let router = Arc::new(Mutex::new(Router::default()));
    let source = peer(&router, "web", 1, 4);
    let (tx, mut rx) = mpsc::channel(1);
    router.lock().unwrap().management = Some(tx);
    for method in [
        "plugins.call",
        "actions.call",
        "services.call",
        "config.resolved",
        "shell.exec",
        "unknown",
    ] {
        assert_eq!(
            failure(manage(
                &router,
                &source.identity,
                &["management.read", "management.write"],
                method,
                json!({"method":"send_private_msg"})
            )),
            ErrorCode::PermissionDenied
        );
    }
    assert_eq!(
        failure(manage(
            &router,
            &source.identity,
            &["management.read"],
            "config.get",
            json!(null)
        )),
        ErrorCode::InvalidRequest
    );
    let first = manage(
        &router,
        &source.identity,
        &["management.write"],
        "plugins.stop",
        json!({"id":"web"}),
    )
    .unwrap();
    assert_eq!(
        failure(manage(
            &router,
            &source.identity,
            &["management.read"],
            "plugins.list",
            json!({})
        )),
        ErrorCode::Overloaded
    );
    let request = rx.recv().await.unwrap();
    assert!(request.is_active());
    drop(first);
    assert!(!request.is_active());
}
#[tokio::test]
async fn management_queued_leases_revoke_on_quiesce_retirement_and_generation_replacement() {
    let router = Arc::new(Mutex::new(Router::default()));
    let source = peer(&router, "web", 1, 4);
    let (tx, mut rx) = mpsc::channel(4);
    router.lock().unwrap().management = Some(tx);
    let first = manage(
        &router,
        &source.identity,
        &["management.read"],
        "config.get",
        json!({"id":"web"}),
    )
    .unwrap();
    let request = rx.recv().await.unwrap();
    assert!(request.is_active());
    router.lock().unwrap().quiesce("web");
    assert!(!request.is_active());
    assert_eq!(
        failure(manage(
            &router,
            &source.identity,
            &["management.read"],
            "plugins.list",
            json!({})
        )),
        ErrorCode::Offline
    );
    drop(first);
    let second_source = peer(&router, "web", 2, 4);
    let second = manage(
        &router,
        &second_source.identity,
        &["management.read"],
        "plugins.list",
        json!({}),
    )
    .unwrap();
    let second_request = rx.recv().await.unwrap();
    assert!(second_request.is_active());
    let third_source = peer(&router, "web", 3, 4);
    assert!(!second_request.is_active());
    assert!(!router.lock().unwrap().is_active(&source.identity));
    assert!(router.lock().unwrap().is_active(&third_source.identity));
    drop(second);
    let third = manage(
        &router,
        &third_source.identity,
        &["management.read"],
        "plugins.list",
        json!({}),
    )
    .unwrap();
    let third_request = rx.recv().await.unwrap();
    drop(third_source);
    assert!(!third_request.is_active());
    drop(third);
}
#[tokio::test]
async fn management_deadlines_cancel_queued_admission_and_never_retry() {
    let router = Arc::new(Mutex::new(Router::default()));
    let source = peer(&router, "web", 1, 4);
    assert_eq!(
        failure(manage(
            &router,
            &source.identity,
            &["management.read"],
            "plugins.list",
            json!({})
        )),
        ErrorCode::Unsupported
    );
    let (tx, mut rx) = mpsc::channel(1);
    router.lock().unwrap().management = Some(tx);
    let Dispatch::Forward(call) = manage(
        &router,
        &source.identity,
        &["management.write"],
        "plugins.reload",
        json!({"id":"web"}),
    )
    .unwrap() else {
        panic!()
    };
    let request = rx.recv().await.unwrap();
    assert_eq!(
        call.complete(Duration::from_millis(1))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Timeout
    );
    assert!(!request.is_active());
    assert!(rx.try_recv().is_err());
}
