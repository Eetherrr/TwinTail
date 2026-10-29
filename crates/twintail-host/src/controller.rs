//! Serialized management actor, durable operations and configuration coordination.
use crate::management::ManagementCall;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, watch};
use twintail_core::{
    ConfigStore, HostConfig, ManagementRequest, PackageStore, PluginEntry, PluginSpec, Supervisor,
    load_package, validate_specs,
};
use twintail_protocol::{Body, ConfigSnapshot, ErrorCode, Message, Outcome, RpcError, VERSION};
type Result<T> = std::result::Result<T, RpcError>;
const MAX_OPERATIONS: usize = 128;
const MAX_JOURNAL: u64 = 256 * 1024;
#[derive(Clone, Default)]
struct View {
    operations: VecDeque<Value>,
    plugins: Value,
    specs: Vec<PluginSpec>,
}
type Shared = Arc<Mutex<View>>;
pub(crate) struct Controller {
    supervisor: Supervisor,
    data: PathBuf,
    packages: PackageStore,
    install: PathBuf,
    plan: HostConfig,
    shared: Shared,
    sequence: u64,
    bridge: Option<mpsc::Receiver<ManagementRequest>>,
}
struct Incoming {
    request: Message,
    reply: Reply,
}
enum Reply {
    Local(tokio::sync::oneshot::Sender<Message>),
    Plugin(ManagementRequest),
}
impl Reply {
    fn is_plugin(&self) -> bool {
        matches!(self, Self::Plugin(_))
    }
    fn plugin_id(&self) -> Option<&str> {
        match self {
            Self::Local(_) => None,
            Self::Plugin(request) => Some(&request.plugin_id),
        }
    }
    fn active(&self) -> bool {
        match self {
            Self::Local(_) => true,
            Self::Plugin(request) => request.is_active(),
        }
    }
    fn actor(&self) -> String {
        match self {
            Self::Local(_) => "local-user".into(),
            Self::Plugin(request) => format!("plugin:{}", request.plugin_id),
        }
    }
    fn send(self, message: Message) -> std::result::Result<(), ()> {
        match self {
            Self::Local(reply) => reply.send(message).map_err(|_| ()),
            Self::Plugin(request) => {
                let result = if !request.is_active() {
                    Err(error(
                        ErrorCode::Cancelled,
                        "management caller is no longer active",
                    ))
                } else {
                    match message.body {
                        Body::Response { outcome, .. } => match outcome {
                            Outcome::Ok(value) => Ok(value),
                            Outcome::Err(e) => Err(e),
                        },
                        _ => Err(error(
                            ErrorCode::InvalidRequest,
                            "invalid management response",
                        )),
                    }
                };
                request
                    .reply
                    .send(result.map_err(|e| error(e.code, "management request failed")))
                    .map_err(|_| ())
            }
        }
    }
}
async fn receive(
    calls: &mut mpsc::Receiver<ManagementCall>,
    bridge: &mut mpsc::Receiver<ManagementRequest>,
) -> Option<Incoming> {
    tokio::select! {
        call = calls.recv() => call.map(|call| Incoming {request:call.request,reply:Reply::Local(call.reply)}),
        Some(request) = bridge.recv() => Some(Incoming {
            request:Message::request("plugin-management",request.method.clone(),request.payload.clone()),
            reply:Reply::Plugin(request),
        }),
    }
}
fn dispatch_query(
    reply: Reply,
    request: (String, String, Value),
    shared: &Shared,
    data: &Path,
    install: &Path,
    permits: &Arc<tokio::sync::Semaphore>,
) {
    let (id, method, payload) = request;
    let permit = match permits.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            let _ = reply.send(response(
                id,
                Err(error(
                    ErrorCode::Overloaded,
                    "management query limit reached",
                )),
            ));
            return;
        }
    };
    let shared = shared.clone();
    let data = data.to_owned();
    let install = install.to_owned();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let result = if reply.active() {
            query(
                &shared,
                &data,
                &install,
                &method,
                &payload,
                reply.plugin_id(),
            )
        } else {
            Err(error(
                ErrorCode::Cancelled,
                "management caller is no longer active",
            ))
        };
        let _ = reply.send(response(id, result));
    });
}
fn error(code: ErrorCode, message: &str) -> RpcError {
    RpcError::new(code, message)
}
fn disk(_: impl std::fmt::Display) -> RpcError {
    error(
        ErrorCode::InvalidConfig,
        "management filesystem operation failed",
    )
}
fn field<'a>(payload: &'a Value, key: &str) -> Result<&'a str> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control))
        .ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "missing or invalid management field",
            )
        })
}
fn response(id: String, result: Result<Value>) -> Message {
    Message::response(
        id,
        match result {
            Ok(v) => Outcome::Ok(v),
            Err(e) => Outcome::Err(e),
        },
    )
}
fn decode(message: Message) -> Result<(String, String, Value)> {
    if message.protocol_version.major != VERSION.major {
        return Err(error(
            ErrorCode::IncompatibleVersion,
            "management version mismatch",
        ));
    }
    match message.body {
        Body::Request {
            request_id,
            method,
            payload,
        } if !request_id.is_empty() && request_id.len() <= 128 => Ok((request_id, method, payload)),
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "management expects a valid request",
        )),
    }
}
fn private_existing(path: &Path, directory: bool) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(io::Error::other("unsafe management state path"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o7777 != if directory { 0o700 } else { 0o600 }
        {
            return Err(io::Error::other("management state must be private"));
        }
    }
    Ok(())
}
fn atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("state path requires parent"))?;
    private_existing(parent, true)?;
    if fs::symlink_metadata(path).is_ok() {
        private_existing(path, false)?;
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
fn persist_journal(data: &Path, shared: &Shared) -> Result<()> {
    let view = shared.lock().unwrap();
    let bytes = serde_json::to_vec(&view.operations).map_err(disk)?;
    if bytes.len() as u64 > MAX_JOURNAL {
        return Err(error(ErrorCode::Overloaded, "operation journal full"));
    }
    atomic(&data.join("operations.json"), &bytes).map_err(disk)
}
impl Controller {
    pub(crate) fn new(
        mut supervisor: Supervisor,
        data: PathBuf,
        install: PathBuf,
    ) -> io::Result<Self> {
        let packages = PackageStore::new(&install).map_err(io::Error::other)?;
        let path = data.join("host.toml");
        let plan = if path.exists() {
            toml::from_str(&fs::read_to_string(&path)?).map_err(io::Error::other)?
        } else {
            HostConfig::default()
        };
        let path = data.join("operations.json");
        let mut operations: VecDeque<Value> = if path.exists() {
            private_existing(&path, false)?;
            if fs::metadata(&path)?.len() > MAX_JOURNAL {
                return Err(io::Error::other("operation journal exceeds limit"));
            }
            let mut bytes = Vec::new();
            fs::File::open(&path)?
                .take(MAX_JOURNAL + 1)
                .read_to_end(&mut bytes)?;
            serde_json::from_slice(&bytes).map_err(io::Error::other)?
        } else {
            VecDeque::new()
        };
        if operations.len() > MAX_OPERATIONS {
            return Err(io::Error::other("operation journal exceeds record limit"));
        }
        for operation in &mut operations {
            if matches!(operation["state"].as_str(), Some("running" | "pending")) {
                operation["state"] = json!("interrupted");
                operation["stage"] = json!("interrupted");
                operation["result"] = json!({"status":"interrupted","needs_recovery":true});
            }
        }
        let shared = Arc::new(Mutex::new(View {
            operations,
            ..View::default()
        }));
        persist_journal(&data, &shared).map_err(io::Error::other)?;
        let (sender, bridge) = mpsc::channel(32);
        supervisor.set_management_sender(sender);
        let mut this = Self {
            supervisor,
            data,
            packages,
            install,
            plan,
            shared,
            sequence: 0,
            bridge: Some(bridge),
        };
        this.refresh();
        Ok(this)
    }
    fn refresh(&mut self) {
        let statuses = self.supervisor.list();
        let specs = statuses
            .iter()
            .filter_map(|s| self.supervisor.spec(&s.id).cloned())
            .collect();
        let mut view = self.shared.lock().unwrap();
        view.plugins = serde_json::to_value(statuses).unwrap();
        view.specs = specs;
    }
    fn store(&self, spec: &PluginSpec) -> Result<ConfigStore> {
        open_store(&self.data, spec)
    }
    fn specs(&self) -> Vec<PluginSpec> {
        self.supervisor
            .list()
            .iter()
            .filter_map(|s| self.supervisor.spec(&s.id).cloned())
            .collect()
    }
    fn write_plan(&mut self, plan: HostConfig) -> Result<()> {
        let text = toml::to_string(&plan).map_err(disk)?;
        atomic(&self.data.join("host.toml"), text.as_bytes()).map_err(disk)?;
        self.plan = plan;
        Ok(())
    }
    fn stage(&self, operation: &str, stage: &str) -> Result<()> {
        if let Some(op) = self
            .shared
            .lock()
            .unwrap()
            .operations
            .iter_mut()
            .find(|op| op["id"] == operation)
        {
            op["stage"] = json!(stage);
        }
        persist_journal(&self.data, &self.shared)
    }
    fn admit(&mut self, method: &str, payload: &Value, actor: &str) -> Result<(String, bool)> {
        if let Some(id) = payload.get("id")
            && !id.as_str().is_some_and(|id| {
                !id.is_empty()
                    && id.len() <= 128
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            })
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "invalid plugin identifier",
            ));
        }
        let fingerprint = fingerprint(payload)?;
        let before_version = payload
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| self.supervisor.spec(id))
            .map(|spec| spec.manifest.version.clone());
        let request_key = match payload.get("request_key") {
            None => None,
            Some(Value::String(key))
                if !key.is_empty()
                    && key.len() <= 128
                    && key
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) =>
            {
                Some(key.clone())
            }
            _ => return Err(error(ErrorCode::InvalidRequest, "invalid request_key")),
        };
        let mut view = self.shared.lock().unwrap();
        if let Some(key) = &request_key
            && let Some(op) = view
                .operations
                .iter()
                .find(|op| op["request_key"] == key.as_str())
        {
            if op["actor"] != actor
                || op["method"] != method
                || op["target"] != payload.get("id").cloned().unwrap_or(Value::Null)
                || op["fingerprint"] != fingerprint
            {
                return Err(error(
                    ErrorCode::Conflict,
                    "request_key already belongs to a different request",
                ));
            }
            return Ok((op["id"].as_str().unwrap().to_owned(), false));
        }
        self.sequence += 1;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let id = format!("op-{now:x}-{:x}", self.sequence);
        if view.operations.len() >= MAX_OPERATIONS {
            view.operations.pop_front();
        }
        view.operations.push_back(json!({"id":id,"state":"running","stage":"accepted","method":method,"target":payload.get("id"),"actor":actor,"request_key":request_key,"fingerprint":fingerprint,"before_version":before_version,"expected_revision":payload.get("expected_revision").and_then(Value::as_u64),"created_unix_ms":now/1_000_000,"result":null}));
        drop(view);
        persist_journal(&self.data, &self.shared)?;
        Ok((id, true))
    }
    fn finish(&self, id: &str, result: &Result<Value>) -> Result<()> {
        let mut view = self.shared.lock().unwrap();
        if let Some(op) = view.operations.iter_mut().find(|op| op["id"] == id) {
            let (state, summary) = match result {
                Ok(value) => {
                    let outcome = value
                        .get("outcome")
                        .and_then(Value::as_str)
                        .filter(|s| ["reloaded", "rolled_back", "failed"].contains(s));
                    (
                        if outcome == Some("failed") {
                            "failed"
                        } else {
                            "completed"
                        },
                        json!({"status":"ok","outcome":outcome,"revision":value.get("revision").and_then(Value::as_u64),"needs_recovery":value.get("needs_recovery").and_then(Value::as_bool).unwrap_or(false)}),
                    )
                }
                Err(e) if e.code == ErrorCode::Cancelled => (
                    "interrupted",
                    json!({"status":"interrupted","needs_recovery":true}),
                ),
                Err(e) => (
                    "failed",
                    json!({"status":"error","code":e.code,"needs_recovery":true}),
                ),
            };
            op["state"] = json!(state);
            op["stage"] = json!(state);
            op["result"] = summary;
        }
        drop(view);
        persist_journal(&self.data, &self.shared)
    }
    async fn acknowledge(&mut self, id: &str, store: &ConfigStore) -> Result<u64> {
        let snapshot = store.read().map_err(RpcError::from)?;
        let generation = self
            .supervisor
            .list()
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| error(ErrorCode::NotFound, "unknown plugin"))?
            .generation;
        if !self.supervisor.supports_config(id) {
            return Err(error(
                ErrorCode::Unsupported,
                "plugin does not support configuration acknowledgement",
            ));
        }
        let status = self
            .supervisor
            .call(id, "config.status".into(), json!({}))
            .await?;
        if status.get("applied_revision").and_then(Value::as_u64) != Some(snapshot.revision)
            || status.get("config_version").and_then(Value::as_u64)
                != Some(snapshot.config_version as u64)
        {
            return Err(error(
                ErrorCode::Conflict,
                "runtime configuration acknowledgement mismatch",
            ));
        }
        self.supervisor.confirm_config(
            id,
            generation,
            Some(store.resolve_snapshot().map_err(RpcError::from)?),
        )?;
        Ok(snapshot.revision)
    }
    async fn confirmed(&mut self, id: &str, store: &mut ConfigStore) -> Result<()> {
        let revision = self.acknowledge(id, store).await?;
        store.mark_applied(revision).map_err(Into::into)
    }
    async fn start(&mut self, id: &str) -> Result<Value> {
        let spec = self
            .supervisor
            .spec(id)
            .cloned()
            .ok_or_else(|| error(ErrorCode::NotFound, "unknown plugin"))?;
        let mut store = self.store(&spec)?;
        self.supervisor
            .set_config(id, Some(store.resolve_snapshot().map_err(RpcError::from)?))?;
        let value = self.supervisor.start(id).await?;
        // Legacy minor-zero plugins have no configuration RPC. They retain startup
        // compatibility; only an explicit acknowledgement records applied state.
        match self.confirmed(id, &mut store).await {
            Ok(()) => {}
            Err(_)
                if spec.manifest.config_schema.is_none()
                    && store.read().map_err(RpcError::from)?.revision == 0 => {}
            Err(e) => {
                let _ = self.supervisor.stop(id).await;
                return Err(e);
            }
        }
        Ok(value)
    }
    pub(crate) async fn bootstrap(&mut self) {
        for spec in self.specs() {
            let payload = json!({"id":spec.manifest.id});
            if let Ok((operation, _)) = self.admit("plugins.start", &payload, "local-user") {
                let result = self.execute(&operation, "plugins.start", payload).await;
                let _ = self.finish(&operation, &result);
            }
        }
        self.refresh();
    }
    async fn execute(&mut self, operation: &str, method: &str, payload: Value) -> Result<Value> {
        if method == "adapter.status" {
            let id = field(&payload, "id")?;
            let value = self
                .supervisor
                .call(id, "connection.status".into(), json!({}))
                .await?;
            return Ok(
                json!({"connected":value["connected"].as_bool(),"generation":value["generation"].as_u64(),"connection_attempts":value["connection_attempts"].as_u64(),"pending_actions":value["pending_actions"].as_u64()}),
            );
        }
        self.stage(operation, "validating")?;
        if method == "plugins.install" {
            self.stage(operation, "installing")?;
            let packages = self.packages.clone();
            let source = field(&payload, "path")?.to_owned();
            let installed = tokio::task::spawn_blocking(move || packages.install_local(source))
                .await
                .map_err(disk)??;
            return serde_json::to_value(installed).map_err(disk);
        }
        let id = field(&payload, "id")?.to_owned();
        match method {
            "plugins.start" => {
                self.stage(operation, "starting")?;
                self.start(&id).await
            }
            "plugins.stop" => {
                self.stage(operation, "stopping")?;
                self.supervisor.stop(&id).await
            }
            "plugins.call" => {
                let method = field(&payload, "method")?;
                if method.len() > 128 {
                    return Err(error(ErrorCode::InvalidRequest, "invalid call method"));
                }
                self.supervisor
                    .call(
                        &id,
                        method.to_owned(),
                        payload.get("payload").cloned().unwrap_or(Value::Null),
                    )
                    .await
            }
            "plugins.enable" => {
                if self.supervisor.spec(&id).is_some() {
                    return Err(error(ErrorCode::Conflict, "plugin already enabled"));
                }
                let version = field(&payload, "version")?;
                let packages = self.packages.clone();
                let verify_id = id.clone();
                let verify_version = version.to_owned();
                let installed = tokio::task::spawn_blocking(move || {
                    packages.verify(&verify_id, &verify_version)
                })
                .await
                .map_err(disk)??;
                let grants: Vec<String> = serde_json::from_value(
                    payload
                        .get("grants")
                        .cloned()
                        .ok_or_else(|| error(ErrorCode::InvalidRequest, "missing grants"))?,
                )
                .map_err(|_| error(ErrorCode::InvalidRequest, "grants must be a string array"))?;
                let spec = load_package(&installed.path, &grants)?;
                let mut specs = self.specs();
                specs.push(spec.clone());
                validate_specs(specs)?;
                self.store(&spec)?
                    .resolve_snapshot()
                    .map_err(RpcError::from)?;
                let old_plan = self.plan.clone();
                let mut plan = old_plan.clone();
                plan.plugins.push(PluginEntry {
                    path: spec
                        .package_dir
                        .strip_prefix(&self.install)
                        .map_err(disk)?
                        .to_owned(),
                    grants,
                });
                self.stage(operation, "persisting_plan")?;
                self.write_plan(plan)?;
                if let Err(e) = self.supervisor.add(spec) {
                    self.write_plan(old_plan)?;
                    return Err(e);
                }
                self.stage(operation, "starting")?;
                self.start(&id).await
            }
            "plugins.disable" => {
                let old = self
                    .supervisor
                    .spec(&id)
                    .cloned()
                    .ok_or_else(|| error(ErrorCode::NotFound, "unknown plugin"))?;
                let mut specs = self.specs();
                specs.retain(|s| s.manifest.id != id);
                validate_specs(specs)?;
                let mut plan = self.plan.clone();
                plan.plugins
                    .retain(|entry| self.install.join(&entry.path) != old.package_dir);
                self.stage(operation, "stopping")?;
                self.supervisor.stop(&id).await?;
                self.stage(operation, "persisting_plan")?;
                self.supervisor.remove(&id)?;
                if let Err(e) = self.write_plan(plan) {
                    self.supervisor.add(old)?;
                    let _ = self.start(&id).await;
                    return Err(e);
                }
                Ok(json!({"id":id,"state":"disabled"}))
            }
            "plugins.uninstall" => {
                let version = field(&payload, "version")?;
                let path = self.install.join(&id).join(version);
                if self.specs().iter().any(|spec| spec.package_dir == path)
                    || self
                        .plan
                        .plugins
                        .iter()
                        .any(|entry| self.install.join(&entry.path) == path)
                {
                    return Err(error(ErrorCode::Conflict, "package version is enabled"));
                }
                self.stage(operation, "uninstalling")?;
                let packages = self.packages.clone();
                let remove_id = id.clone();
                let remove_version = version.to_owned();
                tokio::task::spawn_blocking(move || {
                    packages.uninstall(&remove_id, &remove_version)
                })
                .await
                .map_err(disk)??;
                Ok(json!({"id":id,"version":version,"state":"uninstalled"}))
            }
            "plugins.reload" => {
                let old = self
                    .supervisor
                    .spec(&id)
                    .cloned()
                    .ok_or_else(|| error(ErrorCode::NotFound, "unknown plugin"))?;
                let packages = self.packages.clone();
                let verify_id = id.clone();
                let verify_version = field(&payload, "version")?.to_owned();
                let installed = tokio::task::spawn_blocking(move || {
                    packages.verify(&verify_id, &verify_version)
                })
                .await
                .map_err(disk)??;
                let old_config = self.supervisor.active_config(&id).cloned();
                let candidate = load_package(&installed.path, &old.grants)?;
                let mut store = self.store(&candidate)?;
                let snapshot = store.resolve_snapshot().map_err(RpcError::from)?;
                self.stage(operation, "reloading")?;
                self.supervisor.set_config(&id, Some(snapshot))?;
                let value = self.supervisor.reload(&id, candidate.clone()).await?;
                if value.get("outcome").and_then(Value::as_str) == Some("reloaded") {
                    let finalized: Result<Option<u64>> = async {
                        self.stage(operation, "confirming_config")?;
                        let revision = match self.acknowledge(&id, &store).await {
                            Ok(revision) => Some(revision),
                            Err(_)
                                if candidate.manifest.config_schema.is_none()
                                    && store.read().map_err(RpcError::from)?.revision == 0 =>
                            {
                                None
                            }
                            Err(e) => return Err(e),
                        };
                        let mut plan = self.plan.clone();
                        for entry in &mut plan.plugins {
                            if self.install.join(&entry.path) == old.package_dir {
                                entry.path = candidate
                                    .package_dir
                                    .strip_prefix(&self.install)
                                    .map_err(disk)?
                                    .to_owned();
                            }
                        }
                        self.stage(operation, "persisting_plan")?;
                        self.write_plan(plan)?;
                        Ok(revision)
                    }
                    .await;
                    match finalized {
                        Ok(Some(revision)) => {
                            store.mark_applied(revision).map_err(RpcError::from)?
                        }
                        Ok(None) => {}
                        Err(_) => {
                            let _ = self.stage(operation, "rolling_back_config");
                            self.supervisor.set_config(&id, old_config)?;
                            let rollback = self.supervisor.reload(&id, old).await?;
                            return Ok(
                                json!({"id":id,"outcome":if rollback["outcome"] == "reloaded" { "rolled_back" } else { "failed" },"reason":"management_finalize_failed","needs_recovery":true}),
                            );
                        }
                    }
                }
                Ok(value)
            }
            "config.update" => {
                let spec = self
                    .supervisor
                    .spec(&id)
                    .cloned()
                    .ok_or_else(|| error(ErrorCode::NotFound, "unknown plugin"))?;
                let mut store = self.store(&spec)?;
                let expected = payload
                    .get("expected_revision")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| error(ErrorCode::InvalidRequest, "missing expected_revision"))?;
                let prepared = store
                    .prepare(
                        expected,
                        payload
                            .get("values")
                            .cloned()
                            .ok_or_else(|| error(ErrorCode::InvalidRequest, "missing values"))?,
                    )
                    .map_err(RpcError::from)?;
                let mut resolved: ConfigSnapshot = prepared.snapshot().clone();
                resolved.values = prepared.resolved().clone();
                let active = self
                    .supervisor
                    .list()
                    .iter()
                    .any(|s| s.id == id && s.state == "active");
                let prepared_revision = prepared.snapshot().revision;
                let transaction: Result<ConfigSnapshot> = async {
                    if active {
                        self.stage(operation, "preparing")?;
                        self.supervisor
                            .call(
                                &id,
                                "config.prepare".into(),
                                json!({"expected_revision":expected,"snapshot":resolved}),
                            )
                            .await?;
                    }
                    self.stage(operation, "persisting_config")?;
                    let snapshot = store.persist(prepared).map_err(RpcError::from)?;
                    self.supervisor.set_config(&id, Some(resolved))?;
                    if active {
                        self.stage(operation, "committing")?;
                        self.supervisor
                            .call(
                                &id,
                                "config.commit".into(),
                                json!({"revision":snapshot.revision}),
                            )
                            .await?;
                        self.confirmed(&id, &mut store).await?;
                    }
                    Ok(snapshot)
                }
                .await;
                let snapshot = match transaction {
                    Ok(snapshot) => snapshot,
                    Err(e) => {
                        if active {
                            let _ = self
                                .supervisor
                                .call(
                                    &id,
                                    "config.abort".into(),
                                    json!({"revision":prepared_revision}),
                                )
                                .await;
                        }
                        return Err(e);
                    }
                };
                Ok(
                    json!({"id":id,"revision":snapshot.revision,"status":store.status().map_err(RpcError::from)?}),
                )
            }
            _ => Err(error(
                ErrorCode::Unsupported,
                "management method not implemented",
            )),
        }
    }
    pub(crate) async fn serve(
        mut self,
        mut calls: mpsc::Receiver<ManagementCall>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let mut bridge = self.bridge.take().expect("management bridge initialized");
        let shared = self.shared.clone();
        let data = self.data.clone();
        let install = self.install.clone();
        if *shutdown.borrow() {
            self.supervisor.shutdown().await;
            return;
        }
        let query_permits = Arc::new(tokio::sync::Semaphore::new(16));
        let mut interrupted = false;
        {
            let bootstrap = self.bootstrap();
            tokio::pin!(bootstrap);
            loop {
                tokio::select! {
                    biased;
                    _ = shutdown.changed() => { interrupted = true; break; },
                    _ = &mut bootstrap => break,
                    Some(call) = receive(&mut calls, &mut bridge) => {
                        match if call.reply.active() { decode(call.request) } else { Err(error(ErrorCode::Cancelled,"management caller is no longer active")) } {
                            Ok((id,method,payload)) if is_query(&method) => dispatch_query(call.reply,(id,method,payload),&shared,&data,&install,&query_permits),
                            Ok((id,_,_)) => { let _ = call.reply.send(response(id,Err(error(ErrorCode::Conflict,"host startup is in progress")))); },
                            Err(e) => { let _ = call.reply.send(response("invalid".into(),Err(e))); },
                        }
                    },
                }
            }
        }
        if interrupted {
            self.supervisor.shutdown().await;
            return;
        }
        eprintln!("TwinTail P3 host ready");
        eprintln!("waiting for shutdown signal");
        let mut tick = tokio::time::interval(Duration::from_millis(25));
        loop {
            if *shutdown.borrow() {
                break;
            }
            let call = tokio::select! {
                biased;
                _ = shutdown.changed() => break,
                call = receive(&mut calls, &mut bridge) => match call { Some(call) => call, None => break },
                _ = tick.tick() => { self.supervisor.reconcile().await; self.refresh(); continue; },
            };
            let (request_id, method, payload) = match if call.reply.active() {
                decode(call.request)
            } else {
                Err(error(
                    ErrorCode::Cancelled,
                    "management caller is no longer active",
                ))
            } {
                Ok(v) => v,
                Err(e) => {
                    let _ = call.reply.send(response("invalid".into(), Err(e)));
                    continue;
                }
            };
            if is_query(&method) {
                dispatch_query(
                    call.reply,
                    (request_id, method, payload),
                    &shared,
                    &data,
                    &install,
                    &query_permits,
                );
                continue;
            }
            let diagnostic = method == "adapter.status";
            if !is_mutation(&method) && !diagnostic {
                let _ = call.reply.send(response(
                    request_id,
                    Err(error(
                        ErrorCode::Unsupported,
                        "management method not implemented",
                    )),
                ));
                continue;
            }
            if call.reply.is_plugin() && method == "config.update" {
                let validated =
                    (|| {
                        let id = field(&payload, "id")?;
                        let spec = self
                            .supervisor
                            .spec(id)
                            .ok_or_else(|| error(ErrorCode::NotFound, "unknown plugin"))?;
                        self.store(spec)?
                            .validate_management_input(payload.get("values").ok_or_else(|| {
                                error(ErrorCode::InvalidRequest, "missing values")
                            })?)
                            .map_err(RpcError::from)
                    })();
                if let Err(e) = validated {
                    let _ = call.reply.send(response(request_id, Err(e)));
                    continue;
                }
            }
            let (id, new) = match if diagnostic {
                Ok((String::new(), true))
            } else {
                self.admit(&method, &payload, &call.reply.actor())
            } {
                Ok(v) => v,
                Err(e) => {
                    let _ = call.reply.send(response(request_id, Err(e)));
                    continue;
                }
            };
            let legacy = diagnostic
                || !call.reply.is_plugin()
                    && matches!(
                        method.as_str(),
                        "plugins.start" | "plugins.stop" | "plugins.call"
                    );
            if !call.reply.active() {
                let result = Err(error(
                    ErrorCode::Cancelled,
                    "management caller is no longer active",
                ));
                if new && !diagnostic {
                    let _ = self.finish(&id, &result);
                }
                let _ = call.reply.send(response(request_id, result));
                continue;
            }
            let mut reply = Some(call.reply);
            if !legacy || !new {
                let _ = reply.take().unwrap().send(response(
                    request_id.clone(),
                    Ok(operation_ack(&shared, &id)),
                ));
            }
            if !new {
                continue;
            }
            let result;
            let mut interrupted = false;
            {
                let future = self.execute(&id, &method, payload);
                tokio::pin!(future);
                result = loop {
                    tokio::select! {
                        biased;
                        _ = shutdown.changed() => { interrupted = true; break Err(error(ErrorCode::Cancelled,"host shutdown interrupted operation")); },
                        value = &mut future => break value,
                        Some(call) = receive(&mut calls, &mut bridge) => {
                            match if call.reply.active() { decode(call.request) } else { Err(error(ErrorCode::Cancelled,"management caller is no longer active")) } {
                                Ok((rid,method,payload)) if is_query(&method) => dispatch_query(call.reply,(rid,method,payload),&shared,&data,&install,&query_permits),
                                Ok((rid,method,payload)) => { let result = retry_ack(&shared,&method,&payload,&call.reply.actor()); let _ = call.reply.send(response(rid,result)); },
                                Err(e) => { let _ = call.reply.send(response("invalid".into(),Err(e))); },
                            }
                        },
                    }
                };
            }
            self.refresh();
            if !diagnostic && let Err(e) = self.finish(&id, &result) {
                eprintln!("operation journal update failed: {e}");
            }
            if let Some(reply) = reply {
                let _ = reply.send(response(request_id, result));
            }
            if interrupted {
                break;
            }
        }
        self.supervisor.shutdown().await;
    }
}
fn fingerprint(payload: &Value) -> Result<String> {
    let bytes = serde_json::to_vec(payload).map_err(disk)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
fn retry_ack(shared: &Shared, method: &str, payload: &Value, actor: &str) -> Result<Value> {
    let fingerprint = fingerprint(payload)?;
    let view = shared.lock().unwrap();
    if let Some(key) = payload.get("request_key").and_then(Value::as_str)
        && let Some(op) = view.operations.iter().find(|op| op["request_key"] == key)
    {
        if op["actor"] == actor && op["method"] == method && op["fingerprint"] == fingerprint {
            return Ok(json!({"id":op["id"],"state":op["state"]}));
        }
        return Err(error(
            ErrorCode::Conflict,
            "request_key already belongs to a different request",
        ));
    }
    Err(error(
        ErrorCode::Conflict,
        "another management operation is running; query operations",
    ))
}
fn operation_ack(shared: &Shared, id: &str) -> Value {
    let view = shared.lock().unwrap();
    let state = view
        .operations
        .iter()
        .find(|op| op["id"] == id)
        .and_then(|op| op["state"].as_str())
        .unwrap_or("running");
    json!({"id":id,"state":state})
}
fn open_store(data: &Path, spec: &PluginSpec) -> Result<ConfigStore> {
    ConfigStore::open(
        &data.join("plugins").join(&spec.manifest.id),
        &spec.package_dir,
        spec.manifest.config_schema.as_deref(),
        spec.manifest.config_version,
    )
    .map_err(Into::into)
}
fn is_query(method: &str) -> bool {
    matches!(
        method,
        "plugins.list"
            | "plugins.status"
            | "plugins.installed"
            | "operations.list"
            | "operations.get"
            | "config.get"
            | "config.validate"
            | "config.schema"
            | "management.overview"
            | "diagnostics.logs"
            | "plugins.inspect"
    )
}
fn is_mutation(method: &str) -> bool {
    matches!(
        method,
        "plugins.install"
            | "plugins.enable"
            | "plugins.disable"
            | "plugins.uninstall"
            | "plugins.reload"
            | "plugins.start"
            | "plugins.stop"
            | "plugins.call"
            | "config.update"
    )
}
fn query(
    shared: &Shared,
    data: &Path,
    install: &Path,
    method: &str,
    payload: &Value,
    plugin: Option<&str>,
) -> Result<Value> {
    let view = shared.lock().unwrap().clone();
    let allowed = |capability: &str| {
        plugin.is_none_or(|id| {
            view.specs.iter().any(|spec| {
                spec.manifest.id == id && spec.grants.iter().any(|grant| grant == capability)
            })
        })
    };
    match method {
        "management.overview" => Ok(
            json!({"plugins":safe_statuses(&view.plugins),"operations":{"total":view.operations.len(),"running":view.operations.iter().filter(|operation| operation["state"] == "running").count()},"protocol_version":VERSION,"permissions":{"read":allowed("management.read"),"write":allowed("management.write")},"readonly":!allowed("management.write")}),
        ),
        "diagnostics.logs" => Ok(
            json!({"entries":view.operations.iter().map(|operation| json!({"operation_id":operation["id"],"method":operation["method"],"state":operation["state"],"stage":operation["stage"],"timestamp_unix_ms":operation["created_unix_ms"],"result":operation["result"]})).collect::<Vec<_>>(),"limit":MAX_OPERATIONS}),
        ),
        "plugins.inspect" => {
            let id = field(payload, "id")?;
            let version = field(payload, "version")?;
            let installed = PackageStore::new(install)?
                .inventory()?
                .into_iter()
                .find(|package| package.id == id && package.version == version)
                .ok_or_else(|| error(ErrorCode::NotFound, "unknown installed package"))?;
            let path = installed.path.join("plugin.toml");
            let mut bytes = Vec::new();
            fs::File::open(path)
                .map_err(disk)?
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(disk)?;
            if bytes.len() > 1024 * 1024 {
                return Err(error(
                    ErrorCode::InvalidConfig,
                    "manifest exceeds query limit",
                ));
            }
            let manifest: twintail_core::Manifest =
                toml::from_str(std::str::from_utf8(&bytes).map_err(disk)?).map_err(disk)?;
            twintail_core::validate_manifest(&manifest)?;
            Ok(json!({"manifest":manifest,"verified":false}))
        }
        "operations.list" => Ok(json!(view.operations)),
        "operations.get" => {
            let id = field(payload, "id")?;
            view.operations
                .iter()
                .find(|op| op["id"] == id)
                .cloned()
                .ok_or_else(|| error(ErrorCode::NotFound, "unknown operation"))
        }
        "plugins.list" => Ok(safe_statuses(&view.plugins)),
        "plugins.status" => {
            let id = field(payload, "id")?;
            view.plugins
                .as_array()
                .and_then(|plugins| plugins.iter().find(|p| p["id"] == id))
                .map(safe_status)
                .ok_or_else(|| error(ErrorCode::NotFound, "unknown plugin"))
        }
        "plugins.installed" => {
            serde_json::to_value(PackageStore::new(install)?.inventory()?).map_err(disk)
        }
        "config.get" | "config.validate" | "config.schema" => {
            let id = field(payload, "id")?;
            let spec = view
                .specs
                .iter()
                .find(|spec| spec.manifest.id == id)
                .ok_or_else(|| error(ErrorCode::NotFound, "unknown plugin"))?;
            let store = open_store(data, spec)?;
            let snapshot = || {
                if plugin.is_some() {
                    store.read_for_management()
                } else {
                    store.read()
                }
            };
            if method == "config.schema" {
                Ok(
                    json!({"schema":store.schema(),"snapshot":snapshot().map_err(RpcError::from)?,"status":store.status().map_err(RpcError::from)?}),
                )
            } else if method == "config.get" {
                Ok(
                    json!({"snapshot":snapshot().map_err(RpcError::from)?,"status":store.status().map_err(RpcError::from)?}),
                )
            } else {
                let values = payload
                    .get("values")
                    .ok_or_else(|| error(ErrorCode::InvalidRequest, "missing values"))?;
                if plugin.is_some() {
                    store
                        .validate_management_input(values)
                        .map_err(RpcError::from)?;
                } else {
                    store.validate(values).map_err(RpcError::from)?;
                }
                Ok(json!({"valid":true}))
            }
        }
        _ => Err(error(ErrorCode::Unsupported, "unknown query")),
    }
}

fn safe_status(status: &Value) -> Value {
    let mut status = status.clone();
    if let Some(error) = status.get_mut("error").filter(|error| error.is_object()) {
        *error = json!({"code":error["code"],"message":"plugin lifecycle failed"});
    }
    if let Some(reload) = status.get_mut("reload").filter(|reload| reload.is_object()) {
        for field in ["error", "rollback_error"] {
            if let Some(error) = reload.get_mut(field).filter(|error| !error.is_null()) {
                *error = json!({"code":error["code"],"message":"plugin reload failed"});
            }
        }
    }
    status
}
fn safe_statuses(statuses: &Value) -> Value {
    Value::Array(
        statuses
            .as_array()
            .into_iter()
            .flatten()
            .map(safe_status)
            .collect(),
    )
}
