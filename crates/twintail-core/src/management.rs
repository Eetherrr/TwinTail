//! Authenticated bounded handoff to the host-owned management controller.
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::oneshot;
use twintail_protocol::{ErrorCode, RpcError};

/// An authenticated plugin management request accepted by core routing.
///
/// The host must recheck [`Self::is_active`] before admission, including after any
/// intermediate queue, and return mutation operation ACKs immediately.
/// Waiting for self-stop/reload completion would deadlock lifecycle coordination.
/// Configuration results must retain secret references and never resolved values.
/// Receiving this request does not authorize arbitrary methods: core has already
/// applied the exact safe-method allowlist and read/write capability check.
pub struct ManagementRequest {
    pub(crate) active: Arc<AtomicBool>,
    /// Stable identity authenticated by the process pipe.
    pub plugin_id: String,
    /// Host-assigned instance identity, including its generation binding.
    pub instance_id: String,
    /// Monotonic source generation.
    pub generation: u64,
    /// Exact approved controller method.
    pub method: String,
    /// Approved method-specific payload.
    pub payload: Value,
    /// Absolute admission deadline. Never execute expired requests.
    pub deadline: tokio::time::Instant,
    /// Response channel; closed receivers indicate cancellation/retirement.
    pub reply: oneshot::Sender<Result<Value, RpcError>>,
}

impl ManagementRequest {
    /// Revalidates the source lease without borrowing the lifecycle controller.
    /// False after quiesce/retirement, deadline expiry, or cancellation of the waiter.
    /// The host must check immediately before operation admission; admitted side
    /// effects are not rolled back if the source subsequently disconnects.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
            && self.deadline > tokio::time::Instant::now()
            && !self.reply.is_closed()
    }
}

pub(crate) fn capability(method: &str, payload: &Value) -> Result<&'static str, RpcError> {
    if !payload.is_object() {
        return Err(RpcError::new(
            ErrorCode::InvalidRequest,
            "management payload must be an object",
        ));
    }
    match method {
        "plugins.list"
        | "plugins.status"
        | "plugins.installed"
        | "plugins.inspect"
        | "operations.list"
        | "operations.get"
        | "config.get"
        | "config.schema"
        | "config.validate"
        | "management.overview"
        | "diagnostics.logs"
        | "adapter.status" => Ok("management.read"),
        "plugins.install" | "plugins.enable" | "plugins.disable" | "plugins.uninstall"
        | "plugins.reload" | "plugins.start" | "plugins.stop" | "config.update" => {
            Ok("management.write")
        }
        _ => Err(RpcError::new(
            ErrorCode::PermissionDenied,
            "management method is not permitted through plugin bridge",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn safe_management_methods_have_exact_separate_capabilities() {
        for method in [
            "plugins.list",
            "plugins.status",
            "plugins.installed",
            "plugins.inspect",
            "operations.list",
            "operations.get",
            "config.get",
            "config.schema",
            "config.validate",
            "management.overview",
            "diagnostics.logs",
            "adapter.status",
        ] {
            assert_eq!(
                capability(method, &serde_json::json!({})).unwrap(),
                "management.read"
            );
        }
        for method in [
            "plugins.install",
            "plugins.enable",
            "plugins.disable",
            "plugins.uninstall",
            "plugins.reload",
            "plugins.start",
            "plugins.stop",
            "config.update",
        ] {
            assert_eq!(
                capability(method, &serde_json::json!({})).unwrap(),
                "management.write"
            );
        }
        for method in [
            "plugins.call",
            "management.call",
            "config.resolve",
            "host.read",
            "actions.call",
            "plugins.list.extra",
        ] {
            assert_eq!(
                capability(method, &serde_json::json!({})).unwrap_err().code,
                ErrorCode::PermissionDenied
            );
        }
    }
}
