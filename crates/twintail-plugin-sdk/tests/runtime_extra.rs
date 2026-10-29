//! Failure-path runtime lifecycle contracts.
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncWriteExt, DuplexStream},
    time::timeout,
};
use twintail_plugin_sdk::*;
use twintail_protocol::{Body, MAX_FRAME_BYTES, Message, read_message, write_message};
struct Lifecycle {
    stopped: Arc<AtomicBool>,
    hang: bool,
}
impl Plugin for Lifecycle {
    async fn stop(&self, _: Context) -> Result {
        self.stopped.store(true, Ordering::SeqCst);
        if self.hang {
            std::future::pending().await
        } else {
            Ok(())
        }
    }
}
async fn handshake(host: &mut DuplexStream, minor: u16) {
    assert!(
        matches!(read_message(host,MAX_FRAME_BYTES).await.unwrap().body,Body::Control{method,..} if method=="hello")
    );
    for (method, payload) in [
        ("negotiate", json!({"major":1,"minor":minor})),
        (
            "initialize",
            json!({"instance_id":"test-1","generation":1,"granted_capabilities":[],"data_dir":"/tmp/test","config_revision":1}),
        ),
    ] {
        let mut message = Message::control(method, payload);
        message.protocol_version.minor = minor;
        write_message(host, &message, MAX_FRAME_BYTES)
            .await
            .unwrap();
    }
    assert!(
        matches!(read_message(host,MAX_FRAME_BYTES).await.unwrap().body,Body::Control{method,..} if method=="ready")
    );
    let mut message = Message::control("activate", json!({}));
    message.protocol_version.minor = minor;
    write_message(host, &message, MAX_FRAME_BYTES)
        .await
        .unwrap();
}
fn hello() -> Hello {
    Hello {
        plugin_id: "test".into(),
        version: "0.1.0".into(),
        required_capabilities: vec![],
    }
}
#[tokio::test]
async fn compatible_newer_minor_handshake_and_bounded_stop() {
    let (client, mut host) = tokio::io::duplex(4096);
    let (r, w) = tokio::io::split(client);
    let stopped = Arc::new(AtomicBool::new(false));
    let task = tokio::spawn(run_io(
        Lifecycle {
            stopped: stopped.clone(),
            hang: true,
        },
        hello(),
        r,
        w,
        RuntimeOptions {
            shutdown_timeout: Duration::from_millis(20),
            ..Default::default()
        },
    ));
    handshake(&mut host, 3).await;
    write_message(
        &mut host,
        &Message::control("shutdown", json!({})),
        MAX_FRAME_BYTES,
    )
    .await
    .unwrap();
    let result = timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.unwrap_err().code, ErrorCode::Timeout);
    assert!(stopped.load(Ordering::SeqCst));
}
#[tokio::test]
async fn malformed_frame_retires_transport_and_runs_stop() {
    let (client, mut host) = tokio::io::duplex(4096);
    let (r, w) = tokio::io::split(client);
    let stopped = Arc::new(AtomicBool::new(false));
    let task = tokio::spawn(run_io(
        Lifecycle {
            stopped: stopped.clone(),
            hang: false,
        },
        hello(),
        r,
        w,
        RuntimeOptions::default(),
    ));
    handshake(&mut host, 1).await;
    host.write_all(&[0, 0, 0, 1, b'{']).await.unwrap();
    assert_eq!(
        timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .code,
        ErrorCode::InvalidFrame
    );
    assert!(stopped.load(Ordering::SeqCst));
}
#[tokio::test]
async fn dropping_runtime_aborts_owned_reader_writer_tasks() {
    let (client, mut host) = tokio::io::duplex(4096);
    let (r, w) = tokio::io::split(client);
    let task = tokio::spawn(run_io(
        Lifecycle {
            stopped: Arc::new(AtomicBool::new(false)),
            hang: false,
        },
        hello(),
        r,
        w,
        RuntimeOptions::default(),
    ));
    handshake(&mut host, 1).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    // Owned writer must be dropped, rather than staying alive on its queue.
    assert!(
        timeout(
            Duration::from_secs(1),
            read_message(&mut host, MAX_FRAME_BYTES)
        )
        .await
        .unwrap()
        .is_err()
    );
}
