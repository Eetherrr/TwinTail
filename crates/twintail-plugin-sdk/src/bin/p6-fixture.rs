//! Internal bounded-load fixture. Only synthetic mock traffic; never QQ.
#[path = "../../../../examples/echo-plugin/src/main.rs"]
mod echo;
#[path = "../../../../examples/mock-adapter/src/main.rs"]
mod mock;
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if let Some(path) = args.get(2) {
        std::fs::write(path, std::process::id().to_string()).unwrap();
    }
    match args.get(1).map(String::as_str) {
        Some("mock") => mock::main(),
        Some("echo") => echo::main(),
        Some("slow") => slow(),
        Some("optional") => optional(),
        Some("fail-start") => fail_start(),
        _ => std::process::exit(2),
    }
}
#[tokio::main(flavor = "current_thread")]
async fn slow() {
    use serde_json::json;
    use tokio::io::AsyncWriteExt;
    use twintail_protocol::{Body, MAX_FRAME_BYTES, Message, Outcome, read_message, write_message};
    let mut input = tokio::io::stdin();
    let mut output = tokio::io::stdout();
    let mut hello = Message::control(
        "hello",
        json!({"plugin_id":"soak-slow","version":"0.1.0","required_capabilities":["events.subscribe"]}),
    );
    hello.protocol_version.minor = 1;
    write_message(&mut output, &hello, MAX_FRAME_BYTES)
        .await
        .unwrap();
    output.flush().await.unwrap();
    read_message(&mut input, MAX_FRAME_BYTES).await.unwrap();
    read_message(&mut input, MAX_FRAME_BYTES).await.unwrap();
    write_message(
        &mut output,
        &Message::control("ready", json!({})),
        MAX_FRAME_BYTES,
    )
    .await
    .unwrap();
    output.flush().await.unwrap();
    read_message(&mut input, MAX_FRAME_BYTES).await.unwrap();
    write_message(
        &mut output,
        &Message::request(
            "subscribe",
            "events.subscribe",
            json!({"event_types":["message"]}),
        ),
        MAX_FRAME_BYTES,
    )
    .await
    .unwrap();
    output.flush().await.unwrap();
    let mut subscribed = false;
    loop {
        let message = read_message(&mut input, MAX_FRAME_BYTES).await.unwrap();
        match message.body {
            Body::Response { request_id, .. } if request_id == "subscribe" => subscribed = true,
            Body::Request {
                request_id, method, ..
            } if method == "slow.park" && subscribed => {
                write_message(
                    &mut output,
                    &Message::response(request_id, Outcome::Ok(json!({"parked":true}))),
                    MAX_FRAME_BYTES,
                )
                .await
                .unwrap();
                output.flush().await.unwrap();
                break;
            }
            Body::Request { request_id, .. } => {
                write_message(
                    &mut output,
                    &Message::response(request_id, Outcome::Ok(json!({"ready":subscribed}))),
                    MAX_FRAME_BYTES,
                )
                .await
                .unwrap();
                output.flush().await.unwrap();
            }
            _ => {}
        }
    }
    // Deliberately never consume the pipe again. Supervisor must bound/drop events
    // and eventually kill/reap this trusted fault fixture during shutdown.
    std::future::pending::<()>().await;
}

#[tokio::main(flavor = "current_thread")]
async fn optional() {
    use serde_json::json;
    use tokio::io::AsyncWriteExt;
    use twintail_protocol::{
        Body, ErrorCode, MAX_FRAME_BYTES, Message, Outcome, RpcError, read_message, write_message,
    };
    let mut input = tokio::io::stdin();
    let mut output = tokio::io::stdout();
    let mut hello = Message::control(
        "hello",
        json!({"plugin_id":"contract-fixture","version":"0.1.0","required_capabilities":[]}),
    );
    hello.protocol_version.minor = 1;
    write_message(&mut output, &hello, MAX_FRAME_BYTES)
        .await
        .unwrap();
    output.flush().await.unwrap();
    read_message(&mut input, MAX_FRAME_BYTES).await.unwrap();
    read_message(&mut input, MAX_FRAME_BYTES).await.unwrap();
    write_message(
        &mut output,
        &Message::control("ready", json!({})),
        MAX_FRAME_BYTES,
    )
    .await
    .unwrap();
    output.flush().await.unwrap();
    read_message(&mut input, MAX_FRAME_BYTES).await.unwrap();
    loop {
        match read_message(&mut input, MAX_FRAME_BYTES)
            .await
            .unwrap()
            .body
        {
            Body::Request { request_id, .. } => {
                write_message(
                    &mut output,
                    &Message::response(
                        request_id,
                        Outcome::Err(RpcError::new(
                            ErrorCode::Unsupported,
                            "optional feature unavailable",
                        )),
                    ),
                    MAX_FRAME_BYTES,
                )
                .await
                .unwrap();
                output.flush().await.unwrap();
            }
            Body::Control { method, .. } if method == "shutdown" => break,
            _ => {}
        }
    }
}
#[tokio::main(flavor = "current_thread")]
async fn fail_start() {
    use twintail_plugin_sdk::*;
    struct Fail;
    impl Plugin for Fail {
        async fn start(&self, _context: Context) -> Result {
            Err(RpcError::new(
                ErrorCode::InvalidConfig,
                "injected start failure",
            ))
        }
    }
    let _ = run_stdio(
        Fail,
        Hello {
            plugin_id: "contract-fixture".into(),
            version: "0.1.0".into(),
            required_capabilities: vec![],
        },
    )
    .await;
}
