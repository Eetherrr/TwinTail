//! Deterministic loopback WebSocket fault and resource-bound tests.
use adapter_snowluma::{Config, Transport};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::{sleep, timeout},
};
use tokio_tungstenite::{
    WebSocketStream, accept_async, accept_hdr_async,
    tungstenite::{
        Message,
        handshake::server::{Request, Response},
        http::{Response as HttpResponse, StatusCode},
    },
};
use twintail_plugin_sdk::{BotRef, ErrorCode};
struct CapturedLogs(std::sync::Mutex<Vec<String>>);
static LOGS: CapturedLogs = CapturedLogs(std::sync::Mutex::new(Vec::new()));
impl log::Log for CapturedLogs {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        self.0.lock().unwrap().push(record.args().to_string());
    }
    fn flush(&self) {}
}
fn config(address: std::net::SocketAddr, extra: Value) -> Config {
    let mut value = json!({"url":format!("ws://{address}/onebot"),"self_id":"10001","action_timeout_ms":100,"connect_timeout_ms":100,"reconnect_initial_ms":20,"reconnect_max_ms":80,"ping_interval_ms":5000,"pong_timeout_ms":1000});
    value
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    Config::from_value(value).unwrap()
}
fn bot() -> BotRef {
    BotRef {
        adapter_instance_id: "loopback-instance".into(),
        self_id: "10001".into(),
    }
}
async fn connected(transport: &Transport, generation: u64) {
    timeout(Duration::from_secs(3), async {
        while !transport.status().connected || transport.status().generation < generation {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}
async fn action_frame(socket: &mut WebSocketStream<TcpStream>) -> Value {
    loop {
        match socket.next().await.unwrap().unwrap() {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
            other => panic!("unexpected WS frame {other:?}"),
        }
    }
}
async fn response(socket: &mut WebSocketStream<TcpStream>, echo: Value, data: Value) {
    socket
        .send(Message::Text(
            json!({"status":"ok","retcode":0,"data":data,"echo":echo})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
}
#[test]
fn configuration_bounds_and_credentials_are_conservative() {
    for value in [
        json!({"url":"http://localhost/","self_id":"10001"}),
        json!({"url":"ws://user:secret@localhost/","self_id":"10001"}),
        json!({"url":"ws://localhost/?access_token=secret","self_id":"10001"}),
        json!({"url":"ws://localhost/#secret","self_id":"10001"}),
        json!({"url":"ws://localhost/","self_id":"01"}),
        json!({"url":"ws://localhost/","self_id":"10001","access_token":"secret\r\nInjected: yes"}),
        json!({"url":"ws://localhost/","self_id":"10001","max_pending":0}),
        json!({"url":"ws://localhost/","self_id":"10001","extra":"secret"}),
    ] {
        let err = Config::from_value(value).err().unwrap();
        assert_eq!(err.code, ErrorCode::InvalidConfig);
        assert!(!err.to_string().contains("secret"));
    }
}
// tungstenite fixes the callback error type to an unboxed HTTP response.
#[allow(clippy::result_large_err)]
#[tokio::test]
async fn authorization_bearer_structural_events_out_of_order_and_remote_failure() {
    log::set_logger(&LOGS).unwrap();
    log::set_max_level(log::LevelFilter::Trace);
    assert!(log::STATIC_MAX_LEVEL <= log::LevelFilter::Info);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = config(
        listener.local_addr().unwrap(),
        json!({"access_token":"fixture-credential"}),
    );
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_hdr_async(stream, |request: &Request, response: Response| {
            assert_eq!(request.uri().path(), "/onebot");
            assert!(request.uri().query().is_none());
            assert_eq!(
                request.headers()["authorization"],
                "Bearer fixture-credential"
            );
            Ok(response)
        })
        .await
        .unwrap();
        let first = action_frame(&mut ws).await;
        let second = action_frame(&mut ws).await;
        assert_ne!(first["echo"], second["echo"]);
        assert!(first["echo"].is_string());
        ws.send(Message::Text(json!({"post_type":"message","self_id":10001,"echo":first["echo"],"message":[{"type":"text","data":{"text":"hello"}},{"type":"extension","data":{"x":1}}],"extension":{"keep":true}}).to_string().into())).await.unwrap();
        response(
            &mut ws,
            second["echo"].clone(),
            json!({"action":second["action"]}),
        )
        .await;
        ws.send(Message::Text(json!({"status":"failed","retcode":1400,"data":{"keep":true},"echo":first["echo"],"wording":"remote failure"}).to_string().into())).await.unwrap();
        // Stay alive until stop closes the client socket.
        while let Some(Ok(message)) = ws.next().await {
            if message.is_close() {
                break;
            }
        }
    });
    let (transport, mut events) = Transport::spawn(cfg, bot()).unwrap();
    connected(&transport, 1).await;
    let (first, second) = tokio::join!(
        transport.action("first".into(), json!({})),
        transport.action("second".into(), json!({}))
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first["retcode"], 1400);
    assert_eq!(first["data"]["keep"], true);
    assert_eq!(first["wording"], "remote failure");
    assert_eq!(second["data"]["action"], "second");
    let event = timeout(Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.event_type, "message");
    assert_eq!(event.protocol, "onebot.v11");
    assert_eq!(event.payload["extension"]["keep"], true);
    assert_eq!(event.payload["message"].as_array().unwrap().len(), 2);
    let status = serde_json::to_string(&transport.status()).unwrap();
    assert!(!status.contains("fixture-credential"));
    assert!(!status.contains("ws://"));
    assert!(
        !LOGS
            .0
            .lock()
            .unwrap()
            .join("\n")
            .contains("fixture-credential")
    );
    transport.stop().await;
    server.await.unwrap();
    assert_eq!(transport.status().pending, 0);
}
#[tokio::test]
async fn timeout_late_reply_bounded_pending_and_disconnect_never_replay() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = config(
        listener.local_addr().unwrap(),
        json!({"max_pending":1,"action_timeout_ms":40}),
    );
    let (seen_tx, seen_rx) = oneshot::channel();
    let (close_tx, close_rx) = oneshot::channel();
    let (new_tx, new_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        let old = action_frame(&mut ws).await;
        seen_tx.send(()).unwrap();
        sleep(Duration::from_millis(70)).await;
        response(&mut ws, old["echo"].clone(), json!({"late":true})).await;
        close_rx.await.unwrap();
        let admitted = action_frame(&mut ws).await;
        assert_eq!(admitted["action"], "disconnect");
        drop(ws);
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        new_tx.send(()).unwrap();
        let fresh = action_frame(&mut ws).await;
        assert_eq!(fresh["action"], "fresh");
        assert_ne!(fresh["echo"], old["echo"]);
        response(&mut ws, fresh["echo"].clone(), json!({"fresh":true})).await;
        while let Some(Ok(message)) = ws.next().await {
            if message.is_close() {
                break;
            }
        }
    });
    let (transport, _events) = Transport::spawn(cfg, bot()).unwrap();
    connected(&transport, 1).await;
    let clone = transport.clone();
    let first = tokio::spawn(async move { clone.action("timeout".into(), json!({})).await });
    seen_rx.await.unwrap();
    assert_eq!(
        transport
            .action("overloaded".into(), json!({}))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Overloaded
    );
    assert_eq!(
        first.await.unwrap().unwrap_err().code,
        ErrorCode::UnknownOutcome
    );
    sleep(Duration::from_millis(80)).await;
    assert_eq!(transport.status().pending, 0);
    assert!(transport.status().rejected_frames >= 1);
    close_tx.send(()).unwrap();
    assert_eq!(
        transport
            .action("disconnect".into(), json!({}))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UnknownOutcome
    );
    new_rx.await.unwrap();
    connected(&transport, 2).await;
    assert_eq!(
        transport.action("fresh".into(), json!({})).await.unwrap()["data"]["fresh"],
        true
    );
    transport.stop().await;
    server.await.unwrap();
}
// tungstenite fixes the callback error type to an unboxed HTTP response.
#[allow(clippy::result_large_err)]
#[tokio::test]
async fn authentication_failure_is_redacted_and_reconnect_backoff_is_bounded() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = config(
        listener.local_addr().unwrap(),
        json!({"access_token":"fixture-auth-secret","reconnect_initial_ms":40,"reconnect_max_ms":80}),
    );
    let server = tokio::spawn(async move {
        let mut instants = Vec::new();
        for _ in 0..3 {
            let (stream, _) = listener.accept().await.unwrap();
            instants.push(tokio::time::Instant::now());
            let _ = accept_hdr_async(stream, |_: &Request, _: Response| {
                Err(HttpResponse::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body(Some("fixture-auth-secret".into()))
                    .unwrap())
            })
            .await;
        }
        assert!(instants[1].duration_since(instants[0]) >= Duration::from_millis(35));
        assert!(instants[2].duration_since(instants[1]) >= Duration::from_millis(70));
    });
    let (transport, _) = Transport::spawn(cfg, bot()).unwrap();
    timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    sleep(Duration::from_millis(5)).await;
    assert_eq!(transport.status().last_error, Some("authentication_failed"));
    assert_eq!(
        transport
            .action("send_msg".into(), json!({}))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Offline
    );
    assert!(
        !serde_json::to_string(&transport.status())
            .unwrap()
            .contains("fixture-auth-secret")
    );
    transport.stop().await;
}
#[tokio::test]
async fn event_overflow_does_not_block_actions_and_stop_remains_joinable_after_cancellation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = config(
        listener.local_addr().unwrap(),
        json!({"event_queue":1,"action_timeout_ms":1000}),
    );
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        for n in 0..300 {
            ws.send(Message::Text(
                json!({"post_type":"notice","self_id":10001,"n":n})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        }
        let frame = action_frame(&mut ws).await;
        response(&mut ws, frame["echo"].clone(), json!({"received":true})).await;
        while let Some(Ok(message)) = ws.next().await {
            if message.is_close() {
                break;
            }
        }
    });
    let (transport, _unread) = Transport::spawn(cfg, bot()).unwrap();
    connected(&transport, 1).await;
    assert_eq!(
        transport.action("query".into(), json!({})).await.unwrap()["data"]["received"],
        true
    );
    timeout(Duration::from_secs(1), async {
        while transport.status().dropped_events == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    {
        let stop = transport.stop();
        tokio::pin!(stop);
        assert!(futures_util::poll!(&mut stop).is_pending());
    }
    transport.stop().await;
    assert_eq!(transport.status().phase, "stopped");
    assert!(!transport.status().connected);
    assert_eq!(transport.status().pending, 0);
    server.await.unwrap();
}
#[tokio::test]
async fn heartbeat_does_not_substitute_for_matching_websocket_pong() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = config(
        listener.local_addr().unwrap(),
        json!({"ping_interval_ms":10,"pong_timeout_ms":20,"reconnect_initial_ms":1000,"reconnect_max_ms":1000}),
    );
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        for _ in 0..10 {
            if ws
                .send(Message::Text(
                    json!({"post_type":"meta_event","meta_event_type":"heartbeat","self_id":10001})
                        .to_string()
                        .into(),
                ))
                .await
                .is_err()
            {
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
    });
    let (transport, _events) = Transport::spawn(cfg, bot()).unwrap();
    connected(&transport, 1).await;
    timeout(Duration::from_secs(1), async {
        while transport.status().last_error != Some("pong_timeout") {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert!(transport.status().heartbeats > 0);
    transport.stop().await;
    server.await.unwrap();
}
#[tokio::test]
async fn malformed_account_binary_and_oversized_frames_are_bounded() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = config(
        listener.local_addr().unwrap(),
        json!({"max_frame_bytes":1024,"reconnect_initial_ms":1000,"reconnect_max_ms":1000}),
    );
    let (ready_tx, ready_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ready_rx.await.unwrap();
        for value in [
            "invalid JSON".to_owned(),
            json!({"post_type":"message","self_id":999,"message":"wrong"}).to_string(),
            json!({"status":"ok","retcode":0,"data":{},"echo":7}).to_string(),
        ] {
            ws.send(Message::Text(value.into())).await.unwrap();
        }
        ws.send(Message::Binary(vec![1, 2, 3].into()))
            .await
            .unwrap();
        ws.send(Message::Text("x".repeat(2048).into()))
            .await
            .unwrap();
    });
    let (transport, mut events) = Transport::spawn(cfg, bot()).unwrap();
    connected(&transport, 1).await;
    ready_tx.send(()).unwrap();
    timeout(Duration::from_secs(1), async {
        while transport.status().rejected_frames < 4 || transport.status().connected {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert!(events.try_recv().is_err());
    transport.stop().await;
    server.await.unwrap();
}

#[tokio::test]
async fn matching_pongs_keep_connection_alive_and_remote_ping_gets_pong() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = config(
        listener.local_addr().unwrap(),
        json!({"ping_interval_ms":15,"pong_timeout_ms":100}),
    );
    let (done_tx, done_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        ws.send(Message::Ping(b"remote-ping".to_vec().into()))
            .await
            .unwrap();
        let mut own_pings = 0;
        let mut received_pong = false;
        while own_pings < 2 || !received_pong {
            match ws.next().await.unwrap().unwrap() {
                Message::Ping(bytes) => {
                    own_pings += 1;
                    ws.send(Message::Pong(bytes)).await.unwrap();
                }
                Message::Pong(bytes) => {
                    if bytes.as_ref() == b"remote-ping" {
                        received_pong = true;
                    }
                }
                other => panic!("unexpected control frame {other:?}"),
            }
        }
        done_tx.send(()).unwrap();
        while let Some(Ok(message)) = ws.next().await {
            if message.is_close() {
                break;
            }
            if let Message::Ping(bytes) = message {
                ws.send(Message::Pong(bytes)).await.unwrap();
            }
        }
    });
    let (transport, _events) = Transport::spawn(cfg, bot()).unwrap();
    timeout(Duration::from_secs(2), done_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(transport.status().connected);
    assert_eq!(transport.status().heartbeats, 0);
    transport.stop().await;
    server.await.unwrap();
}

#[tokio::test]
async fn stop_preempts_stalled_handshake_and_drop_closes_connected_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = config(
        listener.local_addr().unwrap(),
        json!({"connect_timeout_ms":10000}),
    );
    let (transport, _events) = Transport::spawn(cfg, bot()).unwrap();
    let (mut stalled, _) = listener.accept().await.unwrap();
    use tokio::io::AsyncReadExt;
    // TCP acceptance does not prove connect_async has written its HTTP request.
    // Observe the complete Upgrade request before cancelling, while deliberately
    // withholding the response so the actor is known to be inside its handshake.
    let mut request = Vec::new();
    timeout(Duration::from_secs(1), async {
        let mut chunk = [0u8; 1024];
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let count = stalled.read(&mut chunk).await.unwrap();
            assert_ne!(count, 0, "adapter closed before sending Upgrade request");
            request.extend_from_slice(&chunk[..count]);
            assert!(
                request.len() <= 8192,
                "Upgrade request exceeds fixture bound"
            );
        }
    })
    .await
    .expect("mock must observe the Upgrade request before stop");
    assert!(request.starts_with(b"GET /onebot HTTP/1.1\r\n"));
    assert!(
        request
            .windows(b"Upgrade: websocket".len())
            .any(|bytes| bytes.eq_ignore_ascii_case(b"Upgrade: websocket"))
    );
    timeout(Duration::from_millis(250), transport.stop())
        .await
        .unwrap();
    assert_eq!(transport.status().phase, "stopped");
    let mut trailing = Vec::new();
    timeout(
        Duration::from_millis(250),
        stalled.read_to_end(&mut trailing),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(trailing.is_empty(), "no request should be sent after stop");
    let cfg = config(listener.local_addr().unwrap(), json!({}));
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        assert!(ws.next().await.unwrap().unwrap().is_close());
    });
    let (transport, _events) = Transport::spawn(cfg, bot()).unwrap();
    connected(&transport, 1).await;
    drop(transport);
    timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
}
