//! Reverse Universal WS behavior against synthetic clients; no SnowLuma claim.
use adapter_snowluma::{Config, Transport};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{net::SocketAddr, time::Duration};
use tokio::{
    net::TcpListener,
    time::{sleep, timeout},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Error, Message, client::IntoClientRequest},
};
use twintail_plugin_sdk::{BotRef, ErrorCode};
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
async fn setup(
    extra: Value,
) -> (
    Transport,
    tokio::sync::mpsc::Receiver<twintail_plugin_sdk::PublishEvent>,
    SocketAddr,
) {
    let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = reserved.local_addr().unwrap();
    drop(reserved);
    let mut config = json!({"transport":"reverse_ws","reverse_bind":address.to_string(),"self_id":"10001","action_timeout_ms":100,"connect_timeout_ms":100,"ping_interval_ms":5000});
    config
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let (transport, events) = Transport::spawn(
        Config::from_value(config).unwrap(),
        BotRef {
            adapter_instance_id: "reverse-fixture".into(),
            self_id: "10001".into(),
        },
    )
    .unwrap();
    (transport, events, address)
}
async fn client(
    address: SocketAddr,
    path: &str,
    account: Option<&str>,
    role: Option<&str>,
    token: Option<&str>,
) -> Result<Socket, Error> {
    let mut request = format!("ws://{address}{path}")
        .into_client_request()
        .unwrap();
    for (name, value) in [
        ("x-self-id", account),
        ("x-client-role", role),
        ("authorization", token),
    ] {
        if let Some(value) = value {
            request.headers_mut().insert(name, value.parse().unwrap());
        }
    }
    connect_async(request).await.map(|(socket, _)| socket)
}
async fn online(transport: &Transport, generation: u64) {
    timeout(Duration::from_secs(2), async {
        loop {
            let s = transport.status();
            if s.connected && s.generation >= generation {
                break;
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}
async fn text(socket: &mut Socket) -> Value {
    timeout(Duration::from_secs(1), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(p) => socket.send(Message::Pong(p)).await.unwrap(),
                _ => {}
            }
        }
    })
    .await
    .unwrap()
}
#[track_caller]
fn status(error: Error) -> u16 {
    match error {
        Error::Http(response) => response.status().as_u16(),
        other => panic!("unexpected error {other}"),
    }
}
#[tokio::test]
async fn reverse_auth_account_role_path_and_unique_headers_are_enforced() {
    let (transport, _events, address) = setup(json!({"access_token":"synthetic-secret"})).await;
    assert_eq!(
        status(
            client(
                address,
                "/onebot/v11",
                Some("10001"),
                Some("Universal"),
                None
            )
            .await
            .err()
            .unwrap()
        ),
        401,
        "missing Bearer credential must return HTTP 401"
    );
    for (path, account, role) in [
        ("/onebot/v11", Some("other"), Some("Universal")),
        ("/onebot/v11", None, Some("Universal")),
        ("/onebot/v11", Some("10001"), Some("Event")),
        ("/wrong", Some("10001"), Some("Universal")),
        ("/onebot/v11?token=x", Some("10001"), Some("Universal")),
    ] {
        assert_eq!(
            status(
                client(
                    address,
                    path,
                    account,
                    role,
                    Some("Bearer synthetic-secret")
                )
                .await
                .err()
                .unwrap()
            ),
            403,
            "path={path}, account={account:?}, role={role:?} must return HTTP 403"
        );
    }
    for name in ["authorization", "x-self-id", "x-client-role"] {
        let mut request = format!("ws://{address}/onebot/v11")
            .into_client_request()
            .unwrap();
        for (header, value) in [
            ("authorization", "Bearer synthetic-secret"),
            ("x-self-id", "10001"),
            ("x-client-role", "Universal"),
        ] {
            request.headers_mut().insert(header, value.parse().unwrap());
        }
        let duplicate = request.headers().get(name).unwrap().clone();
        request.headers_mut().append(name, duplicate);
        let failure = connect_async(request).await.err().unwrap();
        assert_eq!(
            status(failure),
            if name == "authorization" { 401 } else { 403 },
            "duplicate {name} header must return the exact HTTP rejection"
        );
    }
    let mut socket = client(
        address,
        "/onebot/v11",
        Some("10001"),
        Some("Universal"),
        Some("Bearer synthetic-secret"),
    )
    .await
    .unwrap();
    online(&transport, 1).await;
    socket
        .send(Message::Text(
            json!({"post_type":"message","self_id":99999,"message":"mismatch"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    sleep(Duration::from_millis(10)).await;
    assert!(transport.status().rejected_frames > 0);
    assert_eq!(transport.status().generation, 1);
    transport.stop().await;
}
// When the slot is occupied the actor deliberately does not read the second
// client's request. Dropping a socket with unread data may produce TCP RST on
// Windows; HTTP 409 is best effort. Authentication failures above remain exact
// HTTP assertions and never accept transport errors as authorization evidence.
#[tokio::test]
async fn reverse_duplicate_is_rejected_and_original_session_keeps_action_ownership() {
    let (transport, _events, address) = setup(json!({"access_token":"synthetic-secret"})).await;
    let mut original = client(
        address,
        "/onebot/v11",
        Some("10001"),
        Some("Universal"),
        Some("Bearer synthetic-secret"),
    )
    .await
    .unwrap();
    online(&transport, 1).await;
    let duplicate = timeout(
        Duration::from_secs(1),
        client(
            address,
            "/onebot/v11",
            Some("10001"),
            Some("Universal"),
            Some("Bearer synthetic-secret"),
        ),
    )
    .await
    .expect("duplicate must be rejected promptly");
    match duplicate {
        Err(Error::Http(response)) => assert_eq!(response.status().as_u16(), 409),
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
        Err(other) => panic!("unexpected duplicate rejection: {other}"),
        Ok(_) => panic!("second connection must not complete a WebSocket upgrade"),
    }
    // A reset alone does not establish exclusive ownership: prove that the
    // authenticated original socket still receives and completes host actions.
    let action_transport = transport.clone();
    let action = tokio::spawn(async move {
        action_transport
            .action("get_status".into(), json!({}))
            .await
    });
    let request = text(&mut original).await;
    assert_eq!(request["action"], "get_status");
    original
        .send(Message::Text(
            json!({"status":"ok","retcode":0,"data":{"owner":"original"},"echo":request["echo"]})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(action.await.unwrap().unwrap()["data"]["owner"], "original");
    assert!(transport.status().connected);
    assert_eq!(transport.status().generation, 1);
    transport.stop().await;
}
#[tokio::test]
async fn reverse_actions_correlate_timeout_disconnect_and_reconnect_without_replay() {
    let (transport, mut events, address) = setup(json!({})).await;
    assert_eq!(
        transport
            .action("get_status".into(), json!({}))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Offline
    );
    let mut socket = client(
        address,
        "/onebot/v11",
        Some("10001"),
        Some("Universal"),
        None,
    )
    .await
    .unwrap();
    online(&transport, 1).await;
    socket.send(Message::Text(json!({"post_type":"meta_event","meta_event_type":"heartbeat","self_id":10001,"time":1,"extension":{"nested":[1,"x"]}}).to_string().into())).await.unwrap();
    let event = timeout(Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.payload["extension"], json!({"nested":[1,"x"]}));
    let a = transport.clone();
    let first = tokio::spawn(async move { a.action("get_status".into(), json!({})).await });
    let a = transport.clone();
    let second = tokio::spawn(async move { a.action("get_version_info".into(), json!({})).await });
    let one = text(&mut socket).await;
    let two = text(&mut socket).await;
    for request in [&two, &one] {
        socket.send(Message::Text(json!({"status":"ok","retcode":0,"data":{"action":request["action"]},"echo":request["echo"]}).to_string().into())).await.unwrap();
    }
    assert_eq!(
        first.await.unwrap().unwrap()["data"]["action"],
        "get_status"
    );
    assert_eq!(
        second.await.unwrap().unwrap()["data"]["action"],
        "get_version_info"
    );
    let a = transport.clone();
    let timed = tokio::spawn(async move { a.action("send_msg".into(), json!({})).await });
    let stale = text(&mut socket).await;
    assert_eq!(
        timed.await.unwrap().unwrap_err().code,
        ErrorCode::UnknownOutcome
    );
    let a = transport.clone();
    let disconnected = tokio::spawn(async move { a.action("send_msg".into(), json!({})).await });
    let _ = text(&mut socket).await;
    drop(socket);
    assert_eq!(
        disconnected.await.unwrap().unwrap_err().code,
        ErrorCode::UnknownOutcome
    );
    timeout(Duration::from_secs(1), async {
        while transport.status().connected {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let mut socket = client(
        address,
        "/onebot/v11",
        Some("10001"),
        Some("Universal"),
        None,
    )
    .await
    .unwrap();
    online(&transport, 2).await;
    socket
        .send(Message::Text(
            json!({"status":"ok","retcode":0,"data":{},"echo":stale["echo"]})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert!(
        timeout(Duration::from_millis(30), socket.next())
            .await
            .is_err(),
        "admitted actions must not replay"
    );
    let a = transport.clone();
    let fresh = tokio::spawn(async move { a.action("get_status".into(), json!({})).await });
    let frame = text(&mut socket).await;
    assert_ne!(frame["echo"], stale["echo"]);
    socket
        .send(Message::Text(
            json!({"status":"ok","retcode":0,"data":{},"echo":frame["echo"]})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    fresh.await.unwrap().unwrap();
    transport.stop().await;
    assert_eq!(transport.status().phase, "stopped");
    let rebound = TcpListener::bind(address).await.unwrap();
    drop(rebound);
}
#[tokio::test]
async fn reverse_listener_shutdown_interrupts_idle_and_slow_handshake_and_conflicts_fail_start() {
    let (transport, _, address) = setup(json!({"connect_timeout_ms":10000})).await;
    let config = Config::from_value(
        json!({"transport":"reverse_ws","reverse_bind":address.to_string(),"self_id":"10001"}),
    )
    .unwrap();
    let error = Transport::spawn(
        config,
        BotRef {
            adapter_instance_id: "other".into(),
            self_id: "10001".into(),
        },
    )
    .err()
    .unwrap();
    assert_eq!(error.code, ErrorCode::InvalidConfig);
    let _slow = tokio::net::TcpStream::connect(address).await.unwrap();
    sleep(Duration::from_millis(5)).await;
    timeout(Duration::from_millis(500), transport.stop())
        .await
        .unwrap();
    let rebound = TcpListener::bind(address).await.unwrap();
    drop(rebound);
    let (idle, _, address) = setup(json!({})).await;
    timeout(Duration::from_millis(500), idle.stop())
        .await
        .unwrap();
    let _rebound = TcpListener::bind(address).await.unwrap();
}
#[test]
fn reverse_remote_binding_and_ambiguous_endpoint_are_rejected() {
    for extra in [
        json!({"reverse_bind":"0.0.0.0:17281"}),
        json!({"reverse_bind":"0.0.0.0:17281","access_token":"token"}),
        json!({"reverse_bind":"localhost:17281"}),
        json!({"url":"ws://localhost:80"}),
        json!({"reverse_path":"/onebot?token=x"}),
    ] {
        let mut value = json!({"transport":"reverse_ws","self_id":"10001"});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert_eq!(
            Config::from_value(value).err().unwrap().code,
            ErrorCode::InvalidConfig
        );
    }
    assert!(Config::from_value(json!({"url":"ws://localhost:80","self_id":"10001"})).is_ok());
}

#[test]
fn packaged_schema_accepts_legacy_forward_and_reverse_and_requires_forward_endpoint() {
    let data = tempfile::tempdir().unwrap();
    let store = twintail_core::ConfigStore::open(
        data.path(),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
        Some("config.schema.json"),
        1,
    )
    .unwrap();
    for value in [
        json!({"self_id":"10001","url":"ws://localhost:80"}),
        json!({"self_id":"10001","transport":"forward_ws","url":"wss://localhost:443/ws"}),
        json!({"self_id":"10001","transport":"reverse_ws"}),
        json!({"self_id":"10001","transport":"reverse_ws","reverse_bind":"127.0.0.1:17281","reverse_path":"/onebot/v11"}),
    ] {
        store.validate(&value).unwrap();
        Config::from_value(value).unwrap();
    }
    for value in [
        json!({"self_id":"10001"}),
        json!({"self_id":"10001","transport":"forward_ws"}),
        json!({"self_id":"10001","transport":"reverse_ws","url":"ws://localhost:80"}),
        json!({"self_id":"10001","transport":"split_ws"}),
    ] {
        assert!(store.validate(&value).is_err());
    }
}

#[tokio::test]
async fn reverse_handshake_and_pong_deadlines_release_slot_for_new_generation() {
    let (transport, _, address) =
        setup(json!({"connect_timeout_ms":30,"ping_interval_ms":20,"pong_timeout_ms":30})).await;
    let _slow = tokio::net::TcpStream::connect(address).await.unwrap();
    sleep(Duration::from_millis(60)).await;
    let _unpolled = client(
        address,
        "/onebot/v11",
        Some("10001"),
        Some("Universal"),
        None,
    )
    .await
    .unwrap();
    online(&transport, 1).await;
    timeout(Duration::from_secs(1), async {
        loop {
            let status = transport.status();
            if !status.connected && status.generation == 1 {
                break;
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let _replacement = client(
        address,
        "/onebot/v11",
        Some("10001"),
        Some("Universal"),
        None,
    )
    .await
    .unwrap();
    online(&transport, 2).await;
    transport.stop().await;
}
