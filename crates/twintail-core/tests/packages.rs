//! Local installation, containment and corruption contracts.
use std::{fs, path::Path};
use tempfile::TempDir;
use twintail_core::{Manifest, PackageStore, load_package, validate_manifest};
use twintail_protocol::ErrorCode;
fn source(root: &Path) -> std::path::PathBuf {
    let path = root.join("source");
    fs::create_dir_all(path.join("bin")).unwrap();
    fs::write(path.join("plugin.toml"), "id='sample'\nname='Sample'\nversion='1.0.0'\nprotocol_version=1\nhost_api='^0.1'\nentrypoint='bin/plugin'\n").unwrap();
    fs::write(path.join("bin/plugin"), "binary").unwrap();
    path
}
fn read_manifest(path: &Path) -> Manifest {
    toml::from_str(&fs::read_to_string(path.join("plugin.toml")).unwrap()).unwrap()
}
fn write_manifest(path: &Path, manifest: &Manifest) {
    fs::write(path.join("plugin.toml"), toml::to_string(manifest).unwrap()).unwrap();
}
#[test]
fn install_verify_inventory_no_overwrite_and_uninstall_preserves_data() {
    let temp = TempDir::new().unwrap();
    let src = source(temp.path());
    let store = PackageStore::new(temp.path().join("packages")).unwrap();
    fs::create_dir_all(temp.path().join("data/sample")).unwrap();
    fs::write(temp.path().join("data/sample/config.toml"), "private").unwrap();
    let package = store.install_local(&src).unwrap();
    assert_eq!(package.path, temp.path().join("packages/sample/1.0.0"));
    assert_eq!(package.checksum.len(), 64);
    assert_eq!(
        store.verify("sample", "1.0.0").unwrap().checksum,
        package.checksum
    );
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(
        store.install_local(&src).unwrap_err().code,
        ErrorCode::Conflict
    );
    fs::write(package.path.join("bin/plugin"), "damage").unwrap();
    assert!(store.verify("sample", "1.0.0").is_err());
    assert!(load_package(&package.path, &[]).is_err());
    assert!(store.list().is_err());
    store.uninstall("sample", "1.0.0").unwrap();
    assert!(!package.path.exists());
    assert!(temp.path().join("data/sample/config.toml").exists());
    assert!(store.list().unwrap().is_empty());
    for id in ["../escape", "/escape", "a/b"] {
        assert!(store.uninstall(id, "1.0.0").is_err());
    }
}
#[test]
fn target_defaults_and_policy_are_explicit() {
    let temp = TempDir::new().unwrap();
    let src = source(temp.path());
    let mut m = read_manifest(&src);
    assert_eq!(m.config_version, 1);
    assert_eq!(m.reload_policy, "exclusive");
    assert!(m.target.is_none());
    assert!(m.config_schema.is_none());
    m.target = Some("unknown".into());
    assert_eq!(
        validate_manifest(&m).unwrap_err().code,
        ErrorCode::Unsupported
    );
    for (target, os, arch) in [
        ("linux-x86_64", "linux", "x86_64"),
        ("windows-x86_64", "windows", "x86_64"),
        ("linux-aarch64", "linux", "aarch64"),
    ] {
        m.target = Some(target.into());
        if (os, arch) == (std::env::consts::OS, std::env::consts::ARCH) {
            validate_manifest(&m).unwrap();
        } else {
            assert_eq!(
                validate_manifest(&m).unwrap_err().code,
                ErrorCode::IncompatibleVersion
            );
        }
    }
    m.target = None;
    m.reload_policy = "parallel_safe".into();
    assert_eq!(
        validate_manifest(&m).unwrap_err().code,
        ErrorCode::Unsupported
    );
    m.reload_policy = "exclusive".into();
    m.config_version = 0;
    assert!(validate_manifest(&m).is_err());
}
#[test]
fn schema_must_exist_inside_package_and_resources_are_bounded() {
    let temp = TempDir::new().unwrap();
    let src = source(temp.path());
    let mut m = read_manifest(&src);
    m.config_schema = Some("../schema.json".into());
    write_manifest(&src, &m);
    assert!(load_package(&src, &[]).is_err());
    m.config_schema = Some("schema.json".into());
    write_manifest(&src, &m);
    assert!(load_package(&src, &[]).is_err());
    fs::write(src.join("schema.json"), "{}").unwrap();
    load_package(&src, &[]).unwrap();
    let store = PackageStore::new(temp.path().join("packages")).unwrap();
    let file = fs::File::create(src.join("huge")).unwrap();
    file.set_len(64 * 1024 * 1024 + 1).unwrap();
    assert!(store.install_local(&src).is_err());
    fs::remove_file(src.join("huge")).unwrap();
    for i in 0..1024 {
        fs::write(src.join(format!("file{i}")), "").unwrap();
    }
    assert!(store.install_local(&src).is_err());
}
#[cfg(unix)]
#[test]
fn links_special_files_and_link_ancestors_are_rejected_permissions_survive() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = TempDir::new().unwrap();
    let src = source(temp.path());
    let store = PackageStore::new(temp.path().join("packages")).unwrap();
    symlink("bin/plugin", src.join("link")).unwrap();
    assert!(store.install_local(&src).is_err());
    fs::remove_file(src.join("link")).unwrap();
    symlink(&src, temp.path().join("linked-source")).unwrap();
    assert!(
        store
            .install_local(temp.path().join("linked-source"))
            .is_err()
    );
    symlink(temp.path(), temp.path().join("ancestor")).unwrap();
    assert!(
        store
            .install_local(temp.path().join("ancestor/source"))
            .is_err()
    );
    assert!(PackageStore::new(temp.path().join("ancestor/other")).is_err());
    std::os::unix::net::UnixListener::bind(src.join("socket")).unwrap();
    assert!(store.install_local(&src).is_err());
    fs::remove_file(src.join("socket")).unwrap();
    fs::set_permissions(src.join("bin/plugin"), fs::Permissions::from_mode(0o751)).unwrap();
    let p = store.install_local(&src).unwrap();
    assert_eq!(
        fs::metadata(p.path.join("bin/plugin"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o751
    );
    fs::remove_file(p.path.join("bin/plugin")).unwrap();
    symlink("../../../../source/bin/plugin", p.path.join("bin/plugin")).unwrap();
    assert!(store.verify("sample", "1.0.0").is_err());
    assert!(store.uninstall("sample", "1.0.0").is_err());
}

#[test]
fn validate_specs_checks_grants_duplicates_and_dependencies() {
    let temp = TempDir::new().unwrap();
    let src = source(temp.path());
    let spec = load_package(&src, &[]).unwrap();
    let mut illegal = spec.clone();
    illegal.grants.push("unknown".into());
    assert_eq!(
        twintail_core::validate_specs(vec![illegal])
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    let mut excess = spec.clone();
    excess.grants.push("actions.call".into());
    assert!(
        twintail_core::validate_specs(vec![excess]).unwrap()[0]
            .grants
            .is_empty()
    );
    assert_eq!(
        twintail_core::validate_specs(vec![spec.clone(), spec.clone()])
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let mut dependent = spec.clone();
    dependent
        .manifest
        .dependencies
        .insert("missing".into(), "*".into());
    assert_eq!(
        twintail_core::validate_specs(vec![dependent])
            .unwrap_err()
            .code,
        ErrorCode::DependencyFailed
    );
    let mut required = spec;
    required.manifest.capabilities.push("actions.call".into());
    required
        .manifest
        .required_capabilities
        .push("actions.call".into());
    assert_eq!(
        twintail_core::validate_specs(vec![required])
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
}

#[test]
fn package_os_lock_releases_and_old_lock_file_does_not_block() {
    let temp = TempDir::new().unwrap();
    let src = source(temp.path());
    let root = temp.path().join("packages");
    let store = PackageStore::new(&root).unwrap();
    let lock_path = root.join(".package-store.lock");
    fs::write(&lock_path, "stale sentinel from interrupted process").unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    lock.try_lock().unwrap();
    assert_eq!(
        store.install_local(&src).unwrap_err().code,
        ErrorCode::Conflict
    );
    drop(lock);
    store.install_local(&src).unwrap();
    assert!(lock_path.is_file());
    store.uninstall("sample", "1.0.0").unwrap();
}

#[test]
fn source_checksum_inventory_is_verified_before_reinstalling() {
    let temp = TempDir::new().unwrap();
    let src = source(temp.path());
    let first = PackageStore::new(temp.path().join("first")).unwrap();
    let package = first.install_local(&src).unwrap();
    let second = PackageStore::new(temp.path().join("second")).unwrap();
    assert_eq!(
        second.install_local(&package.path).unwrap().checksum,
        package.checksum
    );
    fs::write(package.path.join("bin/plugin"), "corrupt").unwrap();
    let third = PackageStore::new(temp.path().join("third")).unwrap();
    assert!(third.install_local(&package.path).is_err());
    assert!(third.list().unwrap().is_empty());
}

#[test]
fn lightweight_inventory_never_certifies_damaged_bytes_and_bounds_metadata() {
    let temp = TempDir::new().unwrap();
    let src = source(temp.path());
    let store = PackageStore::new(temp.path().join("packages")).unwrap();
    let installed = store.install_local(src).unwrap();
    let entries = store.inventory().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "sample");
    assert_eq!(entries[0].checksum, installed.checksum);
    assert!(!entries[0].verified);
    fs::write(installed.path.join("bin/plugin"), "damage").unwrap();
    assert!(!store.inventory().unwrap()[0].verified);
    assert!(store.verify("sample", "1.0.0").is_err());
    let receipt = installed.path.join("checksums.json");
    let original = fs::read(&receipt).unwrap();
    fs::write(
        &receipt,
        r#"{"../escape":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
    )
    .unwrap();
    assert!(store.inventory().is_err());
    fs::write(&receipt, original).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&receipt)
        .unwrap()
        .set_len(1024 * 1024 + 1)
        .unwrap();
    assert!(store.inventory().is_err());
}

#[cfg(unix)]
#[test]
fn lightweight_inventory_rejects_receipt_and_content_links() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let src = source(temp.path());
    let store = PackageStore::new(temp.path().join("packages")).unwrap();
    let installed = store.install_local(&src).unwrap();
    fs::remove_file(installed.path.join("bin/plugin")).unwrap();
    symlink(src.join("bin/plugin"), installed.path.join("bin/plugin")).unwrap();
    assert!(store.inventory().is_err());
    fs::remove_file(installed.path.join("bin/plugin")).unwrap();
    fs::copy(src.join("bin/plugin"), installed.path.join("bin/plugin")).unwrap();
    let receipt = installed.path.join("checksums.json");
    fs::rename(&receipt, installed.path.join("receipt-backup")).unwrap();
    symlink("receipt-backup", receipt).unwrap();
    assert!(store.inventory().is_err());
}
