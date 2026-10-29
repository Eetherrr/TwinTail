use std::collections::BTreeMap;

use schemars::JsonSchema;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use twintail_protocol::{ErrorCode, RpcError};

/// Version-one plugin package manifest, stored as `plugin.toml`.
///
/// Unknown optional fields are ignored. All dependencies are strong dependencies;
/// an absent or incompatible provider prevents loading the complete plan.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct Manifest {
    /// Stable lowercase identity matching `^[a-z][a-z0-9._-]*$`.
    pub id: String,
    /// Human-readable plugin name.
    pub name: String,
    /// Package version, using SemVer syntax.
    pub version: String,
    /// IPC protocol major version; currently exactly one.
    pub protocol_version: u16,
    /// SemVer requirement that must match host API `0.1.0`.
    pub host_api: String,
    /// Relative executable path inside the package; no parent components.
    pub entrypoint: String,
    /// Optional JSON Schema path relative to this package.
    #[serde(default)]
    pub config_schema: Option<String>,
    /// Configuration format version; omitted versions retain version one.
    #[serde(default = "default_config_version")]
    pub config_version: u32,
    /// Replacement policy. Only `exclusive` is implemented.
    #[serde(default = "default_reload_policy")]
    pub reload_policy: String,
    /// Certified OS/architecture pair. Omission preserves legacy, uncertified packages.
    #[serde(default)]
    pub target: Option<String>,
    /// Literal process arguments; never interpreted by a shell.
    #[serde(default)]
    pub args: Vec<String>,
    /// Requested capabilities; requesting alone never grants access.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Requested capabilities that must also be granted before loading.
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    /// Provider plugin IDs mapped to mandatory SemVer requirements.
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
}

fn default_config_version() -> u32 {
    1
}
fn default_reload_policy() -> String {
    "exclusive".into()
}

pub(crate) fn invalid(message: impl Into<String>) -> RpcError {
    RpcError::new(ErrorCode::InvalidConfig, message)
}

fn valid_id(id: &str) -> bool {
    let mut chars = id.bytes();
    if !chars.next().is_some_and(|c| c.is_ascii_lowercase())
        || !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
        || id.ends_with('.')
    {
        return false;
    }
    // Windows device names remain reserved with an extension, on every platform.
    let base = id
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    !matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) && !(base.len() == 4
        && (base.starts_with("COM") || base.starts_with("LPT"))
        && matches!(base.as_bytes()[3], b'1'..=b'9'))
}

/// Validate manifest syntax, supported versions and capability declarations.
///
/// This synchronous, read-only function has no cancellation or side effects. It
/// does not inspect files or resolve dependency providers; [`crate::load_plan`]
/// performs those checks. Capability names are exact, with no wildcard syntax.
///
/// # Errors
/// Returns `InvalidConfig` for malformed fields, `IncompatibleVersion` for an
/// unsupported protocol/API, or `Unsupported` for unknown capability names.
pub fn validate_manifest(manifest: &Manifest) -> Result<(), RpcError> {
    if !valid_id(&manifest.id) {
        return Err(invalid(format!("invalid plugin id {:?}", manifest.id)));
    }
    if manifest.name.trim().is_empty() {
        return Err(invalid("plugin name must not be empty"));
    }
    Version::parse(&manifest.version)
        .map_err(|e| invalid(format!("plugin {}: invalid version: {e}", manifest.id)))?;
    if manifest.protocol_version != 1 {
        return Err(RpcError::new(
            ErrorCode::IncompatibleVersion,
            "protocol_version must be 1",
        ));
    }
    let api = VersionReq::parse(&manifest.host_api)
        .map_err(|e| invalid(format!("invalid host_api requirement: {e}")))?;
    if !api.matches(&Version::new(0, 1, 0)) {
        return Err(RpcError::new(
            ErrorCode::IncompatibleVersion,
            "host_api does not match 0.1.0",
        ));
    }
    crate::plan::validate_relative(std::path::Path::new(&manifest.entrypoint), "entrypoint")?;
    if let Some(schema) = &manifest.config_schema {
        crate::plan::validate_relative(std::path::Path::new(schema), "config_schema")?;
    }
    if manifest.config_version == 0 {
        return Err(invalid("config_version must be positive"));
    }
    if manifest.reload_policy != "exclusive" {
        return Err(RpcError::new(
            ErrorCode::Unsupported,
            "unsupported reload policy",
        ));
    }
    if let Some(target) = &manifest.target {
        let pair = match target.as_str() {
            "linux-x86_64" => ("linux", "x86_64"),
            "windows-x86_64" => ("windows", "x86_64"),
            "linux-aarch64" => ("linux", "aarch64"),
            _ => {
                return Err(RpcError::new(
                    ErrorCode::Unsupported,
                    "unsupported package target",
                ));
            }
        };
        if pair != (std::env::consts::OS, std::env::consts::ARCH) {
            return Err(RpcError::new(
                ErrorCode::IncompatibleVersion,
                "package target does not match host",
            ));
        }
    }
    for (id, requirement) in &manifest.dependencies {
        if !valid_id(id) {
            return Err(invalid(format!("invalid dependency id {id:?}")));
        }
        VersionReq::parse(requirement)
            .map_err(|e| invalid(format!("invalid dependency requirement for {id}: {e}")))?;
    }
    for capability in manifest
        .capabilities
        .iter()
        .chain(&manifest.required_capabilities)
    {
        crate::permissions::validate_capability(capability)?;
    }
    for required in &manifest.required_capabilities {
        if !manifest.capabilities.contains(required) {
            return Err(invalid(format!(
                "required capability {required} was not requested"
            )));
        }
    }
    Ok(())
}
