//! Optional wire 1.2 configuration transaction payloads.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An immutable configuration revision supplied by the host.
///
/// Values may contain sensitive data or secret references. Consumers must never
/// write full snapshots into diagnostics, logs or externally visible errors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigSnapshot {
    /// Plugin-defined schema/migration version; independent of package version.
    pub config_version: u32,
    /// Host-assigned monotonically increasing revision.
    pub revision: u64,
    /// Configuration object validated by the host before delivery.
    pub values: Value,
}

/// Validate one candidate without changing the plugin's applied configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigPrepare {
    /// Required current applied revision; zero if no snapshot was initialized.
    pub expected_revision: u64,
    /// Candidate revision to validate and retain until commit.
    pub snapshot: ConfigSnapshot,
}

/// Apply exactly the candidate previously accepted by `config.prepare`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigCommit {
    /// Revision of the prepared candidate, already made durable by the host.
    pub revision: u64,
}
