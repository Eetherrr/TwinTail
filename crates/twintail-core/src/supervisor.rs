//! Bounded process supervision. Instances own pipes and never share plugin identity.

use crate::{PluginSpec, authorize};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{mpsc, oneshot},
    task::{JoinHandle, JoinSet},
    time::{Instant, timeout},
};
use twintail_protocol::{
    Body, ConfigSnapshot, ErrorCode, Hello, Initialize, MAX_FRAME_BYTES, Message, Outcome,
    RpcError, VERSION, read_message, write_message,
};

#[path = "routing.rs"]
mod routing;
use routing::{Dispatch, Identity, Registration, Router, SharedRouter};

const QUEUE: usize = 32;
const CONTROL_QUEUE: usize = 8;
const MAX_IDS: usize = 4096;
const IO_TIMEOUT: Duration = Duration::from_secs(2);

/// Lifecycle limits; all durations must be nonzero and at most 30 seconds.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Total hello/ready handshake deadline.
    pub handshake: Duration,
    /// Deadline for plugin calls; a timeout retires the connection.
    pub request: Duration,
    /// Grace period before a child is killed and reaped.
    pub stop: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            handshake: Duration::from_secs(10),
            request: Duration::from_secs(5),
            stop: Duration::from_secs(2),
        }
    }
}

/// Public snapshot; process liveness alone never implies activation.
#[derive(Clone, Debug, Serialize)]
pub struct PluginStatus {
    /// Stable manifest identity.
    pub id: String,
    /// Package version.
    pub version: String,
    /// One of validated, starting, active, draining, stopped, failed.
    pub state: String,
    /// Monotonic generation within this host process.
    pub generation: u64,
    /// Structured reason for the most recent failure.
    pub error: Option<RpcError>,
    /// Latest reload phase or terminal outcome; contains diagnostics, never configuration values.
    pub reload: Option<Value>,
}
struct Slot {
    spec: PluginSpec,
    status: PluginStatus,
    handle: Option<Instance>,
    launching: Option<JoinHandle<Result<Instance, RpcError>>>,
    next_config: Option<ConfigSnapshot>,
    active_config: Option<ConfigSnapshot>,
    reload: Option<ReloadJournal>,
}
#[derive(Clone, Copy)]
enum ReloadPhase {
    Draining,
    Stopping,
    Candidate,
    Restoring,
    Rollback,
}
struct ReloadJournal {
    old_spec: PluginSpec,
    old_config: Option<ConfigSnapshot>,
    candidate: PluginSpec,
    candidate_config: Option<ConfigSnapshot>,
    phase: ReloadPhase,
    drain: Option<oneshot::Receiver<Result<(), RpcError>>>,
    drain_error: Option<RpcError>,
    drain_deadline: Instant,
    failure: Option<RpcError>,
}
struct Instance {
    protocol_minor: u16,
    tx: mpsc::Sender<InstanceCommand>,
    task: JoinHandle<()>,
}
impl Drop for Instance {
    fn drop(&mut self) {
        self.task.abort();
    }
}
enum InstanceCommand {
    Call {
        method: String,
        payload: Value,
        reply: oneshot::Sender<Result<Value, RpcError>>,
        deadline: Instant,
    },
    Drain {
        reply: oneshot::Sender<Result<(), RpcError>>,
        deadline: Instant,
    },
    Stop(oneshot::Sender<()>),
}
struct Notice {
    id: String,
    generation: u64,
    result: Result<(), RpcError>,
}

/// Single-owner lifecycle coordinator with shared, authenticated runtime routing.
///
/// Call [`Self::reconcile`] regularly to update status and cascade dependency failures.
/// Runtime registrations are removed directly when an instance retires, without
/// waiting for reconciliation. Bots, subscriptions and services belong to one
/// `(plugin_id, instance_id, generation)` and activate only after the handshake.
/// Events use separate queues of 32 messages per subscriber; full queues increment
/// the publication's `dropped` count. Each instance permits 32 concurrent forwarded
/// calls. All routing queues and registrations are bounded; no lock is held across
/// an await. Providers remain able to process responses and nested host requests.
///
/// Bot IDs are nonempty strings of at most 128 bytes without control characters or
/// surrounding whitespace. Names are at most 128 ASCII bytes using letters,
/// digits and `._:-/`. Each instance may register at most 128 bots, event types and
/// service names. Service versions must be SemVer (128 bytes), and call version
/// requirements must parse as SemVer ranges (256 bytes). Repeated bot registration
/// is idempotent; subscriptions replace the set, and services replace by name.
/// Event IDs are host-minted, increasing per bot within its instance; payload JSON
/// is preserved. Delivery is best effort, without persistence or replay.
/// Forwarded actions are never retried; after admission, loss of the provider or
/// its deadline yields `UnknownOutcome` when execution cannot be determined.
///
/// Drop kills direct children through `kill_on_drop`; orderly callers must use
/// [`Self::shutdown`] to reap children. P1 does not supervise descendant process trees.
pub struct Supervisor {
    slots: BTreeMap<String, Slot>,
    order: Vec<String>,
    data_root: PathBuf,
    limits: Limits,
    notices_tx: mpsc::Sender<Notice>,
    notices: mpsc::Receiver<Notice>,
    router: SharedRouter,
}
impl Supervisor {
    /// Consumes a validated, topologically sorted loading plan.
    ///
    /// Package roots must remain immutable after validation. No code runs here.
    pub fn new(
        plan: Vec<PluginSpec>,
        data_root: PathBuf,
        limits: Limits,
    ) -> Result<Self, RpcError> {
        if [&limits.handshake, &limits.request, &limits.stop]
            .iter()
            .any(|d| d.is_zero() || **d > Duration::from_secs(30))
        {
            return Err(error(
                ErrorCode::InvalidConfig,
                "limits must be in (0, 30s]",
            ));
        }
        if plan.len() > 128 {
            return Err(error(ErrorCode::Overloaded, "at most 128 plugins"));
        }
        let (notices_tx, notices) = mpsc::channel(256);
        let plan = crate::validate_specs(plan)?;
        let order = plan.iter().map(|p| p.manifest.id.clone()).collect();
        let slots = plan
            .into_iter()
            .map(|spec| {
                let id = spec.manifest.id.clone();
                let status = PluginStatus {
                    id: id.clone(),
                    version: spec.manifest.version.clone(),
                    state: "validated".into(),
                    generation: 0,
                    error: None,
                    reload: None,
                };
                (
                    id,
                    Slot {
                        spec,
                        status,
                        handle: None,
                        launching: None,
                        next_config: None,
                        active_config: None,
                        reload: None,
                    },
                )
            })
            .collect();
        Ok(Self {
            slots,
            order,
            data_root,
            limits,
            notices_tx,
            notices,
            router: Arc::new(Mutex::new(Router::default())),
        })
    }
    /// Installs a bounded host-controller handoff shared by all existing instances.
    /// The host must service this queue independently of instance response handling.
    pub fn set_management_sender(&mut self, sender: mpsc::Sender<crate::ManagementRequest>) {
        self.router.lock().unwrap().management = Some(sender);
    }
    /// Checks that a plugin bridge source still owns an active, accepting generation.
    /// Use before controller admission, including after waiting in another queue.
    pub fn is_active_instance(&self, plugin_id: &str, instance_id: &str, generation: u64) -> bool {
        self.router.lock().unwrap().is_active(&Identity {
            id: plugin_id.into(),
            instance_id: instance_id.into(),
            generation,
        })
    }
    /// Returns the currently selected validated package specification.
    pub fn spec(&self, id: &str) -> Option<&PluginSpec> {
        self.slots.get(id).map(|slot| &slot.spec)
    }
    /// Whether the active instance negotiated configuration transactions (wire minor 2+).
    pub fn supports_config(&self, id: &str) -> bool {
        self.slots.get(id).is_some_and(|s| {
            s.status.state == "active" && s.handle.as_ref().is_some_and(|h| h.protocol_minor >= 2)
        })
    }
    /// Returns the last confirmed active snapshot, including resolved values. Keep it private.
    pub fn active_config(&self, id: &str) -> Option<&ConfigSnapshot> {
        self.slots.get(id).and_then(|s| s.active_config.as_ref())
    }
    /// Records an application-confirmed snapshot after an online commit.
    /// Rejects stale generations and lifecycle transitions with `Conflict`.
    pub fn confirm_config(
        &mut self,
        id: &str,
        generation: u64,
        snapshot: Option<ConfigSnapshot>,
    ) -> Result<(), RpcError> {
        let slot = self
            .slots
            .get_mut(id)
            .ok_or_else(|| error(ErrorCode::NotFound, "unknown plugin"))?;
        if slot.reload.is_some()
            || slot.status.state != "active"
            || slot.status.generation != generation
        {
            return Err(error(
                ErrorCode::Conflict,
                "configuration acknowledgement belongs to inactive generation",
            ));
        }
        slot.active_config = snapshot.clone();
        slot.next_config = snapshot;
        Ok(())
    }
    /// Stages a configuration snapshot for the next launch; active processes are unchanged.
    /// Returns `NotFound` for unknown IDs and `Conflict` during a reload.
    pub fn set_config(&mut self, id: &str, config: Option<ConfigSnapshot>) -> Result<(), RpcError> {
        let slot = self
            .slots
            .get_mut(id)
            .ok_or_else(|| error(ErrorCode::NotFound, "unknown plugin"))?;
        if slot.reload.is_some() || slot.launching.is_some() {
            return Err(error(
                ErrorCode::Conflict,
                "lifecycle operation is in progress",
            ));
        }
        slot.next_config = config;
        Ok(())
    }
    /// Adds a validated package without launching it, checking the entire dependency plan.
    pub fn add(&mut self, spec: PluginSpec) -> Result<Value, RpcError> {
        self.ensure_no_reload()?;
        let id = spec.manifest.id.clone();
        if self.slots.contains_key(&id) {
            return Err(error(ErrorCode::Conflict, "plugin already exists"));
        }
        let mut plan: Vec<_> = self.slots.values().map(|s| s.spec.clone()).collect();
        plan.push(spec.clone());
        let plan = crate::validate_specs(plan)?;
        if plan.len() > 128 {
            return Err(error(ErrorCode::Overloaded, "at most 128 plugins"));
        }
        self.order = plan.iter().map(|s| s.manifest.id.clone()).collect();
        self.slots.insert(
            id.clone(),
            Slot {
                status: PluginStatus {
                    id: id.clone(),
                    version: spec.manifest.version.clone(),
                    state: "validated".into(),
                    generation: 0,
                    error: None,
                    reload: None,
                },
                spec,
                handle: None,
                launching: None,
                next_config: None,
                active_config: None,
                reload: None,
            },
        );
        Ok(json!({"id":id,"state":"validated"}))
    }
    /// Removes a stopped package with no consumers. No files or private data are deleted.
    pub fn remove(&mut self, id: &str) -> Result<PluginSpec, RpcError> {
        self.ensure_no_reload()?;
        let slot = self
            .slots
            .get(id)
            .ok_or_else(|| error(ErrorCode::NotFound, "unknown plugin"))?;
        if slot.reload.is_some()
            || slot.launching.is_some()
            || slot.handle.is_some()
            || !matches!(
                slot.status.state.as_str(),
                "stopped" | "failed" | "validated"
            )
        {
            return Err(error(ErrorCode::Conflict, "stop plugin before removing it"));
        }
        if self
            .slots
            .values()
            .any(|s| s.spec.manifest.dependencies.contains_key(id))
        {
            return Err(error(
                ErrorCode::Conflict,
                "dependants must be removed first",
            ));
        }
        let plan = crate::validate_specs(
            self.slots
                .iter()
                .filter(|(key, _)| key.as_str() != id)
                .map(|(_, s)| s.spec.clone())
                .collect(),
        )?;
        self.order = plan.iter().map(|s| s.manifest.id.clone()).collect();
        Ok(self.slots.remove(id).unwrap().spec)
    }
    /// Exclusively replaces an instance, restoring its old package and active configuration on failure.
    ///
    /// Validates the complete candidate plan before retiring routing. Grants may only shrink.
    /// Active consumers must first be stopped. `parallel_safe` is unsupported. The drain
    /// deadline uses [`Limits::request`]; shutdown then uses [`Limits::stop`].
    /// Cancellation retains the journal in the supervisor; [`Self::reconcile`] resumes it,
    /// and [`Self::shutdown`] cleans all owned children. Results expose `outcome` as
    /// `reloaded`, `rolled_back`, or `failed`; diagnostics never contain configuration values.
    pub async fn reload(&mut self, id: &str, candidate: PluginSpec) -> Result<Value, RpcError> {
        self.reconcile_instances().await;
        self.ensure_no_reload()?;
        let slot = self
            .slots
            .get(id)
            .ok_or_else(|| error(ErrorCode::NotFound, "unknown plugin"))?;
        if slot.reload.is_some() || slot.launching.is_some() {
            return Err(error(
                ErrorCode::Conflict,
                "lifecycle operation in progress",
            ));
        }
        if candidate.manifest.id != id {
            return Err(error(
                ErrorCode::InvalidRequest,
                "candidate identity differs",
            ));
        }
        if candidate.manifest.reload_policy != "exclusive"
            || slot.spec.manifest.reload_policy != "exclusive"
        {
            return Err(error(
                ErrorCode::Unsupported,
                "only exclusive reload is implemented",
            ));
        }
        if candidate
            .grants
            .iter()
            .any(|g| !slot.spec.grants.contains(g))
        {
            return Err(error(
                ErrorCode::PermissionDenied,
                "reload cannot expand grants",
            ));
        }
        if self.slots.values().any(|s| {
            (s.status.state == "active" || s.launching.is_some())
                && s.spec.manifest.dependencies.contains_key(id)
        }) {
            return Err(error(
                ErrorCode::Conflict,
                "active dependants must be stopped first",
            ));
        }
        crate::validate_specs(
            self.slots
                .iter()
                .map(|(key, s)| {
                    if key == id {
                        candidate.clone()
                    } else {
                        s.spec.clone()
                    }
                })
                .collect(),
        )?;
        let deadline = Instant::now() + self.limits.request;
        let slot = self.slots.get_mut(id).unwrap();
        slot.reload = Some(ReloadJournal {
            old_spec: slot.spec.clone(),
            old_config: slot.active_config.clone(),
            candidate,
            candidate_config: slot.next_config.clone(),
            phase: ReloadPhase::Draining,
            drain: None,
            drain_error: None,
            drain_deadline: deadline,
            failure: None,
        });
        slot.status.state = "draining".into();
        slot.status.reload = Some(json!({"phase":"draining"}));
        self.resume_reload(id).await
    }
    fn ensure_no_reload(&self) -> Result<(), RpcError> {
        if self.slots.values().any(|slot| slot.reload.is_some()) {
            return Err(error(ErrorCode::Conflict, "reload is in progress"));
        }
        Ok(())
    }
    fn refresh_order(&mut self) -> Result<(), RpcError> {
        self.order = crate::validate_specs(self.slots.values().map(|s| s.spec.clone()).collect())?
            .into_iter()
            .map(|s| s.manifest.id)
            .collect();
        Ok(())
    }
    async fn resume_reload(&mut self, id: &str) -> Result<Value, RpcError> {
        loop {
            let phase = self.slots[id].reload.as_ref().unwrap().phase;
            match phase {
                ReloadPhase::Draining => {
                    self.router.lock().unwrap().quiesce(id);
                    let slot = self.slots.get_mut(id).unwrap();
                    let journal = slot.reload.as_mut().unwrap();
                    if journal.drain.is_none()
                        && let Some(handle) = &slot.handle
                    {
                        let (reply, rx) = oneshot::channel();
                        match handle.tx.try_send(InstanceCommand::Drain {
                            reply,
                            deadline: journal.drain_deadline,
                        }) {
                            Ok(()) => journal.drain = Some(rx),
                            Err(_) => {
                                journal.drain_error = Some(error(
                                    ErrorCode::Overloaded,
                                    "drain command queue full or closed",
                                ))
                            }
                        }
                    }
                    if let Some(rx) = journal.drain.as_mut() {
                        journal.drain_error =
                            match tokio::time::timeout_at(journal.drain_deadline, rx).await {
                                Ok(Ok(Ok(()))) => None,
                                Ok(Ok(Err(err))) => Some(err),
                                Ok(Err(_)) => {
                                    Some(error(ErrorCode::PluginExited, "drain connection closed"))
                                }
                                Err(_) => {
                                    Some(error(ErrorCode::Timeout, "drain deadline exceeded"))
                                }
                            };
                    } else if slot.handle.is_some() {
                        // Full command queues must still get the bounded window to finish work.
                        tokio::time::sleep_until(journal.drain_deadline).await;
                    }
                    journal.phase = ReloadPhase::Stopping;
                }
                ReloadPhase::Stopping => {
                    self.stop_one(id).await;
                    let slot = self.slots.get_mut(id).unwrap();
                    let journal = slot.reload.as_mut().unwrap();
                    slot.spec = journal.candidate.clone();
                    slot.next_config = journal.candidate_config.clone();
                    slot.status.version = slot.spec.manifest.version.clone();
                    slot.status.reload = Some(json!({"phase":"starting_candidate"}));
                    journal.phase = ReloadPhase::Candidate;
                    self.refresh_order()?;
                }
                ReloadPhase::Restoring => {
                    self.stop_one(id).await;
                    let slot = self.slots.get_mut(id).unwrap();
                    let journal = slot.reload.as_mut().unwrap();
                    slot.spec = journal.old_spec.clone();
                    slot.next_config = journal.old_config.clone();
                    slot.status.version = slot.spec.manifest.version.clone();
                    slot.status.reload = Some(json!({"phase":"starting_rollback"}));
                    journal.phase = ReloadPhase::Rollback;
                    self.refresh_order()?;
                }
                ReloadPhase::Candidate | ReloadPhase::Rollback => {
                    let result = if self.slots[id].status.state == "active" {
                        Ok(json!({}))
                    } else {
                        self.start_one(id).await
                    };
                    let rollback = matches!(phase, ReloadPhase::Rollback);
                    if let Err(err) = result {
                        if !rollback {
                            let journal = self.slots.get_mut(id).unwrap().reload.as_mut().unwrap();
                            journal.failure = Some(err);
                            journal.phase = ReloadPhase::Restoring;
                            continue;
                        }
                        let slot = self.slots.get_mut(id).unwrap();
                        let journal = slot.reload.take().unwrap();
                        let result = json!({"id":id,"outcome":"failed","state":"failed","generation":slot.status.generation,"error":journal.failure,"rollback_error":err,"drain_error":journal.drain_error});
                        slot.status.reload = Some(result.clone());
                        return Ok(result);
                    }
                    let slot = self.slots.get_mut(id).unwrap();
                    let journal = slot.reload.take().unwrap();
                    let result = json!({"id":id,"outcome":if rollback {"rolled_back"} else {"reloaded"},"state":slot.status.state,"generation":slot.status.generation,"error":journal.failure,"drain_error":journal.drain_error});
                    slot.status.reload = Some(result.clone());
                    return Ok(result);
                }
            }
        }
    }
    /// Returns snapshots after the caller has reconciled background notifications.
    pub fn list(&self) -> Vec<PluginStatus> {
        self.order
            .iter()
            .map(|id| self.slots[id].status.clone())
            .collect()
    }
    /// Starts the plan in dependency order; failed plugins do not stop unrelated ones.
    pub async fn start_all(&mut self) {
        for id in self.order.clone() {
            let _ = self.start(&id).await;
        }
    }
    /// Starts a stopped/failed instance with a new generation; strong dependencies must be active.
    /// Cancelling the wait retains the launch task; shutdown joins its bounded handshake and cleanup.
    pub async fn start(&mut self, id: &str) -> Result<Value, RpcError> {
        self.reconcile().await;
        if self.slots.get(id).is_some_and(|s| s.reload.is_some()) {
            return Err(error(ErrorCode::Conflict, "reload is in progress"));
        }
        self.start_one(id).await
    }
    async fn start_one(&mut self, id: &str) -> Result<Value, RpcError> {
        let slot = self
            .slots
            .get(id)
            .ok_or_else(|| error(ErrorCode::NotFound, "unknown plugin"))?;
        if slot.status.state == "active" {
            return Err(error(ErrorCode::Conflict, "plugin already active"));
        }
        for dep in slot.spec.manifest.dependencies.keys() {
            if self
                .slots
                .get(dep)
                .is_none_or(|s| s.status.state != "active")
            {
                let err = error(
                    ErrorCode::DependencyFailed,
                    format!("dependency {dep} is not active"),
                );
                let slot = self.slots.get_mut(id).unwrap();
                slot.status.state = "failed".into();
                slot.status.error = Some(err.clone());
                return Err(err);
            }
        }
        let slot = self.slots.get_mut(id).unwrap();
        if let Some(handle) = slot.handle.as_mut() {
            let _ = (&mut handle.task).await;
        }
        slot.handle = None;
        if slot.launching.is_none() {
            slot.status.generation = slot
                .status
                .generation
                .checked_add(1)
                .ok_or_else(|| error(ErrorCode::Conflict, "generation exhausted"))?;
            let generation = slot.status.generation;
            slot.launching = Some(tokio::spawn(launch(
                slot.spec.clone(),
                slot.next_config.clone(),
                generation,
                self.data_root.clone(),
                self.limits.clone(),
                self.notices_tx.clone(),
                self.router.clone(),
            )));
        }
        let generation = self.slots[id].status.generation;
        let mut tick = tokio::time::interval(Duration::from_millis(25));
        let result = loop {
            let completed = tokio::select! {
                result = self.slots.get_mut(id).unwrap().launching.as_mut().unwrap() => Some(result),
                _ = tick.tick() => None,
            };
            if let Some(result) = completed {
                break result
                    .unwrap_or_else(|_| Err(error(ErrorCode::PluginExited, "launch task exited")));
            }
            self.reconcile_instances().await;
        };
        self.slots.get_mut(id).unwrap().launching = None;
        let slot = self.slots.get_mut(id).unwrap();
        match result {
            Ok(handle) => {
                if handle.task.is_finished() {
                    let err = error(
                        ErrorCode::PluginExited,
                        "plugin exited before activation was observed",
                    );
                    slot.status.state = "failed".into();
                    slot.status.error = Some(err.clone());
                    return Err(err);
                }
                slot.handle = Some(handle);
                slot.status.state = "active".into();
                slot.status.error = None;
                slot.active_config = slot.next_config.clone();
                self.reconcile_instances().await;
                if self.slots[id].status.state != "active" {
                    return Err(error(
                        ErrorCode::DependencyFailed,
                        "dependency failed during startup",
                    ));
                }
                Ok(json!({"id":id,"generation":generation}))
            }
            Err(err) => {
                slot.status.state = "failed".into();
                slot.status.error = Some(err.clone());
                Err(err)
            }
        }
    }
    /// Stops an instance, rejecting the operation when active dependants remain.
    pub async fn stop(&mut self, id: &str) -> Result<Value, RpcError> {
        self.reconcile().await;
        if !self.slots.contains_key(id) {
            return Err(error(ErrorCode::NotFound, "unknown plugin"));
        }
        if self
            .slots
            .values()
            .any(|s| s.status.state == "active" && s.spec.manifest.dependencies.contains_key(id))
        {
            return Err(error(
                ErrorCode::Conflict,
                "active dependants must be stopped first",
            ));
        }
        self.stop_one(id).await;
        Ok(json!({"id":id,"state":"stopped"}))
    }
    async fn stop_pending_launch(&mut self, id: &str) {
        let slot = self.slots.get_mut(id).unwrap();
        if let Some(task) = slot.launching.as_mut()
            && let Ok(Ok(instance)) = task.await
        {
            slot.handle = Some(instance);
        }
        slot.launching = None;
    }
    async fn stop_one(&mut self, id: &str) {
        self.stop_pending_launch(id).await;
        let slot = self.slots.get_mut(id).unwrap();
        if let Some(handle) = slot.handle.as_mut() {
            let (tx, rx) = oneshot::channel();
            // The instance task is always polling commands, even with pending calls.
            if handle.tx.try_send(InstanceCommand::Stop(tx)).is_ok() {
                let _ = timeout(self.limits.stop + IO_TIMEOUT, rx).await;
            }
            if timeout(self.limits.stop + IO_TIMEOUT, &mut handle.task)
                .await
                .is_err()
            {
                handle.task.abort();
                let _ = (&mut handle.task).await;
            }
        }
        slot.handle = None;
        slot.status.state = "stopped".into();
        slot.status.error = None;
    }
    /// Calls a plugin method through its bound pipe; deadline failure retires that generation.
    ///
    /// This host-side API remains independent of plugin routing capabilities.
    /// `actions.handle` calls report `UnknownOutcome` if an admitted invocation
    /// loses its response during retirement. A queue admission failure reports
    /// `Overloaded`; no invocation is automatically retried.
    ///
    /// # Errors
    /// Returns `InvalidRequest` for a malformed method, `NotFound` for an unknown
    /// plugin, `Offline` for an inactive instance, and bounded queue, deadline or
    /// transport errors. Plugin-provided structured errors pass through unchanged.
    ///
    /// # Cancellation
    /// Cancelling the waiter does not retract an already queued plugin request.
    pub async fn call(
        &mut self,
        id: &str,
        method: String,
        payload: Value,
    ) -> Result<Value, RpcError> {
        self.reconcile().await;
        if method.is_empty() || method.len() > 128 {
            return Err(error(ErrorCode::InvalidRequest, "invalid method"));
        }
        let slot = self
            .slots
            .get(id)
            .ok_or_else(|| error(ErrorCode::NotFound, "unknown plugin"))?;
        let handle = slot
            .handle
            .as_ref()
            .filter(|_| slot.status.state == "active")
            .ok_or_else(|| error(ErrorCode::Offline, "plugin is offline"))?;
        let (tx, rx) = oneshot::channel();
        handle
            .tx
            .try_send(InstanceCommand::Call {
                method,
                payload,
                reply: tx,
                deadline: Instant::now() + self.limits.request,
            })
            .map_err(|_| error(ErrorCode::Overloaded, "plugin command queue full or closed"))?;
        match timeout(self.limits.request + IO_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(error(ErrorCode::PluginExited, "plugin task exited")),
            Err(_) => Err(error(ErrorCode::Timeout, "plugin call deadline exceeded")),
        }
    }
    /// Applies instance failures and stops transitive dependants in reverse order.
    pub async fn reconcile(&mut self) {
        self.reconcile_instances().await;
        for id in self.order.clone() {
            if self.slots[&id].reload.is_some() {
                let _ = self.resume_reload(&id).await;
            }
        }
    }
    async fn reconcile_instances(&mut self) {
        while let Ok(notice) = self.notices.try_recv() {
            let Some(slot) = self.slots.get_mut(&notice.id) else {
                continue;
            };
            if slot.status.generation != notice.generation || slot.status.state != "active" {
                continue;
            }
            slot.status.state = if notice.result.is_ok() {
                "stopped"
            } else {
                "failed"
            }
            .into();
            slot.status.error = notice.result.err();
        }
        let mut invalid = BTreeSet::new();
        for id in &self.order {
            let slot = &self.slots[id];
            if slot.status.state != "active"
                || slot
                    .spec
                    .manifest
                    .dependencies
                    .keys()
                    .any(|d| invalid.contains(d))
            {
                invalid.insert(id.clone());
            }
        }
        for id in self.order.clone().into_iter().rev() {
            if invalid.contains(&id) && self.slots[&id].status.state == "active" {
                self.stop_one(&id).await;
                let s = self.slots.get_mut(&id).unwrap();
                s.status.state = "failed".into();
                s.status.error = Some(error(
                    ErrorCode::DependencyFailed,
                    "dependency became unavailable",
                ));
            }
        }
    }
    /// Stops all processes in reverse dependency order, awaiting direct-child cleanup.
    pub async fn shutdown(&mut self) {
        for id in self.order.clone().into_iter().rev() {
            self.stop_one(&id).await;
            self.slots.get_mut(&id).unwrap().reload = None;
        }
    }
}
impl Drop for Supervisor {
    fn drop(&mut self) {
        for s in self.slots.values_mut() {
            if let Some(task) = s.launching.take() {
                task.abort();
            }
            if let Some(h) = s.handle.take() {
                h.task.abort();
            }
        }
    }
}

fn error(code: ErrorCode, message: impl Into<String>) -> RpcError {
    RpcError::new(code, message)
}
fn io_error(e: std::io::Error) -> RpcError {
    let code = if e.kind() == std::io::ErrorKind::UnexpectedEof {
        ErrorCode::PluginExited
    } else {
        ErrorCode::InvalidFrame
    };
    // Do not echo JSON contents or plugin output into public errors.
    error(code, "plugin pipe closed or invalid frame")
}
fn check_version(message: &Message) -> Result<(), RpcError> {
    if message.protocol_version.major != VERSION.major {
        Err(error(
            ErrorCode::IncompatibleVersion,
            "incompatible wire major version",
        ))
    } else {
        Ok(())
    }
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
}
async fn control(stdout: &mut ChildStdout, expected: &str) -> Result<Message, RpcError> {
    let m = read_message(stdout, MAX_FRAME_BYTES)
        .await
        .map_err(io_error)?;
    check_version(&m)?;
    if !matches!(&m.body,Body::Control{method,..} if method==expected) {
        return Err(error(
            ErrorCode::InvalidRequest,
            format!("expected {expected} control"),
        ));
    }
    Ok(m)
}
async fn launch(
    spec: PluginSpec,
    config: Option<ConfigSnapshot>,
    generation: u64,
    data_root: PathBuf,
    limits: Limits,
    notices: mpsc::Sender<Notice>,
    router: SharedRouter,
) -> Result<Instance, RpcError> {
    let id = spec.manifest.id.clone();
    let identity = Identity {
        id: id.clone(),
        instance_id: format!("{}-{id}-{generation}", std::process::id()),
        generation,
    };
    let data = data_root.join("plugins").join(&id).join("data");
    tokio::fs::create_dir_all(&data).await.map_err(|_| {
        error(
            ErrorCode::InvalidConfig,
            "cannot create private data directory",
        )
    })?;
    let mut command = Command::new(&spec.executable);
    command
        .args(&spec.manifest.args)
        .current_dir(&spec.package_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env_clear();
    for key in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let mut child = command
        .spawn()
        .map_err(|_| error(ErrorCode::PluginExited, "cannot execute plugin"))?;
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let mut logs = AbortOnDrop(tokio::spawn(async move {
        let mut buffer = [0; 4096];
        loop {
            match stderr.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    tokio::task::yield_now().await;
                }
            }
        }
    }));
    let handshake_deadline = Instant::now() + limits.handshake;
    let handshake = async {
        let message = control(&mut stdout, "hello").await?;
        let negotiated_minor = message.protocol_version.minor.min(VERSION.minor);
        let Body::Control { payload, .. } = message.body else {
            unreachable!()
        };
        let hello: Hello = serde_json::from_value(payload)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid hello"))?;
        if hello.plugin_id != id || hello.version != spec.manifest.version {
            return Err(error(
                ErrorCode::InvalidRequest,
                "hello identity differs from manifest",
            ));
        }
        if hello
            .required_capabilities
            .iter()
            .any(|c| !spec.manifest.required_capabilities.contains(c))
            || spec
                .manifest
                .required_capabilities
                .iter()
                .any(|c| !hello.required_capabilities.contains(c))
        {
            return Err(error(
                ErrorCode::PermissionDenied,
                "hello required capabilities differ from manifest",
            ));
        }
        for c in &hello.required_capabilities {
            authorize(&spec.grants, c)?;
        }
        write_message(
            &mut stdin,
            &Message::control("negotiate", json!({"major":1,"minor":negotiated_minor})),
            MAX_FRAME_BYTES,
        )
        .await
        .map_err(io_error)?;
        let data_dir = data
            .to_str()
            .ok_or_else(|| error(ErrorCode::InvalidConfig, "plugin data path must be UTF-8"))?
            .to_owned();
        let init = Initialize {
            instance_id: identity.instance_id.clone(),
            generation,
            granted_capabilities: spec.grants.clone(),
            data_dir,
            config_revision: config.as_ref().map_or(0, |snapshot| snapshot.revision),
            config: config.clone(),
        };
        write_message(
            &mut stdin,
            &Message::control("initialize", serde_json::to_value(init).unwrap()),
            MAX_FRAME_BYTES,
        )
        .await
        .map_err(io_error)?;
        control(&mut stdout, "ready").await?;
        write_message(
            &mut stdin,
            &Message::control("activate", json!({})),
            MAX_FRAME_BYTES,
        )
        .await
        .map_err(io_error)?;
        Ok(negotiated_minor)
    };
    let result = timeout(limits.handshake, handshake)
        .await
        .unwrap_or_else(|_| {
            Err(error(
                ErrorCode::Timeout,
                "plugin handshake deadline exceeded",
            ))
        });
    if let Err(e) = &result {
        let _ = child.kill().await;
        let _ = child.wait().await;
        logs.0.abort();
        let _ = (&mut logs.0).await;
        return Err(e.clone());
    }
    let negotiated_minor = result.unwrap();
    let readiness_limits = limits.clone();
    let (tx, rx) = mpsc::channel(QUEUE);
    let (events_tx, events_rx) = mpsc::channel(QUEUE);
    let registration = Router::activate(&router, identity.clone(), tx.clone(), events_tx);
    let task = tokio::spawn(async move {
        run_instance(
            id,
            generation,
            spec.grants,
            child,
            stdin,
            stdout,
            logs,
            rx,
            notices,
            limits,
            router,
            identity,
            registration,
            events_rx,
        )
        .await;
    });
    let mut instance = Instance {
        protocol_minor: negotiated_minor,
        tx,
        task,
    };
    if negotiated_minor >= 2 {
        let (reply, rx) = oneshot::channel();
        let admitted = instance
            .tx
            .try_send(InstanceCommand::Call {
                method: "lifecycle.started".into(),
                payload: json!({}),
                reply,
                deadline: handshake_deadline,
            })
            .is_ok();
        let result = if admitted {
            match tokio::time::timeout_at(handshake_deadline, rx).await {
                Ok(Ok(Ok(value)))
                    if value.get("started").and_then(Value::as_bool) == Some(true)
                        && value.get("config_revision").and_then(Value::as_u64)
                            == Some(config.as_ref().map_or(0, |snapshot| snapshot.revision)) =>
                {
                    Ok(())
                }
                Ok(Ok(Err(err))) => Err(err),
                Ok(Ok(Ok(_))) => Err(error(
                    ErrorCode::InvalidRequest,
                    "invalid startup acknowledgement",
                )),
                _ => Err(error(
                    ErrorCode::Timeout,
                    "application startup acknowledgement failed",
                )),
            }
        } else {
            Err(error(
                ErrorCode::PluginExited,
                "application startup connection closed",
            ))
        };
        if let Err(err) = result {
            let (reply, rx) = oneshot::channel();
            let _ = instance.tx.try_send(InstanceCommand::Stop(reply));
            let _ = timeout(readiness_limits.stop + IO_TIMEOUT, rx).await;
            if timeout(readiness_limits.stop + IO_TIMEOUT, &mut instance.task)
                .await
                .is_err()
            {
                instance.task.abort();
                let _ = (&mut instance.task).await;
            }
            return Err(err);
        }
    }
    Ok(instance)
}

struct AbortOnDrop<T>(JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct DrainState {
    reply: oneshot::Sender<Result<(), RpcError>>,
    deadline: Instant,
    acknowledged: bool,
}

struct Pending {
    action: bool,
    reply: oneshot::Sender<Result<Value, RpcError>>,
    deadline: Instant,
}
#[allow(clippy::too_many_arguments)]
async fn run_instance(
    id: String,
    generation: u64,
    grants: Vec<String>,
    mut child: Child,
    mut stdin: ChildStdin,
    mut stdout: ChildStdout,
    mut logs: AbortOnDrop<()>,
    mut commands: mpsc::Receiver<InstanceCommand>,
    notices: mpsc::Sender<Notice>,
    limits: Limits,
    router: SharedRouter,
    identity: Identity,
    registration: Registration,
    mut events: mpsc::Receiver<Message>,
) {
    let (frames_tx, mut frames) = mpsc::channel(QUEUE);
    let mut reader = AbortOnDrop(tokio::spawn(async move {
        loop {
            let frame = read_message(&mut stdout, MAX_FRAME_BYTES).await;
            let failed = frame.is_err();
            if frames_tx.send(frame).await.is_err() || failed {
                break;
            }
        }
    }));
    let (ctrl_tx, mut ctrl) = mpsc::channel::<Message>(CONTROL_QUEUE);
    let (responses_tx, mut responses) = mpsc::channel::<Message>(QUEUE);
    let (write_failure_tx, mut write_failure) = mpsc::channel(1);
    let mut writer = AbortOnDrop(tokio::spawn(async move {
        let mut events_open = true;
        loop {
            let message = tokio::select! {biased; m=ctrl.recv()=>m,m=responses.recv()=>m,m=events.recv(), if events_open => match m { Some(m) => Some(m), None => { events_open = false; continue; } }};
            let Some(message) = message else { break };
            if !matches!(
                timeout(
                    IO_TIMEOUT,
                    write_message(&mut stdin, &message, MAX_FRAME_BYTES)
                )
                .await,
                Ok(Ok(()))
            ) {
                let _ = write_failure_tx.send(()).await;
                break;
            }
        }
    }));
    let mut forwarding = JoinSet::new();
    let mut pending: BTreeMap<String, Pending> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut sequence = 0u64;
    let mut tick = tokio::time::interval(Duration::from_millis(25));
    let mut stop_reply = None;
    let mut drain: Option<DrainState> = None;
    let mut draining = false;
    let mut drain_error = None;
    let drain_key = format!("drain:{generation}");
    let result = loop {
        tokio::select! {
            biased;
            command=commands.recv()=>match command{
                Some(InstanceCommand::Drain{reply,deadline})=>{
                    draining=true;
                    router.lock().unwrap().quiesce(&id);
                    if drain.is_some() { let _=reply.send(Err(error(ErrorCode::Conflict,"already draining")));continue; }
                    let sent=ctrl_tx.try_send(Message::request(&drain_key,"lifecycle.drain",json!({}))).is_ok();
                    if !sent { drain_error=Some(error(ErrorCode::Overloaded,"drain control queue full")); }
                    drain=Some(DrainState{reply,deadline,acknowledged:!sent});
                },
                Some(InstanceCommand::Stop(reply))=>{stop_reply=Some(reply);break Ok(());},
                None=>break Ok(()),
                Some(InstanceCommand::Call{method,payload,reply,deadline})=>{
                    if draining { let _=reply.send(Err(error(ErrorCode::Cancelled,"instance is draining")));continue; }
                    if deadline <= Instant::now() { let _=reply.send(Err(error(ErrorCode::Timeout,"request expired in queue")));continue; }
                    if pending.len()>=QUEUE{let _=reply.send(Err(error(ErrorCode::Overloaded,"in-flight request limit")));continue;}
                    sequence+=1;let key=format!("h:{generation}:{sequence}");
                    let action = method == "actions.handle";
                    let message=Message::request(&key,method,payload);
                    if ctrl_tx.try_send(message).is_err(){let _=reply.send(Err(error(ErrorCode::Overloaded,"control queue full")));continue;}
                    pending.insert(key,Pending{reply,deadline,action});
                }
            },
            frame=frames.recv()=>{
                let message=match frame{Some(Ok(m))=>m,Some(Err(e))=>break Err(io_error(e)),None=>break Err(error(ErrorCode::PluginExited,"plugin output closed"))};
                if let Err(e)=check_version(&message){break Err(e);}
                match message.body{
                    Body::Response{request_id,outcome}=>{
                        if request_id==drain_key && draining {
                            if let Some(state)=drain.as_mut() { state.acknowledged=true; if let Outcome::Err(err)=outcome { drain_error=Some(err); } }
                            continue;
                        }
                        let Some(call)=pending.remove(&request_id)else{break Err(error(ErrorCode::InvalidRequest,"unknown or reused response ID"));};
                        let result=match outcome{Outcome::Ok(v)=>Ok(v),Outcome::Err(e)=>Err(e)};let _=call.reply.send(result);
                    },
                    Body::Request{request_id,method,payload}=>{
                        if !valid_id(&request_id)||!seen.insert(request_id.clone()){break Err(error(ErrorCode::InvalidRequest,"invalid or duplicate plugin request ID"));}
                        if seen.len()>MAX_IDS{break Err(error(ErrorCode::Overloaded,"connection request-ID budget exhausted"));}
                        let result = if method == "host.health" {
                            authorize(&grants,"host.health").map(|_| Dispatch::Ready(json!({"status":"ok","plugin_id":id,"generation":generation})))
                        } else if method=="management.call" {
                            if forwarding.len()>=QUEUE { Err(error(ErrorCode::Overloaded,"forwarding concurrency limit")) }
                            else { router.lock().unwrap().management(&identity,&grants,payload,limits.request) }
                        } else if let Some(capability) = routing::capability(&method) {
                            authorize(&grants, capability).and_then(|()| {
                                if forwarding.len() >= QUEUE && matches!(method.as_str(), "actions.call" | "services.call") {
                                    return Err(error(ErrorCode::Overloaded, "forwarding concurrency limit"));
                                }
                                router.lock().unwrap().dispatch(&identity, &method, payload, limits.request)
                            })
                        } else { Err(error(ErrorCode::Unsupported, "unknown host method")) };
                        match result {
                            Ok(Dispatch::Forward(call)) => {
                                let deadline = limits.request;
                                let management=method=="management.call";
                                let source=identity.clone();
                                let routing=router.clone();
                                forwarding.spawn(async move {
                                    let mut result=call.complete(deadline).await;
                                    if management && !routing.lock().unwrap().is_active(&source) { result=Err(error(ErrorCode::Offline,"management source generation retired")); }
                                    (request_id,result)
                                });
                            }
                            result => {
                                let outcome = match result { Ok(Dispatch::Ready(v)) => Outcome::Ok(v), Err(e) => Outcome::Err(e), _ => unreachable!() };
                                if responses_tx.try_send(Message::response(request_id,outcome)).is_err(){break Err(error(ErrorCode::Overloaded,"response queue full"));}
                            }
                        }
                    },
                    Body::Control{method,..} if method=="ping"=>{if ctrl_tx.try_send(Message::control("pong",json!({}))).is_err(){break Err(error(ErrorCode::Overloaded,"control queue full"));}},
                    _=>break Err(error(ErrorCode::Unsupported,"unexpected runtime message kind")),
                }
            },
            completed=forwarding.join_next(), if !forwarding.is_empty()=>{
                if let Some(Ok((request_id,result))) = completed {
                    let outcome = match result { Ok(v) => Outcome::Ok(v), Err(e) => Outcome::Err(e) };
                    if responses_tx.try_send(Message::response(request_id,outcome)).is_err(){break Err(error(ErrorCode::Overloaded,"response queue full"));}
                }
            },
            _=write_failure.recv()=>break Err(error(ErrorCode::Timeout,"plugin pipe write failed or timed out")),
            status=child.wait()=>{let _=status;break Err(error(ErrorCode::PluginExited,"plugin process exited"));},
            _=tick.tick()=>{
                if drain.as_ref().is_some_and(|state| state.deadline<=Instant::now() || (state.acknowledged && pending.is_empty() && forwarding.is_empty())) && let Some(state)=drain.take() {
                    let result=if state.deadline<=Instant::now() && (!state.acknowledged || !pending.is_empty() || !forwarding.is_empty()) { Err(error(ErrorCode::Timeout,"drain deadline exceeded")) } else { drain_error.take().map_or(Ok(()),Err) };
                    let _=state.reply.send(result);
                }
                if pending.values().any(|p|p.deadline<=Instant::now()){break Err(error(ErrorCode::Timeout,"plugin response deadline exceeded"));}},
        }
    };
    drop(registration);
    forwarding.abort_all();
    while forwarding.join_next().await.is_some() {}
    let failure = result
        .clone()
        .err()
        .unwrap_or_else(|| error(ErrorCode::Cancelled, "plugin stopped"));
    for (_, p) in pending {
        let failure = if p.action {
            error(
                ErrorCode::UnknownOutcome,
                "action was enqueued; execution outcome is unknown",
            )
        } else {
            failure.clone()
        };
        let _ = p.reply.send(Err(failure));
    }
    let _ = ctrl_tx.try_send(Message::control("shutdown", json!({})));
    // Writer remains alive during grace so shutdown can reach a cooperative plugin.
    if timeout(limits.stop, child.wait()).await.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    reader.0.abort();
    writer.0.abort();
    logs.0.abort();
    let _ = (&mut reader.0).await;
    let _ = (&mut writer.0).await;
    let _ = (&mut logs.0).await;
    let _ = notices.try_send(Notice {
        id,
        generation,
        result,
    });
    if let Some(reply) = stop_reply {
        let _ = reply.send(());
    }
}

#[cfg(test)]
#[path = "supervisor_tests.rs"]
mod tests;
