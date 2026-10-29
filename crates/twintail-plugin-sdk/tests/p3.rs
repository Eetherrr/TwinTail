//! Real SDK subprocess + supervisor + durable configuration transactions.
use serde_json::{Value, json};
use std::{fs, path::Path, time::Duration};
use twintail_core::{ConfigStore, Limits, Manifest, PluginSpec, PreparedConfig, Supervisor};
use twintail_protocol::{ConfigSnapshot, ErrorCode, RpcError};
const ID: &str = "p3-config";
struct Setup {
    _root: tempfile::TempDir,
    directory: std::path::PathBuf,
    spec: PluginSpec,
    store: ConfigStore,
    host: Supervisor,
}
fn setup(schema: Option<Value>) -> Setup {
    let root = tempfile::tempdir().unwrap();
    let package = root.path().join("package");
    fs::create_dir(&package).unwrap();
    if let Some(schema) = &schema {
        fs::write(
            package.join("config.schema.json"),
            serde_json::to_vec(schema).unwrap(),
        )
        .unwrap();
    }
    let mut manifest:Manifest=toml::from_str("id='p3-config'\nname='configuration fixture'\nversion='1.0.0'\nprotocol_version=1\nhost_api='^0.1'\nentrypoint='p3-fixture'\n").unwrap();
    manifest.config_schema = schema.map(|_| "config.schema.json".into());
    let executable = Path::new(env!("CARGO_BIN_EXE_p3-fixture"))
        .canonicalize()
        .unwrap();
    let spec = PluginSpec {
        package_dir: package,
        executable,
        manifest,
        grants: vec![],
    };
    let data = root.path().join("data");
    let directory = data.join("plugins").join(ID);
    let store = ConfigStore::open(
        &directory,
        &spec.package_dir,
        spec.manifest.config_schema.as_deref(),
        1,
    )
    .unwrap();
    let host = Supervisor::new(
        vec![spec.clone()],
        data,
        Limits {
            handshake: Duration::from_secs(2),
            request: Duration::from_secs(2),
            stop: Duration::from_secs(1),
        },
    )
    .unwrap();
    Setup {
        _root: root,
        directory,
        spec,
        store,
        host,
    }
}
async fn call(host: &mut Supervisor, method: &str, payload: Value) -> Result<Value, RpcError> {
    host.call(ID, method.into(), payload).await
}
async fn start(setup: &mut Setup) -> Result<Value, RpcError> {
    let snapshot = setup.store.resolve_snapshot().map_err(RpcError::from)?;
    setup.host.set_config(ID, Some(snapshot))?;
    let result = setup.host.start(ID).await?;
    let state = call(&mut setup.host, "config.status", json!({})).await?;
    let revision = setup.store.read().unwrap().revision;
    assert_eq!(state["applied_revision"], revision);
    setup.store.mark_applied(revision).unwrap();
    Ok(result)
}
fn resolved(candidate: &PreparedConfig) -> ConfigSnapshot {
    let mut snapshot = candidate.snapshot().clone();
    snapshot.values = candidate.resolved().clone();
    snapshot
}
async fn prepare(
    setup: &mut Setup,
    expected: u64,
    candidate: &PreparedConfig,
) -> Result<Value, RpcError> {
    call(
        &mut setup.host,
        "config.prepare",
        json!({"expected_revision":expected,"snapshot":resolved(candidate)}),
    )
    .await
}
async fn commit(setup: &mut Setup, revision: u64) -> Result<Value, RpcError> {
    let result = call(
        &mut setup.host,
        "config.commit",
        json!({"revision":revision}),
    )
    .await?;
    let snapshot = setup.store.resolve_snapshot().unwrap();
    let generation = setup.host.list()[0].generation;
    setup
        .host
        .confirm_config(ID, generation, Some(snapshot))
        .unwrap();
    setup.store.mark_applied(revision).unwrap();
    Ok(result)
}
#[tokio::test]
async fn initial_snapshot_prepare_rejection_and_commit_rejection_preserve_durable_truth() {
    let mut setup = setup(None);
    setup.store.update(0, json!({"message":"initial"})).unwrap();
    start(&mut setup).await.unwrap();
    let probe = call(&mut setup.host, "probe", json!({})).await.unwrap();
    assert_eq!(probe["revision"], 1);
    assert_eq!(probe["initial_revision"], 1);
    assert_eq!(probe["message"], "initial");
    let rejected = setup
        .store
        .prepare(1, json!({"reject_prepare":true,"message":"candidate"}))
        .unwrap();
    assert_eq!(
        prepare(&mut setup, 1, &rejected).await.unwrap_err().code,
        ErrorCode::InvalidConfig
    );
    assert_eq!(setup.store.read().unwrap().revision, 1);
    assert!(setup.store.status().unwrap().is_applied());
    let candidate = setup
        .store
        .prepare(1, json!({"reject_commit":true,"message":"desired"}))
        .unwrap();
    prepare(&mut setup, 1, &candidate).await.unwrap();
    setup.store.persist(candidate).unwrap();
    assert_eq!(
        commit(&mut setup, 2).await.unwrap_err().code,
        ErrorCode::InvalidConfig
    );
    let status = setup.store.status().unwrap();
    assert_eq!(status.desired_revision, 2);
    assert_eq!(status.applied_revision, Some(1));
    assert!(!status.is_applied());
    assert_eq!(
        call(&mut setup.host, "probe", json!({})).await.unwrap()["revision"],
        1
    );
    assert_eq!(setup.host.active_config(ID).unwrap().revision, 1);
    setup.host.shutdown().await;
}
#[tokio::test]
async fn failed_persistence_can_abort_prepared_candidate_and_update_again() {
    let mut setup = setup(None);
    start(&mut setup).await.unwrap();
    let candidate = setup.store.prepare(0, json!({"message":"first"})).unwrap();
    prepare(&mut setup, 0, &candidate).await.unwrap();
    // A filesystem failure after the plugin's validation must not poison future updates.
    fs::create_dir(setup.directory.join("config.toml")).unwrap();
    assert!(setup.store.persist(candidate).is_err());
    fs::remove_dir(setup.directory.join("config.toml")).unwrap();
    call(&mut setup.host, "config.abort", json!({"revision":1}))
        .await
        .unwrap();
    call(&mut setup.host, "config.abort", json!({"revision":1}))
        .await
        .unwrap();
    let candidate = setup.store.prepare(0, json!({"message":"second"})).unwrap();
    prepare(&mut setup, 0, &candidate).await.unwrap();
    setup.store.persist(candidate).unwrap();
    commit(&mut setup, 1).await.unwrap();
    assert_eq!(
        call(&mut setup.host, "probe", json!({})).await.unwrap()["message"],
        "second"
    );
    assert!(setup.store.status().unwrap().is_applied());
    setup.host.shutdown().await;
}
#[tokio::test]
async fn confirmed_online_revision_survives_candidate_start_rollback() {
    let mut setup = setup(None);
    start(&mut setup).await.unwrap();
    let candidate = setup
        .store
        .prepare(0, json!({"message":"online-one"}))
        .unwrap();
    prepare(&mut setup, 0, &candidate).await.unwrap();
    setup.store.persist(candidate).unwrap();
    commit(&mut setup, 1).await.unwrap();
    assert_eq!(setup.host.active_config(ID).unwrap().revision, 1);
    setup
        .store
        .update(1, json!({"reject_start":true,"message":"bad-two"}))
        .unwrap();
    setup
        .host
        .set_config(ID, Some(setup.store.resolve_snapshot().unwrap()))
        .unwrap();
    let result = setup.host.reload(ID, setup.spec.clone()).await.unwrap();
    assert_eq!(result["outcome"], "rolled_back");
    let probe = call(&mut setup.host, "probe", json!({})).await.unwrap();
    assert_eq!(probe["revision"], 1);
    assert_eq!(probe["initial_revision"], 1);
    assert_eq!(probe["message"], "online-one");
    assert_eq!(setup.host.active_config(ID).unwrap().revision, 1);
    let status = setup.store.status().unwrap();
    assert_eq!(status.desired_revision, 2);
    assert_eq!(status.applied_revision, Some(1));
    setup.host.shutdown().await;
}
#[tokio::test]
async fn schema_invalid_initialization_fails_before_spawn_then_valid_snapshot_starts() {
    let mut setup = setup(Some(
        json!({"type":"object","required":["message"],"properties":{"message":{"type":"string"}},"additionalProperties":false}),
    ));
    // Match management's resolve -> stage -> start boundary. Invalid initial state
    // must never reach Supervisor::start or consume a process generation.
    assert_eq!(
        start(&mut setup).await.unwrap_err().code,
        ErrorCode::InvalidConfig
    );
    assert_eq!(setup.host.list()[0].generation, 0);
    assert!(setup.host.active_config(ID).is_none());
    setup.store.update(0, json!({"message":"valid"})).unwrap();
    start(&mut setup).await.unwrap();
    assert_eq!(
        call(&mut setup.host, "probe", json!({})).await.unwrap()["revision"],
        1
    );
    setup.host.shutdown().await;
}
