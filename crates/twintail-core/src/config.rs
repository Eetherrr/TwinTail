//! Bounded, reference-preserving configuration transactions.
//!
//! An OS file lock serializes desired writes and applied acknowledgements. Prepare a
//! candidate, ask the plugin to prepare it, persist desired state, then ask the
//! plugin to commit and record `mark_applied`. A crash always recovers desired
//! state; the separate applied marker never replaces it. Package and data roots
//! must remain exclusively owned by the host while these operations run.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use twintail_protocol::ConfigSnapshot;

/// Maximum serialized schema or configuration values size (one MiB).
pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_FILE_BYTES: usize = MAX_CONFIG_BYTES * 4;
const MAX_DEPTH: usize = 64;
const MAX_NODES: usize = 32768;

/// An environment lookup which never needs process-wide environment mutation.
pub type SecretResolver = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Configuration errors contain field paths and fixed reasons, never input values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    /// JSON pointer or fixed configuration field associated with the failure.
    pub path: String,
    /// Fixed diagnostic category safe for management responses.
    pub reason: &'static str,
}
impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.reason)
    }
}
impl std::error::Error for ConfigError {}
impl From<ConfigError> for twintail_protocol::RpcError {
    fn from(value: ConfigError) -> Self {
        let code = if matches!(
            value.reason,
            "revision conflict" | "revision exhausted" | "configuration transaction busy"
        ) {
            twintail_protocol::ErrorCode::Conflict
        } else {
            twintail_protocol::ErrorCode::InvalidConfig
        };
        Self::new(code, value.to_string())
    }
}
fn error(path: impl Into<String>, reason: &'static str) -> ConfigError {
    ConfigError {
        path: path.into(),
        reason,
    }
}
fn io_error(_: std::io::Error) -> ConfigError {
    error("/", "configuration filesystem operation failed")
}

/// Desired and last acknowledged runtime revisions, stored independently.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigStatus {
    /// Authoritative revision recovered from config.toml.
    pub desired_revision: u64,
    /// Last successful application; absent before the first acknowledgement.
    pub applied_revision: Option<u64>,
}
impl ConfigStatus {
    /// Whether the runtime has acknowledged the current desired configuration.
    pub fn is_applied(&self) -> bool {
        self.applied_revision == Some(self.desired_revision)
    }
}

/// Validated candidate retained between plugin prepare and durable persistence.
///
/// Resolved values are deliberately excluded from Debug and management snapshots.
#[derive(Clone)]
pub struct PreparedConfig {
    snapshot: ConfigSnapshot,
    resolved: Value,
    expected_revision: u64,
}
impl PreparedConfig {
    /// Reference-preserving candidate suitable for management replies.
    pub fn snapshot(&self) -> &ConfigSnapshot {
        &self.snapshot
    }
    /// Resolved candidate for the trusted plugin prepare/initialize channel only.
    pub fn resolved(&self) -> &Value {
        &self.resolved
    }
}

/// Private TOML configuration store with offline Draft 2020-12 validation.
///
/// Methods are synchronous and bounded; contended writes return Conflict. An OS
/// lock protects the revision check and rename across stores and processes.
/// No process action is performed here. Failed validation or writes before rename
/// leave the previous desired file intact. Files use 0600 and directories 0700 on
/// Unix; Windows inherits the host's private data-root ACL.
#[derive(Clone)]
pub struct ConfigStore {
    directory: PathBuf,
    schema: Arc<jsonschema::Validator>,
    schema_value: Value,
    config_version: u32,
    resolver: SecretResolver,
}
impl ConfigStore {
    /// Open a private data directory and compile an optional package-local schema.
    ///
    /// Schema paths must be relative, symlink-free and remain in the canonical
    /// package root. All reference keywords are rejected, including local refs.
    /// Missing configuration reads as revision zero with empty object values.
    /// Errors are sanitized and no existing configuration is overwritten.
    pub fn open(
        directory: &Path,
        package: &Path,
        schema: Option<&str>,
        config_version: u32,
    ) -> Result<Self, ConfigError> {
        if config_version == 0 {
            return Err(error("/config_version", "version must be positive"));
        }
        check_path(directory)?;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(directory).map_err(io_error)?;
        private_dir(directory)?;
        let directory = fs::canonicalize(directory).map_err(io_error)?;
        let schema_value = if let Some(relative) = schema {
            let path = Path::new(relative);
            if path.as_os_str().is_empty()
                || path
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
            {
                return Err(error("/schema", "unsafe schema path"));
            }
            check_path(package)?;
            let package = fs::canonicalize(package).map_err(io_error)?;
            let target = package.join(path);
            check_path(&target)?;
            if !fs::canonicalize(&target)
                .map_err(io_error)?
                .starts_with(&package)
            {
                return Err(error("/schema", "schema escapes package"));
            }
            serde_json::from_slice(&read_bounded(&target, MAX_CONFIG_BYTES)?)
                .map_err(|_| error("/schema", "invalid schema JSON"))?
        } else {
            Value::Bool(true)
        };
        inspect(&schema_value, "", true, &mut 0, 0)?;
        let schema = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema_value)
            .map_err(|_| error("/schema", "invalid Draft 2020-12 schema"))?;
        Ok(Self {
            directory,
            schema: Arc::new(schema),
            schema_value,
            config_version,
            resolver: Arc::new(|name| std::env::var(name).ok()),
        })
    }
    /// Return the validated package schema without resolving configuration secrets.
    pub fn schema(&self) -> &Value {
        &self.schema_value
    }
    /// Replace environment lookup with a deterministic resolver.
    pub fn with_resolver(mut self, resolver: SecretResolver) -> Self {
        self.resolver = resolver;
        self
    }
    /// Replace environment lookup with an immutable map (useful for tests).
    pub fn with_environment(self, environment: BTreeMap<String, String>) -> Self {
        self.with_resolver(Arc::new(move |name| environment.get(name).cloned()))
    }
    /// Read desired state without resolving or returning secret values.
    pub fn read(&self) -> Result<ConfigSnapshot, ConfigError> {
        let path = self.directory.join("config.toml");
        check_path(&path)?;
        if !path.exists() {
            return Ok(ConfigSnapshot {
                config_version: self.config_version,
                revision: 0,
                values: serde_json::json!({}),
            });
        }
        private_file(&path)?;
        let bytes = read_bounded(&path, MAX_FILE_BYTES)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| error("/", "invalid configuration encoding"))?;
        let snapshot: ConfigSnapshot =
            toml::from_str(text).map_err(|_| error("/", "invalid configuration envelope"))?;
        if snapshot.config_version != self.config_version {
            return Err(error("/config_version", "configuration version mismatch"));
        }
        bound_values(&snapshot.values)?;
        inspect(&snapshot.values, "", false, &mut 0, 0)?;
        Ok(snapshot)
    }
    /// Read raw desired state for an authenticated management browser.
    ///
    /// Schema `writeOnly` values must be exact environment-secret references.
    /// An existing inline value returns a sanitized error instead of a redacted
    /// placeholder; use the local CLI to migrate it to an environment reference.
    /// This method does not resolve secrets or require them to be available.
    /// Composition branches are conservatively combined, even when inactive.
    /// Pattern/additional/unevaluated-property schemas may apply more broadly
    /// than validation, so ambiguous secret-bearing structures fail closed.
    pub fn read_for_management(&self) -> Result<ConfigSnapshot, ConfigError> {
        let snapshot = self.read()?;
        management_references(&self.schema_value, &snapshot.values, "", &mut 0, 0)?;
        Ok(snapshot)
    }
    /// Validate browser input while forbidding inline `writeOnly` secrets.
    ///
    /// Exact `{"$secret":"ENV_NAME"}` references are preserved. After the
    /// management policy check, normal environment resolution and schema
    /// validation run. Local CLI validation and persistence remain compatible
    /// with pre-existing inline values through [`Self::validate`].
    pub fn validate_management_input(&self, values: &Value) -> Result<(), ConfigError> {
        bound_values(values)?;
        inspect(values, "", false, &mut 0, 0)?;
        management_references(&self.schema_value, values, "", &mut 0, 0)?;
        self.validate(values)
    }
    /// Read authoritative desired state and resolve it for trusted initialization.
    /// This validates even the empty revision-zero default against the schema.
    pub fn resolve_snapshot(&self) -> Result<ConfigSnapshot, ConfigError> {
        let mut snapshot = self.read()?;
        snapshot.values = self.resolve_validated(&snapshot.values)?;
        Ok(snapshot)
    }
    /// Validate reference values after resolving environment markers.
    ///
    /// Schema errors expose only the instance field path, never validator text.
    pub fn validate(&self, values: &Value) -> Result<(), ConfigError> {
        self.resolve_validated(values).map(|_| ())
    }
    /// Validate an expected revision and construct an unpersisted candidate.
    pub fn prepare(
        &self,
        expected_revision: u64,
        values: Value,
    ) -> Result<PreparedConfig, ConfigError> {
        if self.read()?.revision != expected_revision {
            return Err(error("/revision", "revision conflict"));
        }
        let revision = expected_revision
            .checked_add(1)
            .ok_or_else(|| error("/revision", "revision exhausted"))?;
        let resolved = self.resolve_validated(&values)?;
        Ok(PreparedConfig {
            snapshot: ConfigSnapshot {
                config_version: self.config_version,
                revision,
                values,
            },
            resolved,
            expected_revision,
        })
    }
    /// Atomically persist a prepared desired candidate after locking and rechecking revision.
    ///
    /// The applied sidecar is unchanged. A post-rename directory-sync failure can
    /// mean desired was persisted; callers must reread status before retrying.
    pub fn persist(&mut self, candidate: PreparedConfig) -> Result<ConfigSnapshot, ConfigError> {
        let _lock = self.transaction_lock()?;
        if candidate.snapshot.config_version != self.config_version {
            return Err(error("/config_version", "configuration version mismatch"));
        }
        if self.read()?.revision != candidate.expected_revision {
            return Err(error("/revision", "revision conflict"));
        }
        self.validate(&candidate.snapshot.values)?;
        let text = toml::to_string(&candidate.snapshot)
            .map_err(|_| error("/values", "values cannot be represented in TOML"))?;
        self.atomic_write("config.toml", text.as_bytes())?;
        Ok(candidate.snapshot)
    }
    /// Validate and persist without performing plugin prepare/commit actions.
    pub fn update(
        &mut self,
        expected_revision: u64,
        values: Value,
    ) -> Result<ConfigSnapshot, ConfigError> {
        let candidate = self.prepare(expected_revision, values)?;
        self.persist(candidate)
    }
    /// Record runtime acknowledgement only for the current desired revision.
    pub fn mark_applied(&mut self, revision: u64) -> Result<(), ConfigError> {
        let _lock = self.transaction_lock()?;
        if self.read()?.revision != revision {
            return Err(error("/revision", "revision conflict"));
        }
        let text = toml::to_string(&Applied { revision })
            .map_err(|_| error("/revision", "invalid applied revision"))?;
        self.atomic_write("config.applied.toml", text.as_bytes())
    }
    /// Read desired/applied mismatch, including after process restart.
    pub fn status(&self) -> Result<ConfigStatus, ConfigError> {
        let path = self.directory.join("config.applied.toml");
        check_path(&path)?;
        let applied_revision = if path.exists() {
            private_file(&path)?;
            let bytes = read_bounded(&path, 256)?;
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| error("/revision", "invalid applied marker"))?;
            let applied: Applied =
                toml::from_str(text).map_err(|_| error("/revision", "invalid applied marker"))?;
            Some(applied.revision)
        } else {
            None
        };
        // Read desired after applied: concurrent commits can conservatively
        // appear unapplied, but cannot make a valid marker appear ahead of an
        // older desired snapshot.
        let desired_revision = self.read()?.revision;
        if applied_revision.is_some_and(|revision| revision > desired_revision) {
            return Err(error(
                "/revision",
                "applied marker exceeds desired revision",
            ));
        }
        Ok(ConfigStatus {
            desired_revision,
            applied_revision,
        })
    }
    fn resolve_validated(&self, values: &Value) -> Result<Value, ConfigError> {
        bound_values(values)?;
        inspect(values, "", false, &mut 0, 0)?;
        let resolved = resolve(values, "", &self.resolver, &mut 0)?;
        bound_values(&resolved)?;
        if let Some(issue) = self.schema.iter_errors(&resolved).next() {
            return Err(error(
                issue.instance_path.to_string(),
                "schema validation failed",
            ));
        }
        Ok(resolved)
    }
    fn transaction_lock(&self) -> Result<File, ConfigError> {
        check_path(&self.directory)?;
        private_dir(&self.directory)?;
        let path = self.directory.join("config.lock");
        check_path(&path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path).map_err(io_error)?;
        private_file(&path)?;
        file.try_lock().map_err(|issue| match issue {
            std::fs::TryLockError::WouldBlock => {
                error("/revision", "configuration transaction busy")
            }
            std::fs::TryLockError::Error(issue) => io_error(issue),
        })?;
        // The persistent lock file is deliberately never removed. Closing this
        // handle releases the OS lock, including on process termination.
        Ok(file)
    }
    fn atomic_write(&self, name: &str, bytes: &[u8]) -> Result<(), ConfigError> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(error("/values", "serialized configuration exceeds limit"));
        }
        check_path(&self.directory)?;
        private_dir(&self.directory)?;
        let destination = self.directory.join(name);
        check_path(&destination)?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.directory).map_err(io_error)?;
        private_file(temporary.path())?;
        temporary.write_all(bytes).map_err(io_error)?;
        temporary.as_file().sync_all().map_err(io_error)?;
        temporary
            .persist(&destination)
            .map_err(|_| error("/", "atomic configuration replacement failed"))?;
        #[cfg(unix)]
        File::open(&self.directory)
            .and_then(|file| file.sync_all())
            .map_err(io_error)?;
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Applied {
    revision: u64,
}
// Treat annotations from every possible schema branch as restrictions. Avoid
// invoking a validator with reference placeholders: branches are chosen using
// resolved values, which management reads must never obtain.
fn management_references(
    schema: &Value,
    value: &Value,
    path: &str,
    visits: &mut usize,
    depth: usize,
) -> Result<(), ConfigError> {
    *visits += 1;
    if depth > MAX_DEPTH || *visits > MAX_NODES {
        return Err(error(
            path,
            "management secret policy exceeds complexity limit",
        ));
    }
    let Some(schema) = schema.as_object() else {
        return Ok(());
    };
    if ["$ref", "$dynamicRef", "$recursiveRef"]
        .iter()
        .any(|key| schema.contains_key(*key))
    {
        return Err(error(
            path,
            "management secret policy contains unsupported reference",
        ));
    }
    if schema.get("writeOnly") == Some(&Value::Bool(true)) {
        let reference = value
            .as_object()
            .filter(|object| object.len() == 1)
            .and_then(|object| object.get("$secret"))
            .and_then(Value::as_str);
        if !reference.is_some_and(valid_environment_name) {
            return Err(error(
                path,
                "writeOnly configuration requires an environment secret reference",
            ));
        }
        // This is the entire resolved value at this location. Descending into a
        // marker would incorrectly interpret the host reference as plugin data.
        return Ok(());
    }
    for keyword in ["allOf", "anyOf", "oneOf"] {
        if let Some(branches) = schema.get(keyword).and_then(Value::as_array) {
            for branch in branches {
                management_references(branch, value, path, visits, depth + 1)?;
            }
        }
    }
    for keyword in ["if", "then", "else", "not"] {
        if let Some(branch) = schema.get(keyword) {
            management_references(branch, value, path, visits, depth + 1)?;
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(branches) = schema.get("dependentSchemas").and_then(Value::as_object) {
            for branch in branches.values() {
                management_references(branch, value, path, visits, depth + 1)?;
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        let patterns = schema.get("patternProperties").and_then(Value::as_object);
        for (name, child) in object {
            let field = pointer(path, name);
            let property = properties.and_then(|properties| properties.get(name));
            if let Some(property) = property {
                management_references(property, child, &field, visits, depth + 1)?;
            }
            // Applying patterns without matching is deliberately conservative:
            // pattern engines have differing limits and matching could otherwise
            // become a second unbounded interpretation of untrusted schemas.
            if let Some(patterns) = patterns {
                for pattern in patterns.values() {
                    management_references(pattern, child, &field, visits, depth + 1)?;
                }
            }
            if property.is_none()
                && let Some(additional) = schema.get("additionalProperties")
            {
                management_references(additional, child, &field, visits, depth + 1)?;
            }
            if let Some(unevaluated) = schema.get("unevaluatedProperties") {
                management_references(unevaluated, child, &field, visits, depth + 1)?;
            }
        }
    }
    if let Some(array) = value.as_array() {
        let prefix = schema.get("prefixItems").and_then(Value::as_array);
        for (index, child) in array.iter().enumerate() {
            let field = pointer(path, &index.to_string());
            if let Some(item) = prefix.and_then(|items| items.get(index)) {
                management_references(item, child, &field, visits, depth + 1)?;
            } else if let Some(item) = schema.get("items") {
                management_references(item, child, &field, visits, depth + 1)?;
            }
            for keyword in ["contains", "unevaluatedItems"] {
                if let Some(item) = schema.get(keyword) {
                    management_references(item, child, &field, visits, depth + 1)?;
                }
            }
        }
    }
    Ok(())
}
fn valid_environment_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && name.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || index > 0 && byte.is_ascii_digit()
        })
}
fn bound_values(value: &Value) -> Result<(), ConfigError> {
    if serde_json::to_vec(value)
        .map_err(|_| error("/values", "invalid JSON values"))?
        .len()
        > MAX_CONFIG_BYTES
    {
        return Err(error("/values", "values exceed size limit"));
    }
    Ok(())
}
fn pointer(path: &str, key: &str) -> String {
    format!("{}/{}", path, key.replace('~', "~0").replace('/', "~1"))
}
fn inspect(
    value: &Value,
    path: &str,
    schema: bool,
    nodes: &mut usize,
    depth: usize,
) -> Result<(), ConfigError> {
    *nodes += 1;
    if depth > MAX_DEPTH || *nodes > MAX_NODES {
        return Err(error(path, "structure exceeds complexity limit"));
    }
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let field = pointer(path, key);
                if schema
                    && matches!(
                        key.as_str(),
                        "$ref" | "$dynamicRef" | "$recursiveRef" | "$schema" | "$vocabulary"
                    )
                {
                    // A known dialect declaration is metadata, never fetched.
                    if key == "$schema"
                        && value.as_str() == Some("https://json-schema.org/draft/2020-12/schema")
                    {
                        continue;
                    }
                    return Err(error(
                        field,
                        "schema references and external resources are forbidden",
                    ));
                }
                if !schema
                    && (key == "$redacted"
                        || key == "$secret" && (map.len() != 1 || !value.is_string()))
                {
                    return Err(error(field, "invalid secret or redacted marker"));
                }
                inspect(value, &field, schema, nodes, depth + 1)?;
            }
        }
        Value::Array(array) => {
            for (i, value) in array.iter().enumerate() {
                inspect(
                    value,
                    &pointer(path, &i.to_string()),
                    schema,
                    nodes,
                    depth + 1,
                )?;
            }
        }
        Value::String(text)
            if !schema
                && matches!(
                    text.as_str(),
                    "[REDACTED]" | "<redacted>" | "***" | "********"
                ) =>
        {
            return Err(error(path, "redacted sentinel cannot be persisted"));
        }
        _ => {}
    }
    Ok(())
}
fn resolve(
    value: &Value,
    path: &str,
    resolver: &SecretResolver,
    budget: &mut usize,
) -> Result<Value, ConfigError> {
    *budget += 1;
    if *budget > MAX_CONFIG_BYTES {
        return Err(error(path, "resolved values exceed size limit"));
    }
    match value {
        Value::Object(map) => {
            if let Some(name) = map.get("$secret") {
                let name = name
                    .as_str()
                    .ok_or_else(|| error(path, "invalid secret marker"))?;
                if name.is_empty()
                    || name.len() > 256
                    || !name.bytes().enumerate().all(|(i, b)| {
                        b == b'_' || b.is_ascii_alphabetic() || i > 0 && b.is_ascii_digit()
                    })
                {
                    return Err(error(path, "invalid environment reference"));
                }
                let secret =
                    resolver(name).ok_or_else(|| error(path, "environment secret unavailable"))?;
                *budget = budget.saturating_add(secret.len());
                if *budget > MAX_CONFIG_BYTES {
                    return Err(error(path, "resolved secret exceeds size limit"));
                }
                return Ok(Value::String(secret));
            }
            map.iter()
                .map(|(key, value)| {
                    Ok((
                        key.clone(),
                        resolve(value, &pointer(path, key), resolver, budget)?,
                    ))
                })
                .collect::<Result<serde_json::Map<_, _>, _>>()
                .map(Value::Object)
        }
        Value::Array(array) => array
            .iter()
            .enumerate()
            .map(|(i, value)| resolve(value, &pointer(path, &i.to_string()), resolver, budget))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        _ => Ok(value.clone()),
    }
}
fn check_path(path: &Path) -> Result<(), ConfigError> {
    let mut current = PathBuf::new();
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        if matches!(component, Component::ParentDir) {
            return Err(error("/", "unsafe configuration path"));
        }
        current.push(component);
        if matches!(component, Component::Prefix(_)) {
            // A Windows prefix is not a filesystem root: probing `C:` or
            // `\\\\?\\C:` before its RootDir can fail or use drive-local state.
            // Reject drive-relative paths and inspect the complete root next.
            if !matches!(components.peek(), Some(Component::RootDir)) {
                return Err(error("/", "unsafe configuration path"));
            }
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if is_link_or_reparse_point(&metadata) => {
                return Err(error("/", "symlinks are forbidden"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(e)),
        }
    }
    Ok(())
}
fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Junctions and other reparse points need not report as symlinks.
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}
fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, ConfigError> {
    check_path(path)?;
    let file = File::open(path).map_err(io_error)?;
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(error("/", "configuration path is not a regular file"));
    }
    reject_hard_links(&file.metadata().map_err(io_error)?)?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > limit {
        return Err(error("/", "file exceeds size limit"));
    }
    Ok(bytes)
}
fn private_dir(path: &Path) -> Result<(), ConfigError> {
    if !fs::symlink_metadata(path).map_err(io_error)?.is_dir() {
        return Err(error("/", "configuration root is not a directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
    }
    Ok(())
}
fn reject_hard_links(metadata: &fs::Metadata) -> Result<(), ConfigError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(error("/", "hard-linked configuration files are forbidden"));
        }
    }
    #[cfg(not(unix))]
    let _ = metadata;
    Ok(())
}
fn private_file(path: &Path) -> Result<(), ConfigError> {
    reject_hard_links(&fs::symlink_metadata(path).map_err(io_error)?)?;
    if !fs::symlink_metadata(path).map_err(io_error)?.is_file() {
        return Err(error("/", "configuration path is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(io_error)?;
    }
    Ok(())
}
