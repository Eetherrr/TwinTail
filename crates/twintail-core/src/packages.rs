//! Bounded local package installation. Checksums detect damage, not authenticity.
use crate::{Manifest, load_package, manifest::invalid};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use twintail_protocol::{ErrorCode, RpcError};

const MAX_FILES: usize = 1024;
const MAX_FILE: u64 = 64 * 1024 * 1024;
const MAX_TOTAL: u64 = 256 * 1024 * 1024;

/// Installed identity and content checksum. Installing never enables a plugin.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledPackage {
    /// Stable plugin identity.
    pub id: String,
    /// SemVer package version.
    pub version: String,
    /// Absolute installed package directory.
    pub path: PathBuf,
    /// SHA256 of the sorted relative-file/checksum inventory.
    pub checksum: String,
}

/// Lightweight installed metadata, without verification of current file contents.
///
/// The checksum describes the stored receipt only. Call [`PackageStore::verify`]
/// before trusting or executing a package; this listing never certifies its bytes.
#[derive(Clone, Debug, Serialize)]
pub struct PackageInventory {
    /// Stable identity from the bounded installed manifest.
    pub id: String,
    /// SemVer package version matching the directory layout.
    pub version: String,
    /// Absolute installed package directory.
    pub path: PathBuf,
    /// SHA256 of the stored receipt, not of current package contents.
    pub checksum: String,
    /// Always false: this operation does not verify current file contents.
    pub verified: bool,
}

/// An inert installed README; the package contents have not been fully verified.
#[derive(Clone, Debug, Serialize)]
pub struct PackageReadme {
    /// Selected fixed basename, or null when neither candidate exists.
    pub filename: Option<String>,
    /// UTF-8 document text, or null when missing; never rendered as active HTML.
    pub content: Option<String>,
    /// Always false: this read does not certify package integrity.
    pub verified: bool,
}

/// Versioned package store, independent of host configuration and plugin data.
///
/// The administrator must protect this root and source directories from concurrent
/// external mutation. Safe standard filesystem APIs cannot prevent hostile
/// same-user filesystem replacement races. Operations reject observed symlinks,
/// including ancestors, and serialize writers using an operating-system file lock.
#[derive(Clone, Debug)]
pub struct PackageStore {
    root: PathBuf,
}

fn io(_: std::io::Error) -> RpcError {
    invalid("package filesystem operation failed")
}
fn reject_links(path: &Path) -> Result<(), RpcError> {
    for ancestor in path.ancestors() {
        let meta = fs::symlink_metadata(ancestor).map_err(io)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(invalid("reparse points are forbidden in package paths"));
            }
        }
        if meta.file_type().is_symlink() {
            return Err(invalid("symbolic links are forbidden in package paths"));
        }
    }
    Ok(())
}
fn absolute(path: &Path) -> Result<PathBuf, RpcError> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().map_err(io)?.join(path)
    };
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(invalid("parent path components are forbidden"));
    }
    Ok(path)
}
fn inventory(root: &Path) -> Result<BTreeMap<String, String>, RpcError> {
    fn walk(
        root: &Path,
        path: &Path,
        result: &mut BTreeMap<String, String>,
        total: &mut u64,
        entries: &mut usize,
    ) -> Result<(), RpcError> {
        for entry in fs::read_dir(path).map_err(io)? {
            let entry = entry.map_err(io)?;
            *entries += 1;
            // Also bound empty directories and traversal depth/resource use.
            if *entries > MAX_FILES * 2 {
                return Err(invalid("package has too many entries"));
            }
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| invalid("package path escapes root"))?;
            if relative.components().count() > 64 {
                return Err(invalid("package directory nesting exceeds limit"));
            }
            // Filesystem paths use native separators; receipt keys use portable '/'.
            // Validate components before converting so user-supplied path syntax
            // never gains the normalization allowed for these generated paths.
            let name = relative
                .components()
                .map(|component| match component {
                    std::path::Component::Normal(value) => value
                        .to_str()
                        .ok_or_else(|| invalid("package paths must be UTF-8")),
                    _ => Err(invalid("package path escapes root")),
                })
                .collect::<Result<Vec<_>, _>>()?
                .join("/");
            crate::plan::validate_relative(Path::new(&name), "package file")?;
            let metadata = fs::symlink_metadata(&path).map_err(io)?;
            if metadata.file_type().is_symlink() {
                return Err(invalid("package contains a symbolic link"));
            }
            if metadata.is_dir() {
                walk(root, &path, result, total, entries)?;
            } else if metadata.is_file() {
                if name == "checksums.json" {
                    continue;
                }
                if result.len() >= MAX_FILES || metadata.len() > MAX_FILE {
                    return Err(invalid("package file count or size exceeds limit"));
                }
                let mut file = fs::File::open(path).map_err(io)?.take(MAX_FILE + 1);
                let mut hash = Sha256::new();
                let mut buffer = [0_u8; 65536];
                let mut size = 0_u64;
                loop {
                    let read = file.read(&mut buffer).map_err(io)?;
                    if read == 0 {
                        break;
                    }
                    size += read as u64;
                    if size > MAX_FILE {
                        return Err(invalid("package file exceeds limit"));
                    }
                    hash.update(&buffer[..read]);
                }
                *total += size;
                if *total > MAX_TOTAL {
                    return Err(invalid("package total size exceeds limit"));
                }
                result.insert(name, format!("{:x}", hash.finalize()));
            } else {
                return Err(invalid("package contains a special file"));
            }
        }
        Ok(())
    }
    reject_links(root)?;
    let mut result = BTreeMap::new();
    walk(root, root, &mut result, &mut 0, &mut 0)?;
    Ok(result)
}
fn digest(files: &BTreeMap<String, String>) -> Result<String, RpcError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(files).map_err(|_| invalid("invalid checksum inventory"))?
        )
    ))
}
/// Validate a package checksum file when present; legacy packages may omit it.
pub(crate) fn verify_package_dir(path: &Path) -> Result<Option<String>, RpcError> {
    let checksum_path = path.join("checksums.json");
    let metadata = match fs::symlink_metadata(&checksum_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io(error)),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid("invalid checksum inventory"));
    }
    let actual = inventory(path)?;
    let mut bytes = Vec::new();
    fs::File::open(checksum_path)
        .map_err(io)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > 1024 * 1024 {
        return Err(invalid("checksum inventory exceeds limit"));
    }
    let expected: BTreeMap<String, String> =
        serde_json::from_slice(&bytes).map_err(|_| invalid("invalid checksum inventory"))?;
    if actual != expected {
        return Err(invalid("package checksum mismatch"));
    }
    Ok(Some(digest(&actual)?))
}

// Keep the file handle alive; OS locks release even on process termination.
struct Lock {
    _file: fs::File,
}

fn regular_file(metadata: &fs::Metadata) -> Result<(), RpcError> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid("unsafe package file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(invalid("hard links are forbidden in package files"));
        }
    }
    Ok(())
}
fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>, RpcError> {
    reject_links(path)?;
    let metadata = fs::symlink_metadata(path).map_err(io)?;
    regular_file(&metadata)?;
    if metadata.len() > limit {
        return Err(invalid("package file exceeds byte limit"));
    }
    let file = fs::File::open(path).map_err(io)?;
    regular_file(&file.metadata().map_err(io)?)?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("package file exceeds byte limit"));
    }
    Ok(bytes)
}

/// Preflighted destructive removal, holding the package writer and config CAS locks.
///
/// The host must first reject enabled targets and dependants. Both trees are
/// checked before removal starts. Removal is retryable, not atomic: an I/O failure
/// may leave a partially removed tree. Roots must remain administrator-controlled;
/// standard filesystem APIs do not prevent hostile same-user replacement races.
pub struct PluginPurge {
    _packages: Lock,
    _namespace: fs::File,
    config: std::sync::Mutex<Vec<fs::File>>,
    packages: Vec<PathBuf>,
    data: Vec<PathBuf>,
    retired: Vec<PathBuf>,
    data_path: PathBuf,
    retired_path: PathBuf,
}

fn writable_purge_directory(metadata: &fs::Metadata) -> Result<(), RpcError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Catch obvious permission failures in the full preflight. ACLs, mounts,
        // handles and external mutations can still fail later; this is not atomic.
        if metadata.mode() & 0o222 == 0 || metadata.mode() & 0o111 == 0 {
            return Err(invalid("purge directory is not writable"));
        }
    }
    #[cfg(not(unix))]
    let _ = metadata;
    Ok(())
}

fn purge_tree(root: &Path) -> Result<Vec<PathBuf>, RpcError> {
    // Missing roots are successful, but existing ancestors must still be safe.
    for ancestor in root.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => reject_links(ancestor)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(io(e)),
        }
        break;
    }
    match fs::symlink_metadata(root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io(e)),
        Ok(meta) if !meta.is_dir() => return Err(invalid("purge target must be a directory")),
        Ok(_) => {}
    }
    if let Some(parent) = root.parent() {
        writable_purge_directory(&fs::symlink_metadata(parent).map_err(io)?)?;
    }
    let mut paths = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(path) = pending.pop() {
        if paths.len() >= 1_000_000
            || path
                .strip_prefix(root)
                .map_err(|_| invalid("unsafe purge path"))?
                .components()
                .count()
                > 128
        {
            return Err(invalid("purge tree exceeds traversal limits"));
        }
        reject_links(&path)?;
        let metadata = fs::symlink_metadata(&path).map_err(io)?;
        if metadata.is_dir() {
            writable_purge_directory(&metadata)?;
            for entry in fs::read_dir(&path).map_err(io)? {
                if paths.len() + pending.len() >= 1_000_000 {
                    return Err(invalid("purge tree exceeds traversal limits"));
                }
                pending.push(entry.map_err(io)?.path());
            }
        } else {
            regular_file(&metadata)?;
            #[cfg(windows)]
            if metadata.permissions().readonly() {
                return Err(invalid("purge contains a read-only file"));
            }
        }
        paths.push(path);
    }
    Ok(paths)
}

fn remove_purge_tree(paths: &[PathBuf]) -> Result<(), RpcError> {
    for path in paths.iter().rev() {
        // Recheck before every deletion; never follow observed links or reparse points.
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(io(e)),
        };
        reject_links(path)?;
        if metadata.is_dir() {
            fs::remove_dir(path).map_err(io)?;
        } else {
            regular_file(&metadata)?;
            fs::remove_file(path).map_err(io)?;
        }
    }
    Ok(())
}

impl PluginPurge {
    /// Remove every package version for the target, preserving other identities.
    /// # Errors
    /// Returns a safe filesystem/validation error; partial removal is possible.
    pub fn remove_packages(&self) -> Result<(), RpcError> {
        remove_purge_tree(&self.packages)
    }
    /// Remove the complete plugin namespace, including configuration and data.
    /// # Errors
    /// Returns a safe filesystem/validation error; partial removal is possible.
    pub fn remove_data(&self) -> Result<(), RpcError> {
        // Windows denies renaming a directory containing our open config.lock.
        // Keep the stable external gate throughout closing, rename and deletion.
        self.config
            .lock()
            .map_err(|_| invalid("purge lock state failed"))?
            .clear();
        remove_purge_tree(&self.retired)?;
        if self.data.is_empty() {
            return Ok(());
        }
        // Retire under the stable gate. A ConfigStore using the original path
        // fails instead of rebuilding it; new opens also contend on this gate.
        // The fixed hidden path is preflighted on retry.
        reject_links(&self.data_path)?;
        if fs::symlink_metadata(&self.retired_path).is_ok() {
            return Err(RpcError::new(
                ErrorCode::Conflict,
                "purge retirement path is busy",
            ));
        }
        fs::rename(&self.data_path, &self.retired_path).map_err(io)?;
        let retired = self
            .data
            .iter()
            .map(|path| {
                path.strip_prefix(&self.data_path)
                    .map(|relative| self.retired_path.join(relative))
                    .map_err(|_| invalid("unsafe purge path"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        remove_purge_tree(&retired)
    }
}

impl PackageStore {
    /// Preflight all versions and the complete `data_root/plugins/id` namespace.
    /// Missing targets succeed. Rejects traversal, links (including hard links),
    /// Windows reparse points, special files and overlapping roots before deleting
    /// any content. The guard holds the package lock, a stable external namespace
    /// gate, and legacy config locks. Data is retired to `data_root/plugins/.purge-id`
    /// after closing inner config handles, while retaining the external gate.
    /// CAS writers never acquire a replacement lock during deletion. An interrupted
    /// retirement is preflighted and removed by the next call for the same id.
    /// # Errors
    /// Returns `Conflict` for busy locks, or safe validation/filesystem errors.
    pub fn prepare_purge(
        &self,
        id: &str,
        data_root: impl AsRef<Path>,
    ) -> Result<PluginPurge, RpcError> {
        let package_path = self.location(id, "0.0.0")?.parent().unwrap().to_owned();
        let data_root = absolute(data_root.as_ref())?;
        let data_path = data_root.join("plugins").join(id);
        // Leading dot is excluded by the plugin-id grammar, so this never aliases
        // a second plugin's namespace.
        let retired_path = data_root.join("plugins").join(format!(".purge-{id}"));
        if self.root.starts_with(&data_path)
            || data_path.starts_with(&self.root)
            || self.root.starts_with(&retired_path)
            || retired_path.starts_with(&self.root)
            || package_path.starts_with(&data_root)
            || data_root.starts_with(&package_path)
        {
            return Err(invalid("purge roots overlap"));
        }
        let packages_lock = self.lock()?;
        let namespace = crate::config::lock_namespace(&data_path).map_err(RpcError::from)?;
        let packages = purge_tree(&package_path)?;
        let mut data = purge_tree(&data_path)?;
        let mut retired = purge_tree(&retired_path)?;
        let mut config_locks = Vec::new();
        for (path, tree) in [(&data_path, &mut data), (&retired_path, &mut retired)] {
            if !tree.is_empty() {
                config_locks.push(crate::config::lock_directory(path).map_err(RpcError::from)?);
                // Acquiring the existing CAS lock may have created config.lock.
                *tree = purge_tree(path)?;
            }
        }
        Ok(PluginPurge {
            _packages: packages_lock,
            _namespace: namespace,
            config: std::sync::Mutex::new(config_locks),
            packages,
            data,
            retired,
            data_path,
            retired_path,
        })
    }
    /// Create/open a package root, rejecting symlink ancestors and non-directories.
    /// # Errors
    /// Returns `InvalidConfig` for unsafe roots or filesystem failures.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, RpcError> {
        let root = absolute(root.as_ref())?;
        // Check existing ancestors before creating missing components.
        for ancestor in root.ancestors() {
            match fs::symlink_metadata(ancestor) {
                Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                    return Err(invalid("unsafe package root"));
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io(e)),
            }
        }
        fs::create_dir_all(&root).map_err(io)?;
        reject_links(&root)?;
        Ok(Self {
            root: fs::canonicalize(root).map_err(io)?,
        })
    }
    fn lock(&self) -> Result<Lock, RpcError> {
        reject_links(&self.root)?;
        let path = self.root.join(".package-store.lock");
        let file = match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&path).map_err(io)?;
                reject_links(&path)?;
                regular_file(&metadata)?;
                fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&path)
                    .map_err(io)?
            }
            Err(error) => return Err(io(error)),
        };
        file.try_lock()
            .map_err(|_| RpcError::new(ErrorCode::Conflict, "package store is busy"))?;
        Ok(Lock { _file: file })
    }

    fn location(&self, id: &str, version: &str) -> Result<PathBuf, RpcError> {
        // Reuse the manifest identity and SemVer rules without trusting path text.
        let manifest: Manifest = toml::from_str("id='placeholder'\nname='package'\nversion='1.0.0'\nprotocol_version=1\nhost_api='^0.1'\nentrypoint='plugin'").map_err(|_| invalid("invalid package identity"))?;
        let manifest = Manifest {
            id: id.into(),
            version: version.into(),
            ..manifest
        };
        crate::validate_manifest(&manifest)?;
        Ok(self.root.join(id).join(version))
    }
    /// Copy a validated local package into `root/id/version` via same-root staging.
    /// All links and special files are rejected; limits are 1024 files, 64 MiB per
    /// file and 256 MiB total. Executable permissions are copied. Existing versions
    /// are never overwritten. The returned package is installed but not enabled.
    /// Source checksums, when present, must verify and are regenerated.
    /// Required capabilities do not need host grants during installation.
    /// # Errors
    /// Returns validation errors, `Conflict` for an existing version/busy store,
    /// or `InvalidConfig` for unsafe files, resource limits or copy failures.
    pub fn install_local(&self, source: impl AsRef<Path>) -> Result<InstalledPackage, RpcError> {
        let _lock = self.lock()?;
        let source = absolute(source.as_ref())?;
        reject_links(&source)?;
        let files = inventory(&source)?;
        let manifest_text = fs::read(source.join("plugin.toml")).map_err(io)?;
        if manifest_text.len() > 65536 {
            return Err(invalid("plugin.toml exceeds 64 KiB"));
        }
        let manifest: Manifest = toml::from_str(
            std::str::from_utf8(&manifest_text)
                .map_err(|_| invalid("invalid manifest encoding"))?,
        )
        .map_err(|_| invalid("invalid plugin manifest"))?;
        let original = load_package(&source, &manifest.capabilities)?;
        let destination = self.location(&manifest.id, &manifest.version)?;
        if fs::symlink_metadata(&destination).is_ok() {
            return Err(RpcError::new(
                ErrorCode::Conflict,
                "package version already exists",
            ));
        }
        let staging = tempfile::Builder::new()
            .prefix(".install-")
            .tempdir_in(&self.root)
            .map_err(io)?;
        let mut copied_total = 0_u64;
        for relative in files.keys() {
            let target = staging.path().join(relative);
            fs::create_dir_all(
                target
                    .parent()
                    .ok_or_else(|| invalid("invalid package file"))?,
            )
            .map_err(io)?;
            let available = MAX_FILE.min(MAX_TOTAL - copied_total);
            let mut input = fs::File::open(source.join(relative))
                .map_err(io)?
                .take(available + 1);
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map_err(io)?;
            let copied = std::io::copy(&mut input, &mut output).map_err(io)?;
            if copied > available {
                return Err(invalid("package copy exceeds file or total size limit"));
            }
            copied_total += copied;
            output.sync_all().map_err(io)?;
            fs::set_permissions(
                target,
                fs::metadata(source.join(relative))
                    .map_err(io)?
                    .permissions(),
            )
            .map_err(io)?;
        }
        if inventory(staging.path())? != files {
            return Err(invalid("package changed during installation"));
        }
        let copied = load_package(staging.path(), &manifest.capabilities)?;
        if copied.manifest.id != original.manifest.id
            || copied.manifest.version != original.manifest.version
        {
            return Err(invalid("package identity changed during installation"));
        }
        let checksums =
            serde_json::to_vec(&files).map_err(|_| invalid("invalid checksum inventory"))?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(staging.path().join("checksums.json"))
            .map_err(io)?;
        output.write_all(&checksums).map_err(io)?;
        output.sync_all().map_err(io)?;
        // Windows cannot rename the staging directory with its receipt still open.
        drop(output);
        let parent = destination
            .parent()
            .ok_or_else(|| invalid("invalid package destination"))?;
        fs::create_dir_all(parent).map_err(io)?;
        reject_links(parent)?;
        if fs::symlink_metadata(&destination).is_ok() {
            return Err(RpcError::new(
                ErrorCode::Conflict,
                "package version already exists",
            ));
        }
        fs::rename(staging.path(), &destination).map_err(io)?;
        Ok(InstalledPackage {
            id: manifest.id,
            version: manifest.version,
            path: destination,
            checksum: digest(&files)?,
        })
    }
    /// Verify all package contents against the generated checksum inventory.
    /// # Errors
    /// Returns `InvalidConfig` for damage, unsafe paths or invalid inventory.
    pub fn verify(&self, id: &str, version: &str) -> Result<InstalledPackage, RpcError> {
        let path = self.location(id, version)?;
        reject_links(&path)?;
        let checksum =
            verify_package_dir(&path)?.ok_or_else(|| invalid("missing checksum inventory"))?;
        let text = fs::read_to_string(path.join("plugin.toml")).map_err(io)?;
        let manifest: Manifest =
            toml::from_str(&text).map_err(|_| invalid("invalid installed manifest"))?;
        let spec = load_package(&path, &manifest.capabilities)?;
        if spec.manifest.id != id || spec.manifest.version != version {
            return Err(invalid("installed package identity mismatch"));
        }
        Ok(InstalledPackage {
            id: id.into(),
            version: version.into(),
            path,
            checksum,
        })
    }
    /// List installed packages in identity/version order, verifying every package.
    /// Hidden staging and lock entries are excluded. No runtime state is consulted.
    /// # Errors
    /// Returns validation errors if any installed package is damaged or unsafe.
    pub fn list(&self) -> Result<Vec<InstalledPackage>, RpcError> {
        reject_links(&self.root)?;
        let mut packages = Vec::new();
        for id in fs::read_dir(&self.root).map_err(io)? {
            let id = id.map_err(io)?;
            let name = id
                .file_name()
                .into_string()
                .map_err(|_| invalid("invalid package identity"))?;
            if name.starts_with('.') {
                continue;
            }
            reject_links(&id.path())?;
            for version in fs::read_dir(id.path()).map_err(io)? {
                let version = version
                    .map_err(io)?
                    .file_name()
                    .into_string()
                    .map_err(|_| invalid("invalid package version"))?;
                packages.push(self.verify(&name, &version)?);
            }
        }
        packages.sort_by(|a, b| (&a.id, &a.version).cmp(&(&b.id, &b.version)));
        Ok(packages)
    }
    /// List bounded manifest/receipt metadata without hashing executable contents.
    ///
    /// Entries are sorted by identity/version and always have `verified: false`.
    /// Paths, file kinds, receipt hashes and sizes are checked, but damaged regular
    /// file bytes may still appear here. Use [`Self::verify`] or [`Self::list`] for
    /// full content verification before enabling or reloading a package. Hidden
    /// store entries are excluded. At most 1024 installed versions are returned.
    /// # Errors
    /// Returns `InvalidConfig` for unsafe paths, malformed or oversized metadata,
    /// identity mismatches, missing receipt files or resource-limit violations.
    pub fn inventory(&self) -> Result<Vec<PackageInventory>, RpcError> {
        reject_links(&self.root)?;
        let mut packages = Vec::new();
        let mut identities = 0;
        for entry in fs::read_dir(&self.root).map_err(io)? {
            let entry = entry.map_err(io)?;
            let id = entry
                .file_name()
                .into_string()
                .map_err(|_| invalid("invalid package identity"))?;
            if id.starts_with('.') {
                continue;
            }
            identities += 1;
            if identities > MAX_FILES {
                return Err(invalid("too many installed identities"));
            }
            self.location(&id, "0.0.0")?;
            reject_links(&entry.path())?;
            if !fs::symlink_metadata(entry.path()).map_err(io)?.is_dir() {
                return Err(invalid("invalid package identity directory"));
            }
            for version in fs::read_dir(entry.path()).map_err(io)? {
                if packages.len() >= MAX_FILES {
                    return Err(invalid("too many installed versions"));
                }
                let version = version
                    .map_err(io)?
                    .file_name()
                    .into_string()
                    .map_err(|_| invalid("invalid package version"))?;
                packages.push(self.installed_metadata(&id, &version)?.0);
            }
        }
        packages.sort_by(|a, b| (&a.id, &a.version).cmp(&(&b.id, &b.version)));
        Ok(packages)
    }
    // Metadata validation shared by inventory and targeted inert document reads.
    fn installed_metadata(
        &self,
        id: &str,
        version: &str,
    ) -> Result<(PackageInventory, BTreeMap<String, String>), RpcError> {
        let path = self.location(id, version)?;
        reject_links(&path)?;
        let bytes = bounded(&path.join("plugin.toml"), 65536)?;
        let text =
            std::str::from_utf8(&bytes).map_err(|_| invalid("invalid installed manifest"))?;
        let manifest: Manifest =
            toml::from_str(text).map_err(|_| invalid("invalid installed manifest"))?;
        crate::validate_manifest(&manifest)?;
        if manifest.id != id || manifest.version != version {
            return Err(invalid("installed package identity mismatch"));
        }
        let receipt: BTreeMap<String, String> =
            serde_json::from_slice(&bounded(&path.join("checksums.json"), 1024 * 1024)?)
                .map_err(|_| invalid("invalid checksum inventory"))?;
        if receipt.is_empty()
            || receipt.len() > MAX_FILES
            || !receipt.contains_key("plugin.toml")
            || !receipt.contains_key(&manifest.entrypoint)
            || manifest
                .config_schema
                .as_ref()
                .is_some_and(|schema| !receipt.contains_key(schema))
        {
            return Err(invalid("invalid checksum inventory entries"));
        }
        let mut total = 0_u64;
        for (relative, checksum) in &receipt {
            crate::plan::validate_relative(Path::new(relative), "receipt path")?;
            if Path::new(relative).components().count() > 64
                || checksum.len() != 64
                || !checksum
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(invalid("invalid checksum inventory entry"));
            }
            let file = path.join(relative);
            reject_links(&file)?;
            let metadata = fs::symlink_metadata(file).map_err(io)?;
            regular_file(&metadata)?;
            if metadata.len() > MAX_FILE {
                return Err(invalid("unsafe package receipt file"));
            }
            total += metadata.len();
            if total > MAX_TOTAL {
                return Err(invalid("package total size exceeds limit"));
            }
        }
        Ok((
            PackageInventory {
                id: id.into(),
                version: version.into(),
                path,
                checksum: digest(&receipt)?,
                verified: false,
            },
            receipt,
        ))
    }
    /// Read the first installed README.md or README as inert UTF-8 text.
    ///
    /// Validates bounded manifest/receipt metadata and file paths, but does not hash
    /// executable contents or certify the package. Missing documents succeed with
    /// null fields. The selected document must be receipt-listed and at most 128 KiB.
    /// # Errors
    /// Returns `InvalidConfig` for unsafe paths/links, invalid package metadata,
    /// invalid UTF-8 or a document exceeding the byte limit.
    pub fn readme(&self, id: &str, version: &str) -> Result<PackageReadme, RpcError> {
        let (package, receipt) = self.installed_metadata(id, version)?;
        for filename in ["README.md", "README"] {
            let path = package.path.join(filename);
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io(error)),
                Ok(_) => {}
            }
            if !receipt.contains_key(filename) {
                return Err(invalid("README is not in installed checksum inventory"));
            }
            let bytes = bounded(&path, 128 * 1024)?;
            let content =
                String::from_utf8(bytes).map_err(|_| invalid("README must be valid UTF-8"))?;
            return Ok(PackageReadme {
                filename: Some(filename.into()),
                content: Some(content),
                verified: false,
            });
        }
        Ok(PackageReadme {
            filename: None,
            content: None,
            verified: false,
        })
    }
    /// Remove only a package version from disk, preserving all config and data.
    /// The management layer must reject enabled/in-use versions before calling.
    /// Damaged regular files may still be uninstalled; symlinks are rejected.
    /// # Errors
    /// Returns validation errors for unsafe identities/files or filesystem failures.
    pub fn uninstall(&self, id: &str, version: &str) -> Result<(), RpcError> {
        let _lock = self.lock()?;
        let path = self.location(id, version)?;
        inventory(&path)?;
        fs::remove_dir_all(path).map_err(io)
    }
}
