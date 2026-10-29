//! Actual SDK subprocesses exercise host routing, independently of QQ or network services.
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
use tokio::time::{sleep, timeout};
use twintail_core::{Limits, Manifest, PluginSpec, Supervisor};
use twintail_protocol::ErrorCode;

fn spec(mode: &str) -> PluginSpec {
    let raw = match mode {
        "echo" => include_str!("../../../examples/echo-plugin/plugin.toml"),
        "mock" => include_str!("../../../examples/mock-adapter/plugin.toml"),
        _ => unreachable!(),
    };
    let mut manifest: Manifest = toml::from_str(raw).unwrap();
    manifest.args = vec![mode.into()];
    // Optional grants let the test drive the same typed client from a mock-only endpoint.
    manifest.capabilities.extend([
        "services.call".into(),
        "actions.call".into(),
        "host.health".into(),
    ]);
    let executable = Path::new(env!("CARGO_BIN_EXE_p2-fixture"))
        .canonicalize()
        .unwrap();
    PluginSpec {
        package_dir: executable.parent().unwrap().to_owned(),
        executable,
        grants: manifest.capabilities.clone(),
        manifest,
    }
}
fn host(specs: Vec<PluginSpec>) -> (tempfile::TempDir, Supervisor) {
    let data = tempfile::tempdir().unwrap();
    let host = Supervisor::new(
        specs,
        data.path().to_owned(),
        Limits {
            handshake: Duration::from_secs(3),
            request: Duration::from_secs(2),
            stop: Duration::from_secs(1),
        },
    )
    .unwrap();
    (data, host)
}
async fn call(host: &mut Supervisor, id: &str, method: &str, payload: Value) -> Value {
    host.call(id, method.into(), payload).await.unwrap()
}
async fn ready(host: &mut Supervisor, id: &str, method: &str) -> Value {
    timeout(Duration::from_secs(5), async {
        loop {
            let value = call(host, id, method, json!({})).await;
            if value["ready"] == true {
                break value;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("SDK start callback must complete")
}
async fn actions(host: &mut Supervisor, count: usize) -> Value {
    timeout(Duration::from_secs(5), async {
        loop {
            let value = call(host, "mock-adapter", "mock.actions", json!({})).await;
            if value.as_array().unwrap().len() >= count {
                break value;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("echo action must reach mock adapter")
}

#[tokio::test]
async fn sdk_echo_preserves_segments_and_old_bot_cannot_route_after_restart() {
    let (_data, mut host) = host(vec![spec("mock"), spec("echo")]);
    host.start_all().await;
    let mock = ready(&mut host, "mock-adapter", "mock.ready").await;
    ready(&mut host, "echo-plugin", "echo.ready").await;
    let payload = json!({"message_type":"group","group_id":"9007199254740993","message":[
        {"type":"text","data":{"text":"hello TwinTail"}},
        {"type":"vendor.future","data":{"unknown":[true,null,"9007199254740993"]}}
    ]});
    let report = call(&mut host, "mock-adapter", "mock.emit", payload.clone()).await;
    assert_eq!(report, json!({"delivered":1,"dropped":0}));
    let history = actions(&mut host, 1).await;
    assert_eq!(history[0]["bot"], mock["bot"]);
    assert_eq!(history[0]["action"], "send_msg");
    assert_eq!(history[0]["params"], payload);
    let old = history[0].clone();
    host.stop("echo-plugin").await.unwrap();
    host.stop("mock-adapter").await.unwrap();
    host.start("mock-adapter").await.unwrap();
    let new = ready(&mut host, "mock-adapter", "mock.ready").await;
    assert_ne!(mock["bot"], new["bot"]);
    let error = host
        .call("mock-adapter", "mock.action".into(), old)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Offline);
    host.start("echo-plugin").await.unwrap();
    ready(&mut host, "echo-plugin", "echo.ready").await;
    call(&mut host, "mock-adapter", "mock.emit", payload.clone()).await;
    let history = actions(&mut host, 1).await;
    assert_eq!(history[0]["bot"], new["bot"]);
    assert_eq!(history[0]["params"], payload);
    host.shutdown().await;
}

#[tokio::test]
async fn nested_service_call_resolves_semver_and_enforces_capabilities() {
    let (_data, mut host) = host(vec![spec("mock")]);
    host.start_all().await;
    ready(&mut host, "mock-adapter", "mock.ready").await;
    let request = json!({"provider":"mock-adapter","service":"mock.inspect","version_req":"^1.0","method":"inspect","params":{"extension":[1,null,true]}});
    assert_eq!(
        call(&mut host, "mock-adapter", "mock.service", request.clone()).await,
        request["params"]
    );
    let mut wrong = request.clone();
    wrong["version_req"] = json!("^2");
    assert_eq!(
        host.call("mock-adapter", "mock.service".into(), wrong)
            .await
            .unwrap_err()
            .code,
        ErrorCode::IncompatibleVersion
    );
    let mut missing = request;
    missing["service"] = json!("missing");
    assert_eq!(
        host.call("mock-adapter", "mock.service".into(), missing)
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    host.shutdown().await;

    let mut denied = spec("mock");
    denied
        .grants
        .retain(|c| c != "host.health" && c != "actions.call" && c != "services.call");
    let (_data, mut host) = self::host(vec![denied]);
    host.start_all().await;
    let bot = ready(&mut host, "mock-adapter", "mock.ready").await["bot"].clone();
    for (method, payload) in [
        ("mock.health", json!({})),
        (
            "mock.service",
            json!({"provider":"mock-adapter","service":"mock.inspect","version_req":"^1","method":"inspect","params":{}}),
        ),
        (
            "mock.action",
            json!({"bot":bot,"action":"send_msg","params":{}}),
        ),
    ] {
        assert_eq!(
            host.call("mock-adapter", method.into(), payload)
                .await
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
    }
    assert_eq!(
        call(&mut host, "mock-adapter", "mock.actions", json!({})).await,
        json!([])
    );
    host.shutdown().await;
}
