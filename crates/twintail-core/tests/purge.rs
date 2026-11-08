//! Complete namespace removal is explicit, preflighted, locked and retryable.
use std::{fs, path::Path};
use twintail_core::PackageStore;
use twintail_protocol::ErrorCode;

fn private(path: &Path) {
    fs::create_dir_all(path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
}
fn fixture(root: &Path) -> PackageStore {
    private(&root.join("data/plugins/sample"));
    fs::create_dir_all(root.join("data/plugins/sample/data/nested")).unwrap();
    fs::write(
        root.join("data/plugins/sample/config.toml"),
        "secret fixture",
    )
    .unwrap();
    fs::write(root.join("data/plugins/sample/data/nested/db"), "db").unwrap();
    for id in ["sample", "other"] {
        for version in ["1.0.0", "2.0.0"] {
            fs::create_dir_all(root.join("packages").join(id).join(version)).unwrap();
            fs::write(
                root.join("packages").join(id).join(version).join("binary"),
                "bytes",
            )
            .unwrap();
        }
        private(&root.join("data/plugins").join(id));
    }
    fs::write(root.join("data/plugins/other/config.toml"), "keep").unwrap();
    fs::write(root.join("data/operations.json"), "keep audit").unwrap();
    PackageStore::new(root.join("packages")).unwrap()
}
#[test]
fn all_versions_config_data_removed_other_plugins_and_audit_retained_and_retry_succeeds() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path());
    for _ in 0..2 {
        let purge = store
            .prepare_purge("sample", temp.path().join("data"))
            .unwrap();
        purge.remove_packages().unwrap();
        purge.remove_data().unwrap();
        assert!(!temp.path().join("packages/sample").exists());
        assert!(!temp.path().join("data/plugins/sample").exists());
        assert!(temp.path().join("packages/other/2.0.0/binary").is_file());
        assert!(temp.path().join("data/plugins/other/config.toml").is_file());
        assert_eq!(
            fs::read_to_string(temp.path().join("data/operations.json")).unwrap(),
            "keep audit"
        );
    }
}
#[test]
fn unsafe_ids_overlap_and_config_cas_lock_are_rejected_before_deletion() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path());
    for id in [
        "..",
        ".",
        "../other",
        "sample/../other",
        "CON",
        "sample\\other",
    ] {
        assert!(
            store.prepare_purge(id, temp.path().join("data")).is_err(),
            "{id}"
        );
    }
    assert!(
        store
            .prepare_purge("sample", temp.path().join("packages/sample"))
            .is_err()
    );
    let recovery_overlap =
        PackageStore::new(temp.path().join("data/plugins/.purge-sample/packages")).unwrap();
    assert!(
        recovery_overlap
            .prepare_purge("sample", temp.path().join("data"))
            .is_err()
    );
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(temp.path().join("data/plugins/sample/config.lock"))
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        lock.set_permissions(fs::Permissions::from_mode(0o600))
            .unwrap();
    }
    lock.lock().unwrap();
    assert_eq!(
        store
            .prepare_purge("sample", temp.path().join("data"))
            .err()
            .unwrap()
            .code,
        ErrorCode::Conflict
    );
    assert!(temp.path().join("packages/sample/1.0.0/binary").is_file());
    drop(lock);
    let purge = store
        .prepare_purge("sample", temp.path().join("data"))
        .unwrap();
    assert_eq!(
        store
            .prepare_purge("other", temp.path().join("data"))
            .err()
            .unwrap()
            .code,
        ErrorCode::Conflict
    );
    purge.remove_packages().unwrap();
    purge.remove_data().unwrap();
}
#[test]
fn interrupted_retirement_is_recovered_and_stale_config_store_does_not_recreate_namespace() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path());
    let namespace = temp.path().join("data/plugins/sample");
    fs::remove_file(namespace.join("config.toml")).unwrap();
    let mut config = twintail_core::ConfigStore::open(
        &namespace,
        &temp.path().join("packages/sample/1.0.0"),
        None,
        1,
    )
    .unwrap();
    let candidate = config.prepare(0, serde_json::json!({"n":1})).unwrap();
    let purge = store
        .prepare_purge("sample", temp.path().join("data"))
        .unwrap();
    assert_eq!(
        config.persist(candidate).unwrap_err().reason,
        "configuration transaction busy"
    );
    drop(purge);
    // Simulate a process interruption immediately after retiring the data tree.
    fs::rename(&namespace, temp.path().join("data/plugins/.purge-sample")).unwrap();
    let candidate = config.prepare(0, serde_json::json!({"n":2})).unwrap();
    assert!(config.persist(candidate).is_err());
    assert!(!namespace.exists());
    let purge = store
        .prepare_purge("sample", temp.path().join("data"))
        .unwrap();
    purge.remove_packages().unwrap();
    purge.remove_data().unwrap();
    assert!(!temp.path().join("data/plugins/.purge-sample").exists());
    assert!(!namespace.exists());
}

#[test]
fn stable_external_gate_survives_full_deletion_and_serializes_reinstall_and_retirement() {
    use sha2::{Digest, Sha256};
    use twintail_core::ConfigStore;
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path());
    let namespace = temp.path().join("data/plugins/sample");
    let retired = temp.path().join("data/plugins/.purge-sample");
    fs::remove_file(namespace.join("config.toml")).unwrap();
    let mut config = ConfigStore::open(&namespace, temp.path(), None, 1).unwrap();
    let candidate = config.prepare(0, serde_json::json!({"n":1})).unwrap();
    let digest = Sha256::digest(b"sample");
    let gate = temp
        .path()
        .join("data/plugins/.config-locks")
        .join(format!("{digest:x}.lock"));
    let original = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&gate)
        .unwrap();
    let purge = store
        .prepare_purge("sample", temp.path().join("data"))
        .unwrap();
    purge.remove_packages().unwrap();
    purge.remove_data().unwrap();
    assert!(!namespace.exists());
    assert!(!retired.exists());
    assert!(gate.is_file());
    assert!(original.try_lock().is_err());
    assert_eq!(
        config.persist(candidate).unwrap_err().reason,
        "configuration transaction busy"
    );
    assert!(ConfigStore::open(&namespace, temp.path(), None, 1).is_err());
    assert!(!namespace.exists());
    drop(purge);
    let candidate = config.prepare(0, serde_json::json!({"n":2})).unwrap();
    assert!(config.persist(candidate).is_err());
    assert!(!namespace.exists());
    original.try_lock().unwrap();
    assert!(ConfigStore::open(&namespace, temp.path(), None, 1).is_err());
    assert!(ConfigStore::open(&retired, temp.path(), None, 1).is_err());
    original.unlock().unwrap();
    let mut reinstalled = ConfigStore::open(&namespace, temp.path(), None, 1).unwrap();
    original.try_lock().unwrap();
    assert_eq!(
        reinstalled
            .update(0, serde_json::json!({"n":3}))
            .unwrap_err()
            .reason,
        "configuration transaction busy"
    );
    original.unlock().unwrap();
    reinstalled.update(0, serde_json::json!({"n":3})).unwrap();
    assert!(temp.path().join("data/plugins/other/config.toml").is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&gate).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(gate.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}

#[test]
fn late_io_failure_can_be_partial_and_retired_tree_is_removed_on_retry() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path());
    let purge = store
        .prepare_purge("sample", temp.path().join("data"))
        .unwrap();
    // Simulate a file arriving after the complete preflight. We must report the
    // nonempty-directory failure, not silently claim complete/atomic deletion.
    fs::write(temp.path().join("data/plugins/sample/late"), "late fixture").unwrap();
    purge.remove_packages().unwrap();
    assert!(purge.remove_data().is_err());
    assert!(!temp.path().join("packages/sample").exists());
    assert!(!temp.path().join("data/plugins/sample").exists());
    assert!(temp.path().join("data/plugins/.purge-sample/late").exists());
    drop(purge);
    let retry = store
        .prepare_purge("sample", temp.path().join("data"))
        .unwrap();
    retry.remove_packages().unwrap();
    retry.remove_data().unwrap();
    assert!(!temp.path().join("data/plugins/.purge-sample").exists());
}

#[cfg(unix)]
#[test]
fn readonly_data_parent_is_rejected_before_any_package_deletion() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path());
    let parent = temp.path().join("data/plugins");
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o500)).unwrap();
    assert!(
        store
            .prepare_purge("sample", temp.path().join("data"))
            .is_err()
    );
    assert!(temp.path().join("packages/sample/1.0.0/binary").exists());
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(unix)]
#[test]
fn nested_links_hardlinks_and_link_ancestors_fail_full_preflight() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path());
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), "untouched").unwrap();
    for path in [
        "packages/sample/2.0.0/link",
        "data/plugins/sample/data/link",
    ] {
        let path = temp.path().join(path);
        symlink(&outside, &path).unwrap();
        assert!(
            store
                .prepare_purge("sample", temp.path().join("data"))
                .is_err()
        );
        assert!(temp.path().join("packages/sample/1.0.0/binary").is_file());
        assert!(
            temp.path()
                .join("data/plugins/sample/config.toml")
                .is_file()
        );
        fs::remove_file(path).unwrap();
    }
    fs::hard_link(
        outside.join("keep"),
        temp.path().join("data/plugins/sample/data/hard"),
    )
    .unwrap();
    assert!(
        store
            .prepare_purge("sample", temp.path().join("data"))
            .is_err()
    );
    fs::remove_file(temp.path().join("data/plugins/sample/data/hard")).unwrap();
    symlink(temp.path().join("data"), temp.path().join("alias")).unwrap();
    assert!(
        store
            .prepare_purge("missing", temp.path().join("alias"))
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(outside.join("keep")).unwrap(),
        "untouched"
    );
}
#[cfg(windows)]
#[test]
fn junction_is_rejected_before_any_package_removal() {
    let temp = tempfile::tempdir().unwrap();
    let store = fixture(temp.path());
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), "untouched").unwrap();
    let junction = temp.path().join("data/plugins/sample/data/junction");
    assert!(
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        store
            .prepare_purge("sample", temp.path().join("data"))
            .is_err()
    );
    assert!(temp.path().join("packages/sample/1.0.0/binary").is_file());
    fs::remove_dir(&junction).unwrap();
    assert_eq!(
        fs::read_to_string(outside.join("keep")).unwrap(),
        "untouched"
    );
}
