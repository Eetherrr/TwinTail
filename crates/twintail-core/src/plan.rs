use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

use schemars::JsonSchema;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use twintail_protocol::{ErrorCode, RpcError};

use crate::{Manifest, granted_capabilities, manifest::invalid};

/// Static host configuration. An empty configuration enables no plugins.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct HostConfig {
    /// Packages explicitly enabled by the host administrator.
    pub plugins: Vec<PluginEntry>,
}

/// One explicitly enabled package and its administrator-selected capabilities.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct PluginEntry {
    /// Package directory relative to the canonical installation root.
    pub path: PathBuf,
    /// Exact capability names; omitted grants deny all capabilities.
    #[serde(default)]
    pub grants: Vec<String>,
}

/// A validated package ready for process supervision.
#[derive(Clone, Debug)]
pub struct PluginSpec {
    /// Validated package manifest.
    pub manifest: Manifest,
    /// Canonical package directory within the installation root.
    pub package_dir: PathBuf,
    /// Canonical executable file within the package directory.
    pub executable: PathBuf,
    /// Sorted, unique intersection of requested and administrator-granted names.
    pub grants: Vec<String>,
}

pub(crate) fn validate_relative(path: &Path, label: &str) -> Result<(), RpcError> {
    // Also reject Windows path syntax on Unix so packages remain portable.
    let text = path.to_string_lossy();
    if text.is_empty()
        || text.contains('\\')
        || text.contains(':')
        || !path.components().any(|c| matches!(c, Component::Normal(_)))
        || path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(invalid(format!(
            "{label} must be a nonempty relative path without parent components"
        )));
    }
    Ok(())
}

fn io_error(label: &str, error: std::io::Error) -> RpcError {
    let code = if error.kind() == std::io::ErrorKind::NotFound {
        ErrorCode::NotFound
    } else {
        ErrorCode::InvalidConfig
    };
    RpcError::new(code, format!("{label}: {error}"))
}

fn canonical(path: &Path, label: &str) -> Result<PathBuf, RpcError> {
    fs::canonicalize(path).map_err(|e| io_error(label, e))
}

fn contained(path: &Path, root: &Path, label: &str) -> Result<PathBuf, RpcError> {
    let actual = canonical(path, label)?;
    if !actual.starts_with(root) {
        return Err(invalid(format!("{label} escapes its root")));
    }
    Ok(actual)
}

/// Read a TOML host configuration and validate a complete plugin loading plan.
///
/// `config_path` identifies the host TOML file; package paths inside it are
/// relative to `install_root`, independently of the config's location. The root,
/// package directories and executable files are canonicalized. Parent traversal,
/// absolute paths and symlinks escaping the relevant root are rejected. Manifests
/// are limited to 64 KiB before UTF-8/TOML parsing. Executables must be files;
/// executable permission is checked by spawn; explicit targets are validated here.
///
/// Dependencies always precede consumers. Among currently available nodes, the
/// smallest plugin ID is selected first, independently of config ordering.
/// This synchronous read-only operation has no cancellation support. Packages
/// must remain immutable through spawning to avoid filesystem replacement races.
/// Runtime configuration transactions and optional dependencies are not supported.
///
/// # Errors
/// Returns `NotFound` for absent files, `InvalidConfig` for invalid TOML, paths or
/// oversized manifests, `Conflict` for duplicate IDs, `DependencyFailed` for
/// missing/incompatible dependencies or cycles, and propagates manifest and
/// permission errors. No partial plan is returned on failure.
pub fn load_plan(config_path: &Path, install_root: &Path) -> Result<Vec<PluginSpec>, RpcError> {
    let root = canonical(install_root, "installation root")?;
    if !root.is_dir() {
        return Err(invalid("installation root is not a directory"));
    }
    let config_file = fs::File::open(config_path).map_err(|e| io_error("host configuration", e))?;
    let mut config = String::new();
    config_file
        .take(1_048_577)
        .read_to_string(&mut config)
        .map_err(|e| io_error("host configuration", e))?;
    if config.len() > 1_048_576 {
        return Err(invalid("host.toml exceeds 1 MiB"));
    }
    let config: HostConfig =
        toml::from_str(&config).map_err(|_| invalid("invalid host configuration TOML"))?;
    if config.plugins.len() > 128 {
        return Err(invalid("at most 128 plugins"));
    }
    let mut specs = BTreeMap::new();
    for entry in config.plugins {
        validate_relative(&entry.path, "plugin path")?;
        let package_dir = contained(&root.join(&entry.path), &root, "plugin directory")?;
        if !package_dir.is_dir() {
            return Err(invalid("plugin path is not a directory"));
        }
        let spec = load_package(&package_dir, &entry.grants)?;
        let id = spec.manifest.id.clone();
        if specs.insert(id.clone(), spec).is_some() {
            return Err(RpcError::new(
                ErrorCode::Conflict,
                format!("duplicate plugin id {id}"),
            ));
        }
    }
    sort_plan(specs)
}

/// Validate a local package without requiring a host configuration.
///
/// Reads at most 64 KiB of manifest and checks executable/schema containment.
/// Optional targets are checked against the host OS and architecture; packages
/// without a target remain compatible but have no platform certification.
/// Installed checksums, when present, are verified before parsing.
/// Schema contents are validated separately by the configuration API. No files
/// are modified. Callers must keep packages immutable after validation.
///
/// # Errors
/// Returns manifest/permission errors, `NotFound` for absent files, and
/// `InvalidConfig` for malformed manifests or paths escaping the package.
pub fn load_package(dir: &Path, grants: &[String]) -> Result<PluginSpec, RpcError> {
    let package_dir = canonical(dir, "plugin directory")?;
    if !package_dir.is_dir() {
        return Err(invalid("plugin path is not a directory"));
    }
    crate::packages::verify_package_dir(&package_dir)?;
    let manifest_path = contained(
        &package_dir.join("plugin.toml"),
        &package_dir,
        "plugin manifest",
    )?;
    if !manifest_path.is_file() {
        return Err(invalid("plugin manifest is not a file"));
    }
    let file = fs::File::open(&manifest_path).map_err(|e| io_error("plugin manifest", e))?;
    let mut bytes = Vec::new();
    file.take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error("plugin manifest", e))?;
    if bytes.len() > 65_536 {
        return Err(invalid("plugin.toml exceeds 64 KiB"));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|e| invalid(format!("plugin manifest is not UTF-8: {e}")))?;
    let manifest: Manifest =
        toml::from_str(text).map_err(|_| invalid("invalid plugin manifest TOML"))?;
    let grants = granted_capabilities(&manifest, grants)?;
    let executable = contained(
        &package_dir.join(&manifest.entrypoint),
        &package_dir,
        "entrypoint",
    )?;
    if !executable.is_file() {
        return Err(invalid("entrypoint is not a file"));
    }
    if let Some(schema) = &manifest.config_schema {
        let schema = contained(
            &package_dir.join(schema),
            &package_dir,
            "configuration schema",
        )?;
        if !schema.is_file() {
            return Err(invalid("configuration schema is not a file"));
        }
    }
    Ok(PluginSpec {
        manifest,
        package_dir,
        executable,
        grants,
    })
}

/// Validate manifests, duplicate identities and mandatory dependency constraints,
/// returning deterministic provider-before-consumer order.
///
/// This read-only function does not re-read package files. It is suitable for
/// checking a supervisor's proposed additions/replacements before activation.
/// # Errors
/// Returns manifest errors, `Conflict` for duplicate IDs, or `DependencyFailed`
/// for absent/incompatible providers and dependency cycles.
pub fn validate_specs(specs: Vec<PluginSpec>) -> Result<Vec<PluginSpec>, RpcError> {
    let mut indexed = BTreeMap::new();
    for mut spec in specs {
        spec.grants = granted_capabilities(&spec.manifest, &spec.grants)?;
        if indexed.insert(spec.manifest.id.clone(), spec).is_some() {
            return Err(RpcError::new(ErrorCode::Conflict, "duplicate plugin id"));
        }
    }
    sort_plan(indexed)
}

fn sort_plan(mut specs: BTreeMap<String, PluginSpec>) -> Result<Vec<PluginSpec>, RpcError> {
    let mut indegrees = BTreeMap::new();
    let mut consumers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, spec) in &specs {
        indegrees.insert(id.clone(), spec.manifest.dependencies.len());
        for (dependency, requirement) in &spec.manifest.dependencies {
            let provider = specs.get(dependency).ok_or_else(|| {
                RpcError::new(
                    ErrorCode::DependencyFailed,
                    format!("plugin {id} requires missing dependency {dependency}"),
                )
            })?;
            // Both values were checked by manifest validation above.
            let requirement = VersionReq::parse(requirement).map_err(|e| invalid(e.to_string()))?;
            let version =
                Version::parse(&provider.manifest.version).map_err(|e| invalid(e.to_string()))?;
            if !requirement.matches(&version) {
                return Err(RpcError::new(
                    ErrorCode::DependencyFailed,
                    format!("plugin {id} requires {dependency} {requirement}, installed {version}"),
                ));
            }
            consumers
                .entry(dependency.clone())
                .or_default()
                .push(id.clone());
        }
    }
    let mut ready: BTreeSet<_> = indegrees
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut result = Vec::new();
    while let Some(id) = ready.pop_first() {
        if let Some(spec) = specs.remove(&id) {
            result.push(spec);
        }
        for consumer in consumers.get(&id).into_iter().flatten() {
            if let Some(count) = indegrees.get_mut(consumer) {
                *count -= 1;
                if *count == 0 {
                    ready.insert(consumer.clone());
                }
            }
        }
    }
    if !specs.is_empty() {
        return Err(RpcError::new(
            ErrorCode::DependencyFailed,
            format!(
                "dependency cycle blocks plugins: {}",
                specs.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
        ));
    }
    Ok(result)
}
