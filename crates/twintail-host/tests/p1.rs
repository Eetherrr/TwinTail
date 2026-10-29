//! P1 contract tests use an independent, test-only subprocess without a plugin SDK.

use serde_json::{Value, json};
use std::{collections::BTreeMap, net::TcpStream, path::Path, time::Duration};
use tempfile::TempDir;
use tokio::time::{Instant, sleep, timeout};
use twintail_core::{Limits, Manifest, PluginSpec, PluginStatus, Supervisor};
use twintail_protocol::{ErrorCode, Outcome};

fn spec(id: &str, mode: &str) -> PluginSpec {
    // Deliberately bypass package validation: this suite tests the supervisor,
    // while core's loading-plan tests own package/path/manifest validation.
    let executable = Path::new(env!("CARGO_BIN_EXE_p1-fixture"))
        .canonicalize()
        .unwrap();
    PluginSpec {
        package_dir: executable.parent().unwrap().to_owned(),
        executable,
        grants: vec![],
        manifest: Manifest {
            id: id.into(),
            name: "P1 test fixture".into(),
            version: "0.1.0".into(),
            protocol_version: 1,
            host_api: "^0.1".into(),
            entrypoint: "p1-fixture".into(),
            args: vec![mode.into(), id.into()],
            capabilities: vec!["host.health".into()],
            required_capabilities: vec![],
            dependencies: BTreeMap::new(),
            config_schema: None,
            config_version: 1,
            reload_policy: "exclusive".into(),
            target: None,
        },
    }
}

fn supervisor(plan: Vec<PluginSpec>) -> (TempDir, Supervisor) {
    supervisor_with_limits(
        plan,
        Limits {
            handshake: Duration::from_millis(300),
            request: Duration::from_millis(100),
            stop: Duration::from_millis(100),
        },
    )
}

fn supervisor_with_limits(plan: Vec<PluginSpec>, limits: Limits) -> (TempDir, Supervisor) {
    let data = tempfile::tempdir().unwrap();
    let host = Supervisor::new(plan, data.path().to_owned(), limits).unwrap();
    (data, host)
}

fn status(host: &Supervisor, id: &str) -> PluginStatus {
    host.list().into_iter().find(|s| s.id == id).unwrap()
}

async fn failed(host: &mut Supervisor, id: &str, code: ErrorCode) {
    timeout(Duration::from_secs(2), async {
        loop {
            host.reconcile().await;
            let current = status(host, id);
            if current.state == "failed" {
                assert_eq!(current.error.unwrap().code, code, "{id}");
                break;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("supervisor must report failure within a bounded deadline");
}

async fn probe(host: &mut Supervisor, id: &str) -> Value {
    host.call(id, "probe".into(), json!({})).await.unwrap()
}

#[tokio::test]
async fn normal_lifecycle_calls_shutdown_and_increments_generation() {
    let (data, mut host) = supervisor(vec![spec("normal", "normal")]);
    assert_eq!(status(&host, "normal").state, "validated");
    assert_eq!(status(&host, "normal").generation, 0);
    host.start("normal").await.unwrap();
    assert_eq!(status(&host, "normal").state, "active");
    assert_eq!(probe(&mut host, "normal").await["generation"], 1);
    assert_eq!(
        host.start("normal").await.unwrap_err().code,
        ErrorCode::Conflict
    );
    host.stop("normal").await.unwrap();
    assert_eq!(status(&host, "normal").state, "stopped");
    let stopped = data.path().join("plugins/normal/data/fixture-stopped");
    assert_eq!(std::fs::read(&stopped).unwrap(), b"shutdown");
    assert_eq!(
        host.call("normal", "probe".into(), json!({}))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Offline
    );
    std::fs::remove_file(&stopped).unwrap();
    host.start("normal").await.unwrap();
    assert_eq!(status(&host, "normal").generation, 2);
    assert_eq!(probe(&mut host, "normal").await["generation"], 2);
    host.shutdown().await;
    assert_eq!(status(&host, "normal").state, "stopped");
    assert!(stopped.is_file());
}

#[tokio::test]
async fn handshake_failures_are_structured_and_leave_host_usable() {
    for (mode, code) in [
        ("wrong-version", ErrorCode::IncompatibleVersion),
        ("oversize", ErrorCode::InvalidFrame),
        ("bad-json", ErrorCode::InvalidFrame),
        ("handshake-timeout", ErrorCode::Timeout),
    ] {
        let (_data, mut host) = supervisor(vec![spec("bad", mode), spec("healthy", "normal")]);
        let started = Instant::now();
        assert_eq!(host.start("bad").await.unwrap_err().code, code, "{mode}");
        assert!(started.elapsed() < Duration::from_secs(2), "{mode}");
        assert_eq!(status(&host, "bad").state, "failed");
        assert_eq!(status(&host, "bad").error.unwrap().code, code);
        host.start("healthy").await.unwrap();
        assert_eq!(probe(&mut host, "healthy").await["generation"], 1);
        host.shutdown().await;
    }
}

#[tokio::test]
async fn request_timeout_retires_connection_and_allows_restart() {
    let (_data, mut host) = supervisor(vec![spec("slow", "response-timeout")]);
    host.start("slow").await.unwrap();
    let started = Instant::now();
    assert_eq!(
        host.call("slow", "probe".into(), json!({}))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Timeout
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    failed(&mut host, "slow", ErrorCode::Timeout).await;
    assert_eq!(
        host.call("slow", "probe".into(), json!({}))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Offline
    );
    host.start("slow").await.unwrap();
    assert_eq!(status(&host, "slow").generation, 2);
    host.shutdown().await;
}

#[tokio::test]
async fn requested_capability_is_denied_without_grant_and_allowed_with_grant() {
    let denied = spec("denied", "permission");
    let mut granted = spec("allowed", "granted");
    granted.grants.push("host.health".into());
    let (_data, mut host) = supervisor(vec![denied, granted]);
    host.start_all().await;
    let result = probe(&mut host, "denied").await;
    let outcome: Outcome = serde_json::from_value(result["health"].clone()).unwrap();
    match outcome {
        Outcome::Err(error) => assert_eq!(error.code, ErrorCode::PermissionDenied),
        other => panic!("expected permission denial, got {other:?}"),
    }
    assert_eq!(result["grants"], json!([]));
    assert_eq!(status(&host, "denied").state, "active");
    let result = probe(&mut host, "allowed").await;
    let outcome: Outcome = serde_json::from_value(result["health"].clone()).unwrap();
    match outcome {
        Outcome::Ok(value) => {
            assert_eq!(value["status"], "ok");
            assert_eq!(value["plugin_id"], "allowed");
            assert_eq!(value["generation"], 1);
        }
        other => panic!("expected granted host.health, got {other:?}"),
    }
    assert_eq!(result["grants"], json!(["host.health"]));
    host.shutdown().await;
}

#[tokio::test]
async fn crash_does_not_prevent_starting_healthy_instance() {
    let (_data, mut host) = supervisor(vec![spec("crash", "crash"), spec("healthy", "normal")]);
    host.start("crash").await.unwrap();
    failed(&mut host, "crash", ErrorCode::PluginExited).await;
    host.start("healthy").await.unwrap();
    assert_eq!(probe(&mut host, "healthy").await["generation"], 1);
    assert_eq!(status(&host, "healthy").state, "active");
    host.shutdown().await;
}

#[tokio::test]
async fn duplicate_requests_and_unsolicited_responses_retire_only_offender() {
    for mode in ["duplicate-id", "unknown-response"] {
        let (_data, mut host) = supervisor(vec![spec("bad", mode), spec("healthy", "normal")]);
        host.start_all().await;
        failed(&mut host, "bad", ErrorCode::InvalidRequest).await;
        assert_eq!(probe(&mut host, "healthy").await["generation"], 1);
        host.shutdown().await;
    }
}

#[tokio::test]
async fn dependency_stop_conflict_and_transitive_failure_cascade() {
    let provider = spec("provider", "response-timeout");
    let mut consumer = spec("consumer", "normal");
    consumer
        .manifest
        .dependencies
        .insert("provider".into(), "^0.1".into());
    let mut leaf = spec("leaf", "normal");
    leaf.manifest
        .dependencies
        .insert("consumer".into(), "^0.1".into());
    let (_data, mut host) = supervisor(vec![provider, consumer, leaf, spec("healthy", "normal")]);
    assert_eq!(
        host.start("consumer").await.unwrap_err().code,
        ErrorCode::DependencyFailed
    );
    assert_eq!(status(&host, "consumer").state, "failed");
    host.start_all().await;
    for id in ["provider", "consumer", "leaf", "healthy"] {
        assert_eq!(status(&host, id).state, "active", "{id}");
    }
    assert_eq!(
        host.stop("provider").await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        host.stop("consumer").await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        host.call("provider", "probe".into(), json!({}))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Timeout
    );
    failed(&mut host, "provider", ErrorCode::Timeout).await;
    failed(&mut host, "consumer", ErrorCode::DependencyFailed).await;
    failed(&mut host, "leaf", ErrorCode::DependencyFailed).await;
    assert_eq!(status(&host, "healthy").state, "active");
    assert_eq!(probe(&mut host, "healthy").await["generation"], 1);
    host.shutdown().await;
}

async fn wait_for_file(path: &Path) {
    timeout(Duration::from_secs(2), async {
        while !path.is_file() {
            sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("fixture must publish the expected marker");
}

#[tokio::test]
async fn cancelled_stop_preserves_child_until_shutdown_reaps_it_before_restart() {
    let (data, mut host) = supervisor_with_limits(
        vec![spec("stubborn", "ignore-shutdown")],
        Limits {
            handshake: Duration::from_secs(2),
            request: Duration::from_millis(100),
            stop: Duration::from_millis(500),
        },
    );
    host.start("stubborn").await.unwrap();
    assert_eq!(probe(&mut host, "stubborn").await["generation"], 1);
    let private = data.path().join("plugins/stubborn/data");
    let address = std::fs::read_to_string(private.join("fixture-address")).unwrap();
    let address = address.parse().unwrap();
    let shutdown_received = private.join("fixture-shutdown-received");
    {
        let stop = host.stop("stubborn");
        tokio::pin!(stop);
        tokio::select! {
            result = &mut stop => panic!("ignore-shutdown must still be awaiting child exit: {result:?}"),
            _ = wait_for_file(&shutdown_received) => {}
        }
        // This marker is written only after the process receives shutdown, so
        // stop has reached its await; leaving the scope now cancels that future.
        assert!(TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok());
    }
    timeout(Duration::from_secs(2), host.shutdown())
        .await
        .expect("shutdown must retain ownership of a cancelled stop's child");
    assert!(TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err());
    assert_eq!(status(&host, "stubborn").state, "stopped");
    assert!(
        !private.join("fixture-stopped").exists(),
        "fixture must require host kill"
    );
    host.start("stubborn").await.unwrap();
    assert_eq!(probe(&mut host, "stubborn").await["generation"], 2);
    assert_eq!(
        std::fs::read_to_string(private.join("fixture-address")).unwrap(),
        address.to_string(),
        "the next generation must acquire the old generation's exclusive port"
    );
    assert!(!private.join("fixture-overlap").exists());
    host.shutdown().await;
    assert!(TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err());
}

#[tokio::test]
async fn provider_crash_is_reconciled_while_unrelated_handshake_is_pending() {
    let trigger_dir = tempfile::tempdir().unwrap();
    let trigger = trigger_dir.path().join("slow-handshake");
    let trigger_arg = trigger.to_str().unwrap().to_owned();
    let mut provider = spec("provider", "crash-on-file");
    provider
        .manifest
        .args
        .extend(["0".into(), trigger_arg.clone()]);
    let mut dependent = spec("dependent", "normal");
    dependent
        .manifest
        .dependencies
        .insert("provider".into(), "^0.1".into());
    dependent
        .manifest
        .args
        .extend(["0".into(), trigger_arg.clone()]);
    let mut slow = spec("slow", "slow-handshake");
    slow.manifest.args.extend(["500".into(), trigger_arg]);
    let (data, mut host) = supervisor_with_limits(
        vec![provider, dependent, spec("healthy", "normal"), slow],
        Limits {
            handshake: Duration::from_secs(2),
            request: Duration::from_millis(100),
            stop: Duration::from_millis(100),
        },
    );
    for id in ["provider", "dependent", "healthy"] {
        host.start(id).await.unwrap();
        assert_eq!(probe(&mut host, id).await["generation"], 1);
    }
    // The provider exits only after slow's initialize, when its launch future
    // is pending. No explicit reconcile follows start before these assertions.
    host.start("slow").await.unwrap();
    assert_eq!(status(&host, "provider").state, "failed");
    assert_eq!(
        status(&host, "provider").error.unwrap().code,
        ErrorCode::PluginExited
    );
    assert_eq!(status(&host, "dependent").state, "failed");
    assert_eq!(
        status(&host, "dependent").error.unwrap().code,
        ErrorCode::DependencyFailed
    );
    assert_eq!(
        std::fs::read(
            data.path()
                .join("plugins/slow/data/fixture-dependent-stopped-before-ready")
        )
        .unwrap(),
        b"yes",
        "dependent must stop during the handshake, before start's final reconciliation"
    );
    assert_eq!(status(&host, "healthy").state, "active");
    assert_eq!(probe(&mut host, "healthy").await["generation"], 1);
    assert_eq!(probe(&mut host, "slow").await["generation"], 1);
    host.shutdown().await;
}

#[tokio::test]
async fn reply_and_control_backpressure_retire_only_the_nonreading_peer() {
    for mode in ["flood", "control-flood"] {
        let (_data, mut host) = supervisor(vec![spec("flooder", mode), spec("healthy", "normal")]);
        host.start("healthy").await.unwrap();
        host.start("flooder").await.unwrap();
        failed(&mut host, "flooder", ErrorCode::Overloaded).await;
        assert_eq!(status(&host, "healthy").state, "active", "{mode}");
        assert_eq!(probe(&mut host, "healthy").await["generation"], 1, "{mode}");
        timeout(Duration::from_secs(2), host.shutdown())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn late_response_after_timeout_is_confined_to_retired_generation() {
    let mut late = spec("late", "late");
    late.manifest.args.push("350".into());
    let (data, mut host) = supervisor_with_limits(
        vec![late],
        Limits {
            handshake: Duration::from_secs(2),
            request: Duration::from_millis(100),
            stop: Duration::from_millis(500),
        },
    );
    host.start("late").await.unwrap();
    assert_eq!(
        host.call("late", "probe".into(), json!({}))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Timeout
    );
    failed(&mut host, "late", ErrorCode::Timeout).await;
    assert_eq!(
        std::fs::read(data.path().join("plugins/late/data/fixture-late-response")).unwrap(),
        b"generation 1",
        "the old process must actually emit its delayed response during retirement"
    );
    host.start("late").await.unwrap();
    let current = probe(&mut host, "late").await;
    assert_eq!(current["generation"], 2);
    // P1 restarts after reaping the old process. This covers a real late reply
    // followed by a fresh call, without claiming simultaneous hot reload.
    assert_eq!(probe(&mut host, "late").await["generation"], 2);
    host.shutdown().await;
}

#[tokio::test]
async fn cancelled_start_is_joined_and_stopped_by_shutdown() {
    let (data, mut host) = supervisor(vec![spec("starting", "slow-handshake")]);
    assert!(
        timeout(Duration::from_millis(30), host.start("starting"))
            .await
            .is_err()
    );
    timeout(Duration::from_secs(2), host.shutdown())
        .await
        .unwrap();
    assert_eq!(status(&host, "starting").state, "stopped");
    assert!(
        data.path()
            .join("plugins/starting/data/fixture-stopped")
            .is_file()
    );
    host.start("starting").await.unwrap();
    assert_eq!(probe(&mut host, "starting").await["generation"], 2);
    host.shutdown().await;
}
