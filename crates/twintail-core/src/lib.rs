//! Plugin manifests, explicit permissions, loading plans and bounded process routing.
//!
//! Loading only reads local packages; it never executes them or changes runtime
//! configuration. Dependencies are mandatory; optional dependencies are unsupported.
//! [`ConfigStore`] handles schema-validated configuration revisions independently
//! of package installation and supervision. Validated paths are snapshots:
//! package directories must remain immutable between validation and process spawn.
//!
//! [`Supervisor`] authenticates runtime requests through the bound process pipe.
//! Active instances can register bots, publish/subscribe to events, and call exact
//! adapter generations or explicitly named, SemVer-compatible service providers.

mod manifest;
mod packages;
mod permissions;
mod plan;

pub use manifest::{Manifest, validate_manifest};
pub use permissions::{authorize, granted_capabilities};
pub use plan::{HostConfig, PluginEntry, PluginSpec, load_package, load_plan, validate_specs};

mod supervisor;
pub use supervisor::{Limits, PluginStatus, Supervisor};

pub use packages::{InstalledPackage, PackageInventory, PackageStore};

mod config;
pub use config::{
    ConfigError, ConfigStatus, ConfigStore, MAX_CONFIG_BYTES, PreparedConfig, SecretResolver,
};

mod management;
pub use management::ManagementRequest;
