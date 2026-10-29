//! Independent subprocess + loopback Universal WebSocket contracts.
//! These tests establish TwinTail behavior against a mock, not SnowLuma compatibility.
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    time::{sleep, timeout},
};
use tokio_tungstenite::{WebSocketStream, accept_async, tungstenite::Message};
use twintail_core::{Limits, Manifest, PluginSpec, Supervisor};
use twintail_plugin_sdk::ConfigSnapshot;

const ADAPTER: &str = "adapter-snowluma";
const SELF_ID: &str = "9007199254740993";
const DEADLINE: Duration = Duration::from_secs(5);
type Socket = WebSocketStream<TcpStream>;
// These subprocess fixtures share the OS ephemeral port pool. A reverse port
// reservation cannot be transferred to the child; concurrent forward clients
// could claim the released port before the child binds it.
static TRANSPORT_FIXTURES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn spec(adapter: bool) -> PluginSpec {
    let (id, binary, grants) = if adapter {
        (
            ADAPTER,
            env!("CARGO_BIN_EXE_adapter-snowluma"),
            vec!["bots.register", "events.publish", "services.provide"],
        )
    } else {
        (
            "echo-plugin",
            env!("CARGO_BIN_EXE_p4-echo-fixture"),
            vec!["events.subscribe", "actions.call"],
        )
    };
    let executable = Path::new(binary).canonicalize().unwrap();
    let mut manifest: Manifest = toml::from_str(&format!(
        "id='{id}'\nname='{id}'\nversion='0.1.0'\nprotocol_version=1\nhost_api='^0.1'\nentrypoint='fixture'\nreload_policy='exclusive'\n"
    )).unwrap();
    manifest.capabilities = grants.iter().map(|v| (*v).to_owned()).collect();
    manifest.required_capabilities = manifest.capabilities.clone();
    PluginSpec {
        package_dir: executable.parent().unwrap().to_owned(),
        executable,
        grants: manifest.capabilities.clone(),
        manifest,
    }
}
async fn setup(listener: &TcpListener) -> (tempfile::TempDir, Supervisor) {
    let data = tempfile::tempdir().unwrap();
    let mut host = Supervisor::new(
        vec![spec(true), spec(false)],
        data.path().to_owned(),
        Limits {
            handshake: Duration::from_secs(3),
            request: Duration::from_secs(3),
            stop: Duration::from_secs(2),
        },
    )
    .unwrap();
    host.set_config(
        ADAPTER,
        Some(ConfigSnapshot {
            revision: 1,
            config_version: 1,
            values: json!({"url":format!("ws://{}/onebot/v11/ws",listener.local_addr().unwrap()),
            "self_id":SELF_ID,"access_token":"loopback-test-token",
            "reconnect_initial_ms":20,"reconnect_max_ms":100,
            "ping_interval_ms":30000,"pong_timeout_ms":30000}),
        }),
    )
    .unwrap();
    host.start("echo-plugin").await.unwrap();
    assert_eq!(
        host.call("echo-plugin", "echo.ready".into(), json!({}))
            .await
            .unwrap()["ready"],
        true
    );
    host.start(ADAPTER).await.unwrap();
    (data, host)
}
async fn accept(listener: &TcpListener) -> Socket {
    timeout(DEADLINE, async {
        let (stream, _) = listener.accept().await.unwrap();
        accept_async(stream).await.unwrap()
    })
    .await
    .expect("adapter must connect to mock Universal endpoint")
}
async fn connected(host: &mut Supervisor) -> Value {
    timeout(DEADLINE, async {
        loop {
            let status = host
                .call(ADAPTER, "connection.status".into(), json!({}))
                .await
                .unwrap();
            if status["connected"] == true {
                break status;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("IPC active must become externally connected")
}
async fn drained(host: &mut Supervisor) {
    // Shorter than the configured 3s action deadline: timeout cleanup must not
    // masquerade as successful response correlation.
    timeout(Duration::from_secs(1), async {
        loop {
            let status = host
                .call(ADAPTER, "connection.status".into(), json!({}))
                .await
                .unwrap();
            if status["pending"] == 0 {
                assert_eq!(
                    status["rejected_frames"], 0,
                    "matching response must be accepted"
                );
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("matching action response must release pending correlation slot");
}
async fn exchange<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    number: u64,
) -> Value {
    let segments = if number == 1 {
        json!([
            {"type":"text","data":{"text":format!("loopback message {number}")}},
            {"type":"vendor.extension","data":{"opaque":[true,null,"9007199254740993"]}}
        ])
    } else {
        json!("reconnected [CQ:image,file=opaque,extension=preserved]")
    };
    socket
        .send(Message::Text(
            json!({
                "time":1,"self_id":SELF_ID,"post_type":"message","message_type":"group",
                "sub_type":"normal","message_id":number,"group_id":"9007199254740995",
                "user_id":"9007199254740997","message":segments,"raw_message":"unmodified",
                "sender":{"user_id":"9007199254740997"},"extension":{"round":number}
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    timeout(DEADLINE, async {
        loop {
            match socket
                .next()
                .await
                .expect("adapter must keep socket open")
                .unwrap()
            {
                Message::Text(text) => {
                    let action: Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(action["action"], "send_msg");
                    assert_eq!(
                        action["params"],
                        json!({"message":segments,"message_type":"group",
                        "group_id":"9007199254740995","user_id":"9007199254740997"})
                    );
                    assert!(
                        !action["echo"].is_null(),
                        "wire action needs correlation echo"
                    );
                    socket
                        .send(Message::Text(
                            json!({"status":"ok","retcode":0,
                        "data":{"message_id":number+100},"echo":action["echo"],
                        "vendor_extension":{"accepted":true}})
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    break action;
                }
                Message::Ping(data) => socket.send(Message::Pong(data)).await.unwrap(),
                other => panic!("unexpected frame while waiting for echo action: {other:?}"),
            }
        }
    })
    .await
    .expect("WS event must traverse host and echo SDK process back to WS action")
}
async fn closed<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
) {
    timeout(DEADLINE, async {
        loop {
            match socket.next().await {
                None | Some(Ok(Message::Close(_))) => break,
                Some(Ok(Message::Ping(data))) => socket.send(Message::Pong(data)).await.unwrap(),
                other => panic!("old adapter socket must close cleanly: {other:?}"),
            }
        }
    })
    .await
    .expect("exclusive replacement must close old WebSocket")
}

#[tokio::test]
async fn message_echo_roundtrip_survives_peer_disconnect_without_restarting_plugin() {
    let _fixture = TRANSPORT_FIXTURES.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (_data, mut host) = setup(&listener).await;
    let mut socket = accept(&listener).await;
    let first_connection = connected(&mut host).await;
    let generation = host
        .list()
        .into_iter()
        .find(|status| status.id == ADAPTER)
        .unwrap()
        .generation;
    exchange(&mut socket, 1).await;
    drained(&mut host).await;
    socket.close(None).await.unwrap();
    closed(&mut socket).await;
    drop(socket);
    let mut replacement = accept(&listener).await;
    let second_connection = connected(&mut host).await;
    assert!(
        second_connection["generation"].as_u64().unwrap()
            > first_connection["generation"].as_u64().unwrap()
    );
    exchange(&mut replacement, 2).await;
    drained(&mut host).await;
    assert_eq!(
        host.list()
            .into_iter()
            .find(|status| status.id == ADAPTER)
            .unwrap()
            .generation,
        generation,
        "transport reconnect must not require restarting the SDK plugin process"
    );
    host.shutdown().await;
    closed(&mut replacement).await;
}

#[tokio::test]
async fn exclusive_reload_closes_old_socket_before_new_connection_and_keeps_echo_routing() {
    let _fixture = TRANSPORT_FIXTURES.lock().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (_data, mut host) = setup(&listener).await;
    let mut socket = accept(&listener).await;
    connected(&mut host).await;
    exchange(&mut socket, 1).await;
    drained(&mut host).await;
    let first = host
        .list()
        .into_iter()
        .find(|status| status.id == ADAPTER)
        .unwrap();
    let echo = host
        .list()
        .into_iter()
        .find(|status| status.id == "echo-plugin")
        .unwrap();
    let replacement = tokio::spawn(async move {
        // A new connection before old close violates exclusive transport ownership.
        tokio::select! {
            biased;
            _ = closed(&mut socket) => {},
            connection = listener.accept() => panic!("new socket arrived before old socket closed: {connection:?}"),
        }
        accept(&listener).await
    });
    let outcome = host.reload(ADAPTER, spec(true)).await.unwrap();
    assert_eq!(outcome["outcome"], "reloaded");
    let mut socket = timeout(DEADLINE, replacement).await.unwrap().unwrap();
    connected(&mut host).await;
    let next = host
        .list()
        .into_iter()
        .find(|status| status.id == ADAPTER)
        .unwrap();
    assert!(next.generation > first.generation);
    assert_eq!(
        host.list()
            .into_iter()
            .find(|status| status.id == "echo-plugin")
            .unwrap()
            .generation,
        echo.generation
    );
    exchange(&mut socket, 2).await;
    drained(&mut host).await;
    host.shutdown().await;
    closed(&mut socket).await;
}

#[tokio::test]
async fn reverse_subprocess_echo_and_exclusive_reload_rebind_same_listener() {
    let _fixture = TRANSPORT_FIXTURES.lock().await;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let reserve = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = reserve.local_addr().unwrap();
    let data = tempfile::tempdir().unwrap();
    let mut host = Supervisor::new(
        vec![spec(true), spec(false)],
        data.path().to_owned(),
        Limits {
            handshake: Duration::from_secs(3),
            request: Duration::from_secs(3),
            stop: Duration::from_secs(2),
        },
    )
    .unwrap();
    host.set_config(ADAPTER,Some(ConfigSnapshot{config_version:1,revision:1,values:json!({"transport":"reverse_ws","self_id":SELF_ID,"reverse_bind":address.to_string(),"access_token":"synthetic-token","ping_interval_ms":30000,"pong_timeout_ms":30000})})).unwrap();
    host.start("echo-plugin").await.unwrap();
    host.call("echo-plugin", "echo.ready".into(), json!({}))
        .await
        .unwrap();
    // Release only after unrelated subprocess setup is complete.
    drop(reserve);
    host.start(ADAPTER).await.unwrap();
    let connect = || async {
        let mut request = format!("ws://{address}/onebot/v11")
            .into_client_request()
            .unwrap();
        for (name, value) in [
            ("x-self-id", SELF_ID),
            ("x-client-role", "Universal"),
            ("authorization", "Bearer synthetic-token"),
        ] {
            request.headers_mut().insert(name, value.parse().unwrap());
        }
        tokio_tungstenite::connect_async(request).await.unwrap().0
    };
    let mut socket = connect().await;
    connected(&mut host).await;
    exchange(&mut socket, 10).await;
    drained(&mut host).await;
    let generation = host
        .list()
        .into_iter()
        .find(|v| v.id == ADAPTER)
        .unwrap()
        .generation;
    let outcome = host.reload(ADAPTER, spec(true)).await.unwrap();
    assert_eq!(outcome["outcome"], "reloaded");
    closed(&mut socket).await;
    assert!(
        host.list()
            .into_iter()
            .find(|v| v.id == ADAPTER)
            .unwrap()
            .generation
            > generation
    );
    let mut replacement = connect().await;
    connected(&mut host).await;
    exchange(&mut replacement, 11).await;
    drained(&mut host).await;
    host.shutdown().await;
    closed(&mut replacement).await;
    let _rebound = TcpListener::bind(address).await.unwrap();
}

#[tokio::test]
async fn reverse_subprocess_occupied_listener_fails_start_without_claiming_ready() {
    let _fixture = TRANSPORT_FIXTURES.lock().await;
    let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let data = tempfile::tempdir().unwrap();
    let mut host = Supervisor::new(
        vec![spec(true)],
        data.path().to_owned(),
        Limits {
            handshake: Duration::from_secs(3),
            request: Duration::from_secs(3),
            stop: Duration::from_secs(2),
        },
    )
    .unwrap();
    host.set_config(ADAPTER,Some(ConfigSnapshot{config_version:1,revision:1,values:json!({"transport":"reverse_ws","self_id":SELF_ID,"reverse_bind":reserved.local_addr().unwrap().to_string()})})).unwrap();
    let failure = host.start(ADAPTER).await.unwrap_err();
    assert_eq!(failure.code, twintail_plugin_sdk::ErrorCode::PluginExited);
    assert_eq!(failure.message, "plugin pipe closed or invalid frame");
    let status = host
        .list()
        .into_iter()
        .find(|status| status.id == ADAPTER)
        .unwrap();
    assert_eq!(status.state, "failed");
    host.shutdown().await;
}
