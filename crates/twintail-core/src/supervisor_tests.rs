use super::*;
use crate::Manifest;

fn spec(id: &str) -> PluginSpec {
    let manifest: Manifest = toml::from_str(&format!("id = '{id}'\nname = '{id}'\nversion = '1.0.0'\nprotocol_version = 1\nhost_api = '^0.1'\nentrypoint = 'missing'\n")).unwrap();
    PluginSpec {
        manifest,
        package_dir: std::env::temp_dir(),
        executable: std::env::temp_dir().join("twintail-nonexistent-p3-executable"),
        grants: vec![],
    }
}
fn snapshot(revision: u64) -> ConfigSnapshot {
    ConfigSnapshot {
        config_version: 1,
        revision,
        values: json!({"secret_reference":"env:EXAMPLE"}),
    }
}
fn supervisor(root: &std::path::Path) -> Supervisor {
    Supervisor::new(
        vec![spec("plugin")],
        root.into(),
        Limits {
            handshake: Duration::from_millis(50),
            request: Duration::from_millis(20),
            stop: Duration::from_millis(10),
        },
    )
    .unwrap()
}

#[tokio::test]
async fn cancelled_reload_preserves_old_active_configuration_and_resumes_rollback() {
    let root = tempfile::tempdir().unwrap();
    let mut supervisor = supervisor(root.path());
    supervisor.slots.get_mut("plugin").unwrap().active_config = Some(snapshot(1));
    supervisor.set_config("plugin", Some(snapshot(2))).unwrap();
    let (tx, mut commands) = mpsc::channel(2);
    let task = tokio::spawn(async move {
        // Hold drain acknowledgement until the caller cancels its waiter.
        let command = commands.recv().await.unwrap();
        let InstanceCommand::Drain { reply, .. } = command else {
            panic!()
        };
        std::future::pending::<()>().await;
        drop(reply);
    });
    let slot = supervisor.slots.get_mut("plugin").unwrap();
    slot.handle = Some(Instance {
        protocol_minor: 2,
        tx,
        task,
    });
    slot.status.state = "active".into();
    let mut candidate = spec("plugin");
    candidate.manifest.version = "2.0.0".into();
    assert!(
        timeout(
            Duration::from_millis(1),
            supervisor.reload("plugin", candidate)
        )
        .await
        .is_err()
    );
    let slot = &supervisor.slots["plugin"];
    let journal = slot.reload.as_ref().unwrap();
    assert_eq!(journal.old_config.as_ref().unwrap().revision, 1);
    assert_eq!(journal.candidate_config.as_ref().unwrap().revision, 2);
    assert_eq!(
        supervisor.set_config("plugin", None).unwrap_err().code,
        ErrorCode::Conflict
    );
    // Both executables are absent: reconcile must finish the saved operation with
    // two new generations and the original configuration selected for recovery.
    supervisor.reconcile().await;
    let slot = &supervisor.slots["plugin"];
    assert!(slot.reload.is_none());
    assert_eq!(slot.status.reload.as_ref().unwrap()["outcome"], "failed");
    assert_eq!(slot.status.generation, 2);
    assert_eq!(slot.next_config.as_ref().unwrap().revision, 1);
    assert_eq!(slot.spec.manifest.version, "1.0.0");
    supervisor.shutdown().await;
}

#[tokio::test]
async fn shutdown_cleans_cancelled_reload_without_starting_candidate() {
    let root = tempfile::tempdir().unwrap();
    let mut supervisor = supervisor(root.path());
    let (tx, mut commands) = mpsc::channel(2);
    let task = tokio::spawn(async move {
        while let Some(command) = commands.recv().await {
            match command {
                InstanceCommand::Drain { reply, .. } => {
                    let _keep = reply;
                    std::future::pending::<()>().await;
                }
                _ => break,
            }
        }
    });
    supervisor.slots.get_mut("plugin").unwrap().handle = Some(Instance {
        protocol_minor: 2,
        tx,
        task,
    });
    assert!(
        timeout(
            Duration::from_millis(1),
            supervisor.reload("plugin", spec("plugin"))
        )
        .await
        .is_err()
    );
    supervisor.shutdown().await;
    let slot = &supervisor.slots["plugin"];
    assert!(slot.handle.is_none());
    assert!(slot.reload.is_none());
    assert_eq!(slot.status.generation, 0);
    assert_eq!(slot.status.state, "stopped");
}

#[test]
fn add_remove_validate_dependencies_and_do_not_launch_packages() {
    let root = tempfile::tempdir().unwrap();
    let mut supervisor = supervisor(root.path());
    let mut consumer = spec("consumer");
    consumer
        .manifest
        .dependencies
        .insert("plugin".into(), "^1".into());
    supervisor.add(consumer).unwrap();
    assert_eq!(
        supervisor.remove("plugin").unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(supervisor.list()[0].id, "plugin");
    supervisor.remove("consumer").unwrap();
    supervisor.remove("plugin").unwrap();
    assert!(supervisor.list().is_empty());
    let mut invalid = spec("consumer");
    invalid
        .manifest
        .dependencies
        .insert("missing".into(), "*".into());
    assert_eq!(
        supervisor.add(invalid).unwrap_err().code,
        ErrorCode::DependencyFailed
    );
}

#[test]
fn online_configuration_acknowledgement_is_generation_bound() {
    let root = tempfile::tempdir().unwrap();
    let mut supervisor = supervisor(root.path());
    let slot = supervisor.slots.get_mut("plugin").unwrap();
    slot.status.state = "active".into();
    slot.status.generation = 3;
    slot.active_config = Some(snapshot(0));
    assert_eq!(
        supervisor
            .confirm_config("plugin", 2, Some(snapshot(1)))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(supervisor.active_config("plugin").unwrap().revision, 0);
    supervisor
        .confirm_config("plugin", 3, Some(snapshot(1)))
        .unwrap();
    assert_eq!(supervisor.active_config("plugin").unwrap().revision, 1);
    assert_eq!(
        supervisor.slots["plugin"]
            .next_config
            .as_ref()
            .unwrap()
            .revision,
        1
    );
}

#[tokio::test]
async fn dropping_supervisor_during_readiness_aborts_owned_instance_task() {
    struct DropSignal(Option<oneshot::Sender<()>>);
    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }
    let root = tempfile::tempdir().unwrap();
    let mut supervisor = supervisor(root.path());
    let (ready, received) = oneshot::channel();
    let (ended, ended_rx) = oneshot::channel();
    let guard = DropSignal(Some(ended));
    supervisor.slots.get_mut("plugin").unwrap().launching = Some(tokio::spawn(async move {
        let (tx, _rx) = mpsc::channel(1);
        let task = tokio::spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        let instance = Instance {
            protocol_minor: 2,
            tx,
            task,
        };
        let _ = ready.send(());
        // Model launch waiting for the SDK's lifecycle.started acknowledgement.
        std::future::pending::<()>().await;
        Ok(instance)
    }));
    received.await.unwrap();
    drop(supervisor);
    timeout(Duration::from_secs(1), ended_rx)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn reload_prevalidation_preserves_generation_and_selected_spec() {
    let root = tempfile::tempdir().unwrap();
    let mut supervisor = supervisor(root.path());
    let mut candidate = spec("other");
    assert_eq!(
        supervisor
            .reload("plugin", candidate.clone())
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    candidate = spec("plugin");
    candidate.manifest.reload_policy = "parallel_safe".into();
    assert_eq!(
        supervisor
            .reload("plugin", candidate)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    candidate = spec("plugin");
    candidate.grants.push("events.publish".into());
    assert_eq!(
        supervisor
            .reload("plugin", candidate)
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    let mut consumer = spec("consumer");
    consumer
        .manifest
        .dependencies
        .insert("plugin".into(), "^1".into());
    supervisor.add(consumer).unwrap();
    candidate = spec("plugin");
    candidate.manifest.version = "2.0.0".into();
    assert_eq!(
        supervisor
            .reload("plugin", candidate)
            .await
            .unwrap_err()
            .code,
        ErrorCode::DependencyFailed
    );
    supervisor.slots.get_mut("plugin").unwrap().status.state = "active".into();
    supervisor.slots.get_mut("consumer").unwrap().status.state = "active".into();
    assert_eq!(
        supervisor
            .reload("plugin", spec("plugin"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(supervisor.spec("plugin").unwrap().manifest.version, "1.0.0");
    let slot = &supervisor.slots["plugin"];
    assert_eq!(slot.status.generation, 0);
    assert!(slot.reload.is_none());
}
