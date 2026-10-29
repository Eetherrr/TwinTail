use twintail_protocol::{ErrorCode, RpcError};

use crate::{Manifest, validate_manifest};

pub(crate) fn validate_capability(capability: &str) -> Result<(), RpcError> {
    match capability {
        "host.health" | "host.read" | "bots.register" | "events.publish" | "events.subscribe"
        | "actions.call" | "services.provide" | "services.call" | "storage.private"
        | "management.read" | "management.write" => Ok(()),
        _ => Err(RpcError::new(
            ErrorCode::Unsupported,
            format!("unsupported capability {capability:?}"),
        )),
    }
}

/// Compute the exact intersection of requested capabilities and explicit grants.
///
/// The result is unique and sorted lexicographically. Empty grants deny all.
/// Wildcards, prefixes and unknown names are rejected. Routing capabilities are
/// exact method-level grants; management grants authorize only allowlisted bridge
/// methods. `host.read` and `storage.private` remain reserved. This function performs no I/O or runtime changes.
///
/// # Errors
/// Propagates manifest validation errors, returns `Unsupported` for unknown grant
/// names, and `PermissionDenied` if a required capability is not explicitly granted.
pub fn granted_capabilities(
    manifest: &Manifest,
    grants: &[String],
) -> Result<Vec<String>, RpcError> {
    validate_manifest(manifest)?;
    for grant in grants {
        validate_capability(grant)?;
    }
    let granted: std::collections::BTreeSet<_> = manifest
        .capabilities
        .iter()
        .filter(|capability| grants.contains(capability))
        .cloned()
        .collect();
    for required in &manifest.required_capabilities {
        if !granted.contains(required) {
            return Err(RpcError::new(
                ErrorCode::PermissionDenied,
                format!("plugin {} requires capability {required}", manifest.id),
            ));
        }
    }
    Ok(granted.into_iter().collect())
}

/// Authorize one exact capability against a previously computed grant set.
///
/// No wildcard or prefix expansion occurs. The grant set should be produced by
/// [`granted_capabilities`]; this read-only check does not authenticate a caller.
///
/// # Errors
/// Returns `Unsupported` for an unknown requested capability and
/// `PermissionDenied` when the exact capability is absent from `granted`.
pub fn authorize(granted: &[String], capability: &str) -> Result<(), RpcError> {
    validate_capability(capability)?;
    if granted.iter().any(|grant| grant == capability) {
        Ok(())
    } else {
        Err(RpcError::new(
            ErrorCode::PermissionDenied,
            format!("capability {capability} was not granted"),
        ))
    }
}
