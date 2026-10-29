//! Exclusive replacement contracts using the independent subprocess fixture.
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, time::Duration};
use twintail_core::{Limits, Manifest, PluginSpec, Supervisor};
use twintail_protocol::ErrorCode;

fn spec(id: &str, mode: &str) -> PluginSpec {
    let executable = Path::new(env!("CARGO_BIN_EXE_p1-fixture"))
        .canonicalize()
        .unwrap();
    PluginSpec {
        package_dir: executable.parent().unwrap().to_owned(),
        executable,
        grants: vec![],
        manifest: Manifest {
            id: id.into(),
            name: "P3 fixture".into(),
            version: if mode.ends_with("-v2") {
                "0.2.0"
            } else {
                "0.1.0"
            }
            .into(),
            protocol_version: 1,
            host_api: "^0.1".into(),
            entrypoint: "p1-fixture".into(),
            args: vec![mode.into(), id.into()],
            capabilities: vec![],
            required_capabilities: vec![],
            dependencies: BTreeMap::new(),
            config_schema: None,
            config_version: 1,
            reload_policy: "exclusive".into(),
            target: None,
        },
    }
}
fn host(plan: Vec<PluginSpec>) -> (tempfile::TempDir, Supervisor) {
    let dir = tempfile::tempdir().unwrap();
    let host = Supervisor::new(
        plan,
        dir.path().to_owned(),
        Limits {
            handshake: Duration::from_secs(1),
            request: Duration::from_millis(100),
            stop: Duration::from_millis(100),
        },
    )
    .unwrap();
    (dir, host)
}
async fn probe(host: &mut Supervisor, id: &str) -> Value {
    host.call(id, "probe".into(), json!({})).await.unwrap()
}

#[tokio::test]
async fn exclusive_replacement_releases_port_and_preserves_other_plugin() {
    let (dir, mut host) = host(vec![
        spec("adapter", "ignore-shutdown"),
        spec("healthy", "normal"),
    ]);
    host.start("adapter").await.unwrap();
    host.start("healthy").await.unwrap();
    let first = probe(&mut host, "adapter").await;
    let result = host
        .reload("adapter", spec("adapter", "ignore-shutdown-v2"))
        .await
        .unwrap();
    assert_eq!(result["outcome"], "reloaded");
    let second = probe(&mut host, "adapter").await;
    assert_ne!(first["instance_id"], second["instance_id"]);
    assert!(second["generation"].as_u64().unwrap() > first["generation"].as_u64().unwrap());
    assert!(
        !dir.path()
            .join("plugins/adapter/data/fixture-overlap")
            .exists()
    );
    assert_eq!(probe(&mut host, "healthy").await["generation"], 1);
    host.shutdown().await;
}

#[tokio::test]
async fn candidate_failure_restores_old_package_with_new_generation() {
    let (_dir, mut host) = host(vec![spec("plugin", "normal")]);
    host.start("plugin").await.unwrap();
    let result = host
        .reload("plugin", spec("plugin", "wrong-version"))
        .await
        .unwrap();
    assert_eq!(result["outcome"], "rolled_back");
    assert!(result.get("error").is_some());
    assert!(
        probe(&mut host, "plugin").await["generation"]
            .as_u64()
            .unwrap()
            >= 3
    );
    host.shutdown().await;
}

#[tokio::test]
async fn rollback_failure_is_observable_and_other_plugin_survives() {
    let (_dir, mut host) = host(vec![
        spec("plugin", "start-once"),
        spec("healthy", "normal"),
    ]);
    host.start("plugin").await.unwrap();
    host.start("healthy").await.unwrap();
    let result = host
        .reload("plugin", spec("plugin", "wrong-version"))
        .await
        .unwrap();
    assert_eq!(result["outcome"], "failed");
    assert!(result.get("rollback_error").is_some());
    assert_eq!(
        host.list().iter().find(|s| s.id == "plugin").unwrap().state,
        "failed"
    );
    assert_eq!(probe(&mut host, "healthy").await["generation"], 1);
    host.shutdown().await;
}

#[tokio::test]
async fn active_dependants_and_capability_escalation_block_replacement() {
    let mut consumer = spec("consumer", "normal");
    consumer
        .manifest
        .dependencies
        .insert("provider".into(), "^0.1".into());
    let (_dir, mut host) = host(vec![spec("provider", "normal"), consumer]);
    host.start("provider").await.unwrap();
    host.start("consumer").await.unwrap();
    assert_eq!(
        host.reload("provider", spec("provider", "normal"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    host.stop("consumer").await.unwrap();
    let mut candidate = spec("provider", "normal");
    candidate.manifest.capabilities.push("host.health".into());
    candidate.grants.push("host.health".into());
    assert!(host.reload("provider", candidate).await.is_err());
    assert_eq!(probe(&mut host, "provider").await["generation"], 1);
    host.shutdown().await;
}

#[tokio::test]
async fn cancelled_reload_is_resumed_from_supervisor_owned_journal() {
    let (_dir, mut host) = host(vec![spec("plugin", "normal")]);
    host.start("plugin").await.unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(10),
            host.reload("plugin", spec("plugin", "normal-v2"))
        )
        .await
        .is_err()
    );
    host.reconcile().await;
    let status = host.list().into_iter().find(|s| s.id == "plugin").unwrap();
    assert_eq!(status.version, "0.2.0");
    assert_eq!(status.state, "active");
    assert_eq!(status.reload.unwrap()["outcome"], "reloaded");
    assert!(
        probe(&mut host, "plugin").await["generation"]
            .as_u64()
            .unwrap()
            > 1
    );
    host.shutdown().await;
}

#[tokio::test]
async fn shutdown_after_cancelled_reload_releases_owned_port() {
    let (dir, mut host) = host(vec![spec("plugin", "ignore-shutdown")]);
    host.start("plugin").await.unwrap();
    let address =
        std::fs::read_to_string(dir.path().join("plugins/plugin/data/fixture-address")).unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(10),
            host.reload("plugin", spec("plugin", "ignore-shutdown-v2"))
        )
        .await
        .is_err()
    );
    host.shutdown().await;
    let lease = std::net::TcpListener::bind(address)
        .expect("shutdown must reap the direct child holding the lease");
    drop(lease);
    assert_eq!(host.list()[0].state, "stopped");
}
