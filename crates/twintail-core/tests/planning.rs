//! Contract tests for manifests, permission decisions and package loading plans.

use std::{collections::BTreeMap, fs, path::Path};
use tempfile::TempDir;
use twintail_core::{
    HostConfig, Manifest, PluginEntry, authorize, granted_capabilities, load_plan,
    validate_manifest,
};
use twintail_protocol::ErrorCode;

fn manifest(id: &str) -> Manifest {
    Manifest {
        id: id.into(),
        name: id.into(),
        version: "1.2.3".into(),
        protocol_version: 1,
        host_api: "^0.1".into(),
        entrypoint: "bin/plugin".into(),
        config_schema: None,
        config_version: 1,
        reload_policy: "exclusive".into(),
        target: None,
        args: vec![],
        capabilities: vec![],
        required_capabilities: vec![],
        dependencies: BTreeMap::new(),
    }
}

fn package(root: &Path, folder: &str, manifest: &Manifest) {
    let dir = root.join(folder);
    fs::create_dir_all(dir.join("bin")).unwrap();
    fs::write(dir.join("bin/plugin"), "placeholder").unwrap();
    fs::write(dir.join("plugin.toml"), toml::to_string(manifest).unwrap()).unwrap();
}

fn config(root: &Path, paths: &[&str]) -> std::path::PathBuf {
    let config = HostConfig {
        plugins: paths
            .iter()
            .map(|path| PluginEntry {
                path: path.into(),
                grants: vec![],
            })
            .collect(),
    };
    let file = root.join("host.toml");
    fs::write(&file, toml::to_string(&config).unwrap()).unwrap();
    file
}

#[test]
fn identity_and_versions_are_validated() {
    for id in [
        "", "A", "0abc", "a/b", "a\\b", "a b", "a..", "con", "con.txt", "nul", "aux", "prn",
        "com1", "lpt9.x", "中文",
    ] {
        assert_eq!(
            validate_manifest(&manifest(id)).unwrap_err().code,
            ErrorCode::InvalidConfig,
            "{id}"
        );
    }
    for id in ["a", "a.b-c_9", "com0", "com10", "console"] {
        validate_manifest(&manifest(id)).unwrap();
    }
    let mut m = manifest("a");
    m.version = "1.0".into();
    assert_eq!(
        validate_manifest(&m).unwrap_err().code,
        ErrorCode::InvalidConfig
    );
    m.version = "1.0.0".into();
    m.protocol_version = 2;
    assert_eq!(
        validate_manifest(&m).unwrap_err().code,
        ErrorCode::IncompatibleVersion
    );
    m.protocol_version = 1;
    m.host_api = ">=1".into();
    assert_eq!(
        validate_manifest(&m).unwrap_err().code,
        ErrorCode::IncompatibleVersion
    );
    m.host_api = "invalid".into();
    assert_eq!(
        validate_manifest(&m).unwrap_err().code,
        ErrorCode::InvalidConfig
    );
}

#[test]
fn permissions_are_exact_sorted_and_default_denied() {
    let mut m = manifest("a");
    m.capabilities = vec!["host.read".into(), "host.health".into(), "host.read".into()];
    assert!(granted_capabilities(&m, &[]).unwrap().is_empty());
    let grants = granted_capabilities(
        &m,
        &[
            "host.read".into(),
            "events.subscribe".into(),
            "host.health".into(),
        ],
    )
    .unwrap();
    assert_eq!(grants, ["host.health", "host.read"]);
    authorize(&grants, "host.read").unwrap();
    assert_eq!(
        authorize(&grants, "actions.call").unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        granted_capabilities(&m, &["host.*".into()])
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    assert_eq!(
        authorize(&["host.*".into()], "host.read").unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    m.required_capabilities = vec!["host.read".into()];
    assert_eq!(
        granted_capabilities(&m, &[]).unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    m.required_capabilities = vec!["actions.call".into()];
    assert_eq!(
        validate_manifest(&m).unwrap_err().code,
        ErrorCode::InvalidConfig
    );
    m.required_capabilities.clear();
    m.capabilities.push("unknown".into());
    assert_eq!(
        validate_manifest(&m).unwrap_err().code,
        ErrorCode::Unsupported
    );
}

#[test]
fn stable_topology_and_canonical_paths() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let mut a = manifest("a");
    a.dependencies.insert("z".into(), "^1".into());
    package(root, "a", &a);
    package(root, "b", &manifest("b"));
    package(root, "z", &manifest("z"));
    for paths in [["a", "b", "z"], ["z", "b", "a"]] {
        let plan = load_plan(&config(root, &paths), root).unwrap();
        assert_eq!(
            plan.iter()
                .map(|s| s.manifest.id.as_str())
                .collect::<Vec<_>>(),
            ["b", "z", "a"]
        );
        assert_eq!(
            plan[0].executable,
            fs::canonicalize(root.join("b/bin/plugin")).unwrap()
        );
        assert_eq!(
            plan[0].package_dir,
            fs::canonicalize(root.join("b")).unwrap()
        );
    }
}

#[test]
fn dependency_errors_are_explicit() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let mut a = manifest("a");
    a.dependencies.insert("b".into(), "^1".into());
    package(root, "a", &a);
    let error = load_plan(&config(root, &["a"]), root).unwrap_err();
    assert_eq!(error.code, ErrorCode::DependencyFailed);
    assert!(error.message.contains("missing dependency b"));
    let mut b = manifest("b");
    b.version = "2.0.0".into();
    package(root, "b", &b);
    assert_eq!(
        load_plan(&config(root, &["a", "b"]), root)
            .unwrap_err()
            .code,
        ErrorCode::DependencyFailed
    );
    b.version = "1.0.0".into();
    b.dependencies.insert("a".into(), "*".into());
    package(root, "b", &b);
    let error = load_plan(&config(root, &["a", "b"]), root).unwrap_err();
    assert_eq!(error.code, ErrorCode::DependencyFailed);
    assert!(error.message.contains("cycle"));
    a.dependencies.clear();
    a.dependencies.insert("a".into(), "*".into());
    package(root, "a", &a);
    assert!(
        load_plan(&config(root, &["a"]), root)
            .unwrap_err()
            .message
            .contains("cycle")
    );
    a.dependencies.clear();
    package(root, "a", &a);
    package(root, "other", &a);
    assert_eq!(
        load_plan(&config(root, &["a", "other"]), root)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}

#[test]
fn paths_and_file_types_are_restricted() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    package(root, "a", &manifest("a"));
    for path in ["../a", "/a", "C:\\a", "a/../a", ""] {
        assert_eq!(
            load_plan(&config(root, &[path]), root).unwrap_err().code,
            ErrorCode::InvalidConfig,
            "{path}"
        );
    }
    let mut m = manifest("a");
    for entry in [
        "../outside",
        "/bin/sh",
        "bin/../plugin",
        "C:\\cmd.exe",
        "",
        ".",
    ] {
        m.entrypoint = entry.into();
        assert_eq!(
            validate_manifest(&m).unwrap_err().code,
            ErrorCode::InvalidConfig,
            "{entry}"
        );
    }
    m.entrypoint = "bin".into();
    package(root, "a", &m);
    assert_eq!(
        load_plan(&config(root, &["a"]), root).unwrap_err().code,
        ErrorCode::InvalidConfig
    );
    m.entrypoint = "missing".into();
    package(root, "a", &m);
    assert_eq!(
        load_plan(&config(root, &["a"]), root).unwrap_err().code,
        ErrorCode::NotFound
    );
    fs::write(root.join("file"), "x").unwrap();
    assert_eq!(
        load_plan(&config(root, &["file"]), root).unwrap_err().code,
        ErrorCode::InvalidConfig
    );
}

#[test]
fn manifest_size_boundary_utf8_and_defaults() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    package(root, "a", &manifest("a"));
    let path = root.join("a/plugin.toml");
    let base = fs::read_to_string(&path).unwrap();
    let mut exact = base.clone();
    exact.push('#');
    exact.extend(std::iter::repeat_n('x', 65_536 - base.len() - 1));
    fs::write(&path, &exact).unwrap();
    load_plan(&config(root, &["a"]), root).unwrap();
    exact.push('x');
    fs::write(&path, exact).unwrap();
    assert!(
        load_plan(&config(root, &["a"]), root)
            .unwrap_err()
            .message
            .contains("64 KiB")
    );
    fs::write(&path, [0xff]).unwrap();
    assert_eq!(
        load_plan(&config(root, &["a"]), root).unwrap_err().code,
        ErrorCode::InvalidConfig
    );
    fs::write(root.join("host.toml"), "").unwrap();
    assert!(load_plan(&root.join("host.toml"), root).unwrap().is_empty());
    let m: Manifest = toml::from_str("id='a'\nname='A'\nversion='1.0.0'\nprotocol_version=1\nhost_api='^0.1'\nentrypoint='bin/a'\n").unwrap();
    assert!(
        m.args.is_empty()
            && m.capabilities.is_empty()
            && m.required_capabilities.is_empty()
            && m.dependencies.is_empty()
    );
}

#[cfg(unix)]
#[test]
fn escaping_symlinks_are_rejected_and_internal_links_resolve() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let outside = TempDir::new().unwrap();
    package(outside.path(), "outside", &manifest("a"));
    symlink(outside.path().join("outside"), root.join("escape")).unwrap();
    assert_eq!(
        load_plan(&config(root, &["escape"]), root)
            .unwrap_err()
            .code,
        ErrorCode::InvalidConfig
    );
    package(root, "a", &manifest("a"));
    fs::remove_file(root.join("a/bin/plugin")).unwrap();
    symlink(
        outside.path().join("outside/bin/plugin"),
        root.join("a/bin/plugin"),
    )
    .unwrap();
    assert_eq!(
        load_plan(&config(root, &["a"]), root).unwrap_err().code,
        ErrorCode::InvalidConfig
    );
    fs::remove_file(root.join("a/bin/plugin")).unwrap();
    fs::write(root.join("a/bin/real"), "x").unwrap();
    symlink("real", root.join("a/bin/plugin")).unwrap();
    let plan = load_plan(&config(root, &["a"]), root).unwrap();
    assert_eq!(
        plan[0].executable,
        fs::canonicalize(root.join("a/bin/real")).unwrap()
    );
    fs::remove_file(root.join("a/plugin.toml")).unwrap();
    symlink(
        outside.path().join("outside/plugin.toml"),
        root.join("a/plugin.toml"),
    )
    .unwrap();
    assert_eq!(
        load_plan(&config(root, &["a"]), root).unwrap_err().code,
        ErrorCode::InvalidConfig
    );
}

#[test]
fn routing_grants_require_exact_explicit_authorization() {
    let names = [
        "bots.register",
        "events.subscribe",
        "events.publish",
        "actions.call",
        "services.provide",
        "services.call",
    ];
    let mut m = manifest("adapter");
    m.capabilities = names.iter().map(|name| (*name).into()).collect();
    assert!(granted_capabilities(&m, &[]).unwrap().is_empty());
    let granted =
        granted_capabilities(&m, &["events.publish".into(), "bots.register".into()]).unwrap();
    authorize(&granted, "events.publish").unwrap();
    authorize(&granted, "bots.register").unwrap();
    for name in [
        "events.subscribe",
        "actions.call",
        "services.provide",
        "services.call",
    ] {
        assert_eq!(
            authorize(&granted, name).unwrap_err().code,
            ErrorCode::PermissionDenied
        );
    }
    for name in ["bots.*", "events.*", "services.register"] {
        assert_eq!(
            granted_capabilities(&m, &[name.into()]).unwrap_err().code,
            ErrorCode::Unsupported
        );
    }
}
