//! P2 routing payloads. The host binds caller identity to the process pipe.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An account attached to one host-assigned adapter instance (including its generation).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub struct BotRef {
    /// Host-assigned adapter instance ID; never a plugin-selected routing identity.
    pub adapter_instance_id: String,
    /// Protocol account identifier represented as a string without precision loss.
    pub self_id: String,
}
/// Register an account under the authenticated adapter instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RegisterBot {
    /// Protocol account identifier.
    pub self_id: String,
}
/// Replace the caller's exact event-type subscription set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Subscribe {
    /// Exact event types; an empty list unsubscribes. No wildcard expansion.
    pub event_types: Vec<String>,
}
/// Publish an event for a previously registered account.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PublishEvent {
    /// Account registered by this process instance.
    pub self_id: String,
    /// Protocol namespace, for example `mock.v1`.
    pub protocol: String,
    /// Event type used for subscription matching.
    pub event_type: String,
    /// Arbitrary protocol payload; message segments and unknown fields survive.
    pub payload: Value,
}
/// Host-stamped event delivered to each matching subscriber, without replay guarantees.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EventEnvelope {
    /// Host-local event correlation ID; not an upstream deduplication key.
    pub event_id: String,
    /// Authenticated source account and adapter instance.
    pub bot: BotRef,
    /// Host receive time since Unix epoch in milliseconds.
    pub received_at_ms: u64,
    /// Protocol namespace.
    pub protocol: String,
    /// Exact subscription event type.
    pub event_type: String,
    /// Original protocol JSON, including extension fields.
    pub payload: Value,
}
/// Admission counts for a best-effort fan-out; admission does not imply handler completion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeliveryReport {
    /// Number admitted to bounded subscriber queues.
    pub delivered: u32,
    /// Number rejected because a matching subscriber could not accept delivery.
    pub dropped: u32,
}
/// Forward an action to the exact adapter instance bound by `bot`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionCall {
    /// Target adapter instance and account.
    pub bot: BotRef,
    /// Protocol action name. No implicit retry is permitted.
    pub action: String,
    /// Action parameters, preserving protocol extensions.
    pub params: Value,
}
/// Register a named service under the authenticated provider instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ServiceRegistration {
    /// Exact service name, scoped to the provider plugin.
    pub name: String,
    /// SemVer interface version.
    pub version: String,
}
/// Resolve a provider's current registration once, then bind the call to that generation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ServiceCall {
    /// Stable provider plugin ID.
    pub provider: String,
    /// Registered service name.
    pub service: String,
    /// Required SemVer interface range.
    pub version_req: String,
    /// Service-specific method name.
    pub method: String,
    /// Service-specific arguments.
    pub params: Value,
}
