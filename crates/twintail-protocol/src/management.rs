//! Authenticated plugin-to-host management bridge payloads.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A safe management method invoked through `management.call`.
///
/// The host authenticates the source pipe and applies exact `management.read`
/// or `management.write` grants. Unknown methods and arbitrary plugin RPC calls
/// are rejected. Mutations return operation admission acknowledgements; callers
/// query operations for completion, especially when changing their own plugin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagementCall {
    /// Exact allowlisted host management method.
    pub method: String,
    /// Method-specific JSON object. Configuration reads contain secret references,
    /// never resolved values. No source identity supplied here is trusted.
    pub payload: Value,
}
