//! Configuration persistence, recovery, secret and path-boundary regression tests.

use serde_json::json;
use std::{collections::BTreeMap, fs};
use tempfile::TempDir;
use twintail_core::{ConfigStore, MAX_CONFIG_BYTES};

fn roots() -> (TempDir, TempDir) {
    (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap())
}
fn store(data: &TempDir, package: &TempDir) -> ConfigStore {
    ConfigStore::open(data.path(), package.path(), None, 1).unwrap()
}

#[test]
fn revisions_prepare_and_restart_preserve_desired_applied_gap() {
    let (data, package) = roots();
    let mut config = store(&data, &package);
    assert_eq!(config.read().unwrap().revision, 0);
    let candidate = config.prepare(0, json!({"enabled": true})).unwrap();
    assert_eq!(candidate.snapshot().revision, 1);
    assert_eq!(config.read().unwrap().revision, 0);
    config.persist(candidate).unwrap();
    assert!(config.update(0, json!({})).is_err());
    assert!(!config.status().unwrap().is_applied());
    config.mark_applied(1).unwrap();
    config.update(1, json!({"enabled":false})).unwrap();
    let reopened = store(&data, &package);
    let status = reopened.status().unwrap();
    assert_eq!(status.desired_revision, 2);
    assert_eq!(status.applied_revision, Some(1));
    assert!(!status.is_applied());
    assert_eq!(reopened.read().unwrap().values, json!({"enabled":false}));
    assert!(config.mark_applied(1).is_err());
}

#[test]
fn stale_prepared_candidate_cannot_overwrite_new_desired() {
    let (data, package) = roots();
    let mut config = store(&data, &package);
    let candidate = config.prepare(0, json!({"n":1})).unwrap();
    config.update(0, json!({"n":2})).unwrap();
    assert!(config.persist(candidate).is_err());
    assert_eq!(config.read().unwrap().values, json!({"n":2}));
}

#[test]
fn secret_references_validate_resolved_values_but_never_persist_them() {
    let (data, package) = roots();
    fs::write(package.path().join("schema.json"), serde_json::to_vec(&json!({"type":"object","required":["token"],"properties":{"token":{"type":"string","minLength":8}}})).unwrap()).unwrap();
    let mut config = ConfigStore::open(data.path(), package.path(), Some("schema.json"), 2)
        .unwrap()
        .with_environment(BTreeMap::from([(
            "TOKEN".into(),
            "secret-fixture-value".into(),
        )]));
    let values = json!({"token":{"$secret":"TOKEN"}});
    let candidate = config.prepare(0, values.clone()).unwrap();
    assert_eq!(candidate.resolved()["token"], "secret-fixture-value");
    config.persist(candidate).unwrap();
    assert_eq!(config.read().unwrap().values, values);
    assert_eq!(
        config.resolve_snapshot().unwrap().values["token"],
        "secret-fixture-value"
    );
    let disk = fs::read_to_string(data.path().join("config.toml")).unwrap();
    assert!(!disk.contains("secret-fixture-value"));
    assert!(disk.contains("TOKEN"));
    assert!(config.update(1, json!({"token":"[REDACTED]"})).is_err());
    let invalid = config
        .clone()
        .with_environment(BTreeMap::from([("TOKEN".into(), "bad".into())]));
    let error = invalid.validate(&values).unwrap_err();
    assert_eq!(error.path, "/token");
    assert!(!format!("{error:?} {error}").contains("bad"));
    assert_eq!(config.read().unwrap().revision, 1);
}

#[test]
fn validation_and_unrepresentable_values_keep_old_file() {
    let (data, package) = roots();
    let mut config = store(&data, &package);
    config.update(0, json!({"enabled":true})).unwrap();
    let previous = fs::read(data.path().join("config.toml")).unwrap();
    assert!(config.update(1, json!({"null":null})).is_err());
    assert_eq!(fs::read(data.path().join("config.toml")).unwrap(), previous);
    assert!(
        config
            .update(1, json!({"big":"x".repeat(MAX_CONFIG_BYTES)}))
            .is_err()
    );
    assert_eq!(fs::read(data.path().join("config.toml")).unwrap(), previous);
    assert_eq!(fs::read_dir(data.path()).unwrap().count(), 2);
}

#[test]
fn schemas_are_offline_bounded_and_checked() {
    let (data, package) = roots();
    for schema in [
        json!({"$ref":"https://example.invalid/secret"}),
        json!({"$ref":"file:///tmp/private"}),
        json!({"$ref":"#/$defs/n"}),
        json!({"type":42}),
        json!({"$dynamicRef":"#x"}),
    ] {
        fs::write(
            package.path().join("schema.json"),
            serde_json::to_vec(&schema).unwrap(),
        )
        .unwrap();
        assert!(ConfigStore::open(data.path(), package.path(), Some("schema.json"), 1).is_err());
    }
    fs::write(
        package.path().join("schema.json"),
        " ".repeat(MAX_CONFIG_BYTES + 1),
    )
    .unwrap();
    assert!(ConfigStore::open(data.path(), package.path(), Some("schema.json"), 1).is_err());
    assert!(ConfigStore::open(data.path(), package.path(), Some("../schema.json"), 1).is_err());
    let mut nested = json!({});
    for _ in 0..70 {
        nested = json!({"properties":{"x":nested}});
    }
    fs::write(
        package.path().join("schema.json"),
        serde_json::to_vec(&nested).unwrap(),
    )
    .unwrap();
    assert!(ConfigStore::open(data.path(), package.path(), Some("schema.json"), 1).is_err());
}

#[test]
fn default_is_not_validated_until_initialization_and_revision_is_checked() {
    let (data, package) = roots();
    fs::write(
        package.path().join("schema.json"),
        r#"{"type":"object","required":["token"]}"#,
    )
    .unwrap();
    let config = ConfigStore::open(data.path(), package.path(), Some("schema.json"), 1).unwrap();
    assert_eq!(config.read().unwrap().revision, 0);
    assert!(config.resolve_snapshot().is_err());
    fs::write(
        data.path().join("config.toml"),
        format!("config_version = 1\nrevision = {}\n[values]\n", u64::MAX),
    )
    .unwrap();
    // TOML's integer domain may reject u64::MAX before the explicit revision
    // checked_add; in either case no wrapped candidate can be created.
    assert!(config.prepare(u64::MAX, json!({"token":"x"})).is_err());
}

#[cfg(unix)]
#[test]
fn symlinks_are_rejected_and_files_are_private() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (data, package) = roots();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("schema.json"), "true").unwrap();
    symlink(
        outside.path().join("schema.json"),
        package.path().join("schema.json"),
    )
    .unwrap();
    assert!(ConfigStore::open(data.path(), package.path(), Some("schema.json"), 1).is_err());
    let mut config = store(&data, &package);
    symlink(
        outside.path().join("config.toml"),
        data.path().join("config.toml"),
    )
    .unwrap();
    assert!(config.update(0, json!({})).is_err());
    fs::remove_file(data.path().join("config.toml")).unwrap();
    config.update(0, json!({"n":1})).unwrap();
    assert_eq!(
        fs::metadata(data.path()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(data.path().join("config.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    symlink(outside.path(), data.path().join("linked")).unwrap();
    assert!(ConfigStore::open(&data.path().join("linked/child"), package.path(), None, 1).is_err());
}

#[test]
fn competing_stores_and_clones_have_one_cas_winner() {
    use std::sync::{Arc, Barrier};
    for clone in [false, true] {
        let (data, package) = roots();
        let mut first = store(&data, &package);
        let mut second = if clone {
            first.clone()
        } else {
            store(&data, &package)
        };
        let a = first.prepare(0, json!({"winner":"a"})).unwrap();
        let b = second.prepare(0, json!({"winner":"b"})).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let other = barrier.clone();
        let a = std::thread::spawn(move || {
            other.wait();
            first.persist(a)
        });
        let b = std::thread::spawn(move || {
            barrier.wait();
            second.persist(b)
        });
        let results = [a.join().unwrap(), b.join().unwrap()];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let failure = results.into_iter().find_map(Result::err).unwrap();
        let rpc: twintail_protocol::RpcError = failure.into();
        assert_eq!(rpc.code, twintail_protocol::ErrorCode::Conflict);
        assert_eq!(store(&data, &package).read().unwrap().revision, 1);
    }
}

#[test]
fn busy_lock_and_stale_acknowledgement_do_not_mutate_desired_or_applied() {
    let (data, package) = roots();
    let mut config = store(&data, &package);
    config.update(0, json!({"n":1})).unwrap();
    config.mark_applied(1).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(data.path().join("config.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let failure: twintail_protocol::RpcError = config.update(1, json!({"n":2})).unwrap_err().into();
    assert_eq!(failure.code, twintail_protocol::ErrorCode::Conflict);
    assert_eq!(config.status().unwrap().desired_revision, 1);
    drop(lock);
    config.update(1, json!({"n":2})).unwrap();
    assert!(config.mark_applied(1).is_err());
    assert_eq!(config.status().unwrap().applied_revision, Some(1));
    assert_eq!(config.status().unwrap().desired_revision, 2);
}

#[cfg(unix)]
#[test]
fn nested_directories_are_private_and_hardlinks_are_rejected() {
    use std::os::unix::fs::PermissionsExt;
    let (data, package) = roots();
    let directory = data.path().join("plugins/sample");
    let mut config = ConfigStore::open(&directory, package.path(), None, 1).unwrap();
    assert_eq!(
        fs::metadata(data.path().join("plugins"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    config.update(0, json!({"n":1})).unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::hard_link(
        directory.join("config.toml"),
        outside.path().join("alias.toml"),
    )
    .unwrap();
    assert!(config.read().is_err());
    fs::remove_file(outside.path().join("alias.toml")).unwrap();
    assert_eq!(config.read().unwrap().revision, 1);
    fs::write(outside.path().join("schema.json"), "true").unwrap();
    fs::hard_link(
        outside.path().join("schema.json"),
        package.path().join("schema.json"),
    )
    .unwrap();
    assert!(ConfigStore::open(&directory, package.path(), Some("schema.json"), 1).is_err());
}

fn schema_store(schema: serde_json::Value) -> (TempDir, TempDir, ConfigStore) {
    let (data, package) = roots();
    fs::write(
        package.path().join("schema.json"),
        serde_json::to_vec(&schema).unwrap(),
    )
    .unwrap();
    let config = ConfigStore::open(data.path(), package.path(), Some("schema.json"), 1)
        .unwrap()
        .with_environment(BTreeMap::from([(
            "TOKEN".into(),
            "private-env-fixture".into(),
        )]));
    (data, package, config)
}

#[test]
fn management_writeonly_preserves_refs_and_cli_can_repair_legacy_literals() {
    let schema = json!({"type":"object", "properties": {
        "public":{"type":"string"}, "token":{"type":"string","writeOnly":true}
    }});
    let (_data, _package, mut config) = schema_store(schema);
    let legacy = json!({"public":"ordinary", "token":"legacy-private-fixture"});
    // Local CLI keeps compatibility, but management must never expose this value.
    config.validate(&legacy).unwrap();
    config.update(0, legacy.clone()).unwrap();
    assert_eq!(config.read().unwrap().values, legacy);
    let failure = config.read_for_management().unwrap_err();
    assert_eq!(failure.path, "/token");
    assert!(!format!("{failure:?}").contains("legacy-private-fixture"));
    assert!(config.validate_management_input(&legacy).is_err());
    assert_eq!(config.read().unwrap().revision, 1);
    let refs = json!({"public":"ordinary", "token":{"$secret":"TOKEN"}});
    config.validate_management_input(&refs).unwrap();
    config.update(1, refs.clone()).unwrap();
    assert_eq!(config.read_for_management().unwrap().values, refs);
    // Reading a reference is independent of availability of the environment.
    let unavailable = config.clone().with_environment(BTreeMap::new());
    assert_eq!(unavailable.read_for_management().unwrap().values, refs);
    assert!(unavailable.validate_management_input(&refs).is_err());
    config
        .validate_management_input(&json!({"public":"ordinary"}))
        .unwrap();
    for value in [
        json!(null),
        json!(42),
        json!({"$secret":"TOKEN","extra":true}),
        json!({"$secret":"bad-name"}),
        json!("[REDACTED]"),
    ] {
        assert!(
            config
                .validate_management_input(&json!({"token":value}))
                .is_err()
        );
    }
}

#[test]
fn management_secret_policy_covers_nested_items_and_conservative_composition() {
    let schemas_and_values = [
        (
            json!({"properties":{"nested":{"properties":{"secret":{"writeOnly":true}}}}}),
            json!({"nested":{"secret":"private"}}),
            "/nested/secret",
        ),
        (
            json!({"properties":{"tokens":{"items":{"writeOnly":true}}}}),
            json!({"tokens":["private"]}),
            "/tokens/0",
        ),
        (
            json!({"prefixItems":[{"writeOnly":true}],"items":true}),
            json!(["private"]),
            "/0",
        ),
        (
            json!({"prefixItems":[true],"items":{"writeOnly":true}}),
            json!(["public", "private"]),
            "/1",
        ),
        (
            json!({"allOf":[{"properties":{"token":{"writeOnly":true}}}]}),
            json!({"token":"private"}),
            "/token",
        ),
        (
            json!({"anyOf":[true,{"properties":{"token":{"writeOnly":true}}}]}),
            json!({"token":"private"}),
            "/token",
        ),
        (
            json!({"oneOf":[true,{"properties":{"token":{"writeOnly":true}}}]}),
            json!({"token":"private"}),
            "/token",
        ),
        (
            json!({"if":false,"then":{"properties":{"token":{"writeOnly":true}}}}),
            json!({"token":"private"}),
            "/token",
        ),
        (
            json!({"if":true,"else":{"properties":{"token":{"writeOnly":true}}}}),
            json!({"token":"private"}),
            "/token",
        ),
        (
            json!({"not":{"properties":{"token":{"writeOnly":true}}}}),
            json!({"token":"private"}),
            "/token",
        ),
        (
            json!({"dependentSchemas":{"other":{"properties":{"token":{"writeOnly":true}}}}}),
            json!({"token":"private"}),
            "/token",
        ),
        (
            json!({"patternProperties":{"^token$":{"writeOnly":true}}}),
            json!({"token":"private"}),
            "/token",
        ),
        (
            json!({"additionalProperties":{"writeOnly":true}}),
            json!({"token":"private"}),
            "/token",
        ),
        (
            json!({"unevaluatedProperties":{"writeOnly":true}}),
            json!({"token":"private"}),
            "/token",
        ),
        (
            json!({"contains":{"writeOnly":true}}),
            json!(["private"]),
            "/0",
        ),
        (
            json!({"unevaluatedItems":{"writeOnly":true}}),
            json!(["private"]),
            "/0",
        ),
    ];
    for (schema, values, expected_path) in schemas_and_values {
        let (data, _package, config) = schema_store(schema);
        // Handwritten existing state permits checking management reads even for
        // a conditional branch which would be rejected by normal validation.
        let snapshot = twintail_protocol::ConfigSnapshot {
            config_version: 1,
            revision: 1,
            values: values.clone(),
        };
        fs::write(
            data.path().join("config.toml"),
            toml::to_string(&snapshot).unwrap(),
        )
        .unwrap();
        let failure = config.read_for_management().unwrap_err();
        assert_eq!(failure.path, expected_path);
        assert_eq!(
            failure.reason,
            "writeOnly configuration requires an environment secret reference"
        );
        assert!(config.validate_management_input(&values).is_err());
    }
}

#[test]
fn management_policy_does_not_mark_unapplied_definitions_as_secrets() {
    let (_data, _package, mut config) = schema_store(json!({
        "$defs":{"unused":{"writeOnly":true}}, "properties":{"normal":{"type":"string"}}
    }));
    let values = json!({"normal":"public"});
    config.validate_management_input(&values).unwrap();
    config.update(0, values.clone()).unwrap();
    assert_eq!(config.read_for_management().unwrap().values, values);
}

#[cfg(windows)]
#[test]
fn windows_native_and_verbatim_roots_support_schema_and_repeated_transactions() {
    use std::path::{Component, PathBuf, Prefix};
    let (data, package) = roots();
    fs::write(package.path().join("schema.json"), r#"{"type":"object"}"#).unwrap();
    let extended_data = fs::canonicalize(data.path()).unwrap();
    let extended_package = fs::canonicalize(package.path()).unwrap();
    // canonicalize uses the extended namespace on Windows. Exercise both forms,
    // including the ordinary drive path supplied by CARGO_MANIFEST_DIR in CI.
    let native = |path: &std::path::Path| {
        PathBuf::from(path.to_str().unwrap().strip_prefix(r"\\?\").unwrap())
    };
    let native_data = native(&extended_data);
    let native_package = native(&extended_package);
    assert!(
        matches!(native_data.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_)))
    );
    assert!(
        matches!(extended_data.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::VerbatimDisk(_)))
    );
    for (root, package, child) in [
        (&native_data, &native_package, "native"),
        (&extended_data, &extended_package, "extended"),
    ] {
        let directory = root.join(child).join("config");
        let mut config = ConfigStore::open(&directory, package, Some("schema.json"), 1).unwrap();
        config.update(0, json!({"enabled":true})).unwrap();
        config.mark_applied(1).unwrap();
        config.update(1, json!({"enabled":false})).unwrap();
        let reopened = ConfigStore::open(&directory, package, Some("schema.json"), 1).unwrap();
        assert_eq!(reopened.read().unwrap().values, json!({"enabled":false}));
        assert_eq!(reopened.status().unwrap().desired_revision, 2);
        assert_eq!(reopened.status().unwrap().applied_revision, Some(1));
    }
}

#[cfg(windows)]
#[test]
fn windows_drive_relative_configuration_roots_are_rejected() {
    let (_, package) = roots();
    let result = ConfigStore::open(
        std::path::Path::new(r"C:twintail-unsafe-relative"),
        package.path(),
        None,
        1,
    );
    assert_eq!(result.err().unwrap().reason, "unsafe configuration path");
}

#[cfg(windows)]
#[test]
fn windows_junction_roots_ancestors_and_schema_packages_are_rejected() {
    let (data, package) = roots();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("schema.json"), "true").unwrap();
    let junction = data.path().join("junction");
    // Creating a directory junction needs no symlink privilege or Developer Mode.
    let output = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&junction)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "junction fixture creation failed");
    assert!(ConfigStore::open(&junction, package.path(), None, 1).is_err());
    assert!(ConfigStore::open(&junction.join("child"), package.path(), None, 1).is_err());
    assert!(!outside.path().join("child").exists());
    assert!(ConfigStore::open(data.path(), &junction, Some("schema.json"), 1).is_err());
    assert!(ConfigStore::open(data.path(), data.path(), Some("junction/schema.json"), 1).is_err());
    fs::remove_dir(junction).unwrap();
}
