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

impl PackageStore {
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
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(invalid("unsafe package lock file"));
                }
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
        fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>, RpcError> {
            reject_links(path)?;
            let metadata = fs::symlink_metadata(path).map_err(io)?;
            if !metadata.is_file() || metadata.len() > limit {
                return Err(invalid("invalid or oversized package metadata"));
            }
            let mut bytes = Vec::new();
            fs::File::open(path)
                .map_err(io)?
                .take(limit + 1)
                .read_to_end(&mut bytes)
                .map_err(io)?;
            if bytes.len() as u64 > limit {
                return Err(invalid("package metadata exceeds limit"));
            }
            Ok(bytes)
        }
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
                let path = self.location(&id, &version)?;
                reject_links(&path)?;
                let bytes = bounded(&path.join("plugin.toml"), 65536)?;
                let text = std::str::from_utf8(&bytes)
                    .map_err(|_| invalid("invalid installed manifest"))?;
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
                    if !metadata.is_file() || metadata.len() > MAX_FILE {
                        return Err(invalid("unsafe package receipt file"));
                    }
                    total += metadata.len();
                    if total > MAX_TOTAL {
                        return Err(invalid("package total size exceeds limit"));
                    }
                }
                packages.push(PackageInventory {
                    id: id.clone(),
                    version,
                    path,
                    checksum: digest(&receipt)?,
                    verified: false,
                });
            }
        }
        packages.sort_by(|a, b| (&a.id, &a.version).cmp(&(&b.id, &b.version)));
        Ok(packages)
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
