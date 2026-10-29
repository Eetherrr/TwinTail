//! Safe asynchronous process-plugin runtime for TwinTail wire 1.2.
//!
//! Implement [`Plugin`] and call [`run_stdio`]. stdout is exclusively wire data;
//! write diagnostics to stderr. Handlers may await nested host calls: the reader
//! continues resolving responses independently. All runtime queues, pending calls
//! and concurrent handlers are bounded. Calls are never retried automatically.

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{mpsc, oneshot, watch},
    time::{Instant, timeout_at},
};
pub use twintail_protocol::{
    ActionCall, BotRef, ConfigCommit, ConfigPrepare, ConfigSnapshot, DeliveryReport, ErrorCode,
    EventEnvelope, Hello, Initialize, ManagementCall, PublishEvent, RegisterBot, RpcError,
    ServiceCall, ServiceRegistration, Subscribe,
};
use twintail_protocol::{Message, Outcome};
mod configuration;
mod runtime;
pub use runtime::{run_io, run_stdio};

/// Result returned by lifecycle and event callbacks.
pub type Result<T = ()> = std::result::Result<T, RpcError>;

/// Runtime resource limits. Zero queue/concurrency/identity limits are rejected.
#[derive(Clone, Debug)]
pub struct RuntimeOptions {
    /// Capacity of each independent reader, request, response, control and event queue.
    pub queue_capacity: usize,
    /// Maximum callbacks executing concurrently, including activation.
    pub max_handlers: usize,
    /// Maximum outstanding plugin-to-host calls.
    pub max_pending: usize,
    /// Maximum distinct inbound request IDs retained before retiring the connection.
    pub max_request_ids: usize,
    /// Deadline for each outgoing host call, including queue admission.
    pub request_timeout: Duration,
    /// Total handshake deadline.
    pub handshake_timeout: Duration,
    /// Deadline for the stop callback; handler futures are cancelled before stop.
    pub shutdown_timeout: Duration,
    /// Maximum time for lifecycle.drain to wait for already running callbacks.
    pub drain_timeout: Duration,
}
impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            queue_capacity: 64,
            max_handlers: 16,
            max_pending: 128,
            max_request_ids: 65536,
            request_timeout: Duration::from_secs(10),
            handshake_timeout: Duration::from_secs(10),
            shutdown_timeout: Duration::from_secs(3),
            drain_timeout: Duration::from_secs(30),
        }
    }
}

/// A plugin's callbacks, driven by the runtime after host activation.
///
/// Callbacks run concurrently up to [`RuntimeOptions::max_handlers`]. Futures
/// must yield cooperatively; blocking work belongs on a bounded blocking worker.
/// On shutdown callback futures are dropped and joined before [`Plugin::stop`].
/// Put connection-owned resources in the plugin so dropping futures releases
/// them. Detached `tokio::spawn` tasks are not tracked: use [`Context::cancelled`]
/// and explicitly join any tasks your plugin owns in `stop`.
pub trait Plugin: Send + Sync + 'static {
    /// Activated: register bots/services or subscribe here. Failure retires this runtime.
    /// Business callbacks begin after this returns; avoid cyclic initialization
    /// calls that require this plugin to serve an incoming request first.
    fn start(&self, _context: Context) -> impl Future<Output = Result> + Send {
        async { Ok(()) }
    }
    /// Handle an event; completion is best-effort and errors terminate the runtime.
    fn event(
        &self,
        _context: Context,
        _event: EventEnvelope,
    ) -> impl Future<Output = Result> + Send {
        async { Ok(()) }
    }
    /// Handle `actions.handle`; successful JSON preserves protocol-specific results.
    fn action(
        &self,
        _context: Context,
        _call: ActionCall,
    ) -> impl Future<Output = Result<Value>> + Send {
        async { Err(unsupported()) }
    }
    /// Handle `services.handle` for a service registered by this instance.
    fn service(
        &self,
        _context: Context,
        _call: ServiceCall,
    ) -> impl Future<Output = Result<Value>> + Send {
        async { Err(unsupported()) }
    }
    /// Handle a custom host invocation such as a mock-only testing endpoint.
    fn request(
        &self,
        _context: Context,
        _method: String,
        _payload: Value,
    ) -> impl Future<Output = Result<Value>> + Send {
        async { Err(unsupported()) }
    }
    /// Validate a candidate configuration without side effects or applied-state changes.
    ///
    /// The host makes the candidate durable only after validation succeeds. Do not
    /// log secrets, full snapshots or resolved secret values. Configuration callbacks
    /// are serialized with one another; business callbacks may run concurrently, so
    /// synchronize plugin-owned state explicitly. Unsupported by default.
    fn prepare_config(
        &self,
        _context: Context,
        _snapshot: ConfigSnapshot,
    ) -> impl Future<Output = Result> + Send {
        async { Err(unsupported()) }
    }
    /// Apply a previously prepared snapshot after the host made it durable.
    ///
    /// Context::config remains the old snapshot during this callback and changes
    /// only after success. Failure retains the old SDK snapshot, clears preparation
    /// and leaves desired/applied reconciliation to the host; plugin side effects
    /// are not automatically rolled back. Never put secrets in logs or errors.
    fn commit_config(
        &self,
        _context: Context,
        _snapshot: ConfigSnapshot,
    ) -> impl Future<Output = Result> + Send {
        async { Err(unsupported()) }
    }
    /// Release plugin-owned resources after cancellation. Bounded by shutdown_timeout.
    fn stop(&self, _context: Context) -> impl Future<Output = Result> + Send {
        async { Ok(()) }
    }
}
fn unsupported() -> RpcError {
    RpcError::new(ErrorCode::Unsupported, "plugin method unsupported")
}
pub(crate) fn error(code: ErrorCode, message: impl Into<String>) -> RpcError {
    RpcError::new(code, message)
}
pub(crate) fn io_error(e: std::io::Error) -> RpcError {
    error(
        if e.kind() == std::io::ErrorKind::InvalidData {
            ErrorCode::InvalidFrame
        } else {
            ErrorCode::PluginExited
        },
        e.to_string(),
    )
}
pub(crate) fn decode<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|e| error(ErrorCode::InvalidRequest, e.to_string()))
}

type Pending = HashMap<String, oneshot::Sender<Result<Value>>>;
pub(crate) struct Inner {
    pub init: Initialize,
    pub config: Mutex<Option<ConfigSnapshot>>,
    pub draining: AtomicBool,
    pub requests: mpsc::Sender<Message>,
    pub pending: Mutex<Pending>,
    pub next: AtomicU64,
    pub cancel: watch::Sender<bool>,
    pub options: RuntimeOptions,
}
/// Cloneable typed host client and immutable instance context.
///
/// Every call uses a unique ID and one deadline covering queueing and response.
/// Dropping a call removes its pending entry. Timeout of an action returns
/// `UnknownOutcome`: its remote side effect may already have occurred.
#[derive(Clone)]
pub struct Context(pub(crate) Arc<Inner>);
impl Context {
    /// Host-assigned identity, grants, data directory and configuration revision.
    pub fn initialize(&self) -> &Initialize {
        &self.0.init
    }
    /// Clone the currently applied immutable configuration, if one was initialized.
    ///
    /// Successful config.commit updates future reads; existing clones remain valid.
    /// initialize().config retains the startup snapshot. Snapshots may be sensitive:
    /// never log values or expose them through diagnostic handlers.
    pub fn config(&self) -> Option<ConfigSnapshot> {
        self.0.config.lock().unwrap().clone()
    }
    /// Whether shutdown or transport retirement has begun.
    pub fn is_cancelled(&self) -> bool {
        *self.0.cancel.borrow()
    }
    /// Wait until runtime cancellation; safe to call from multiple tasks.
    pub async fn cancelled(&self) {
        let mut rx = self.0.cancel.subscribe();
        if !*rx.borrow_and_update() {
            let _ = rx.changed().await;
        }
    }
    /// Register a bot under this adapter's authenticated instance.
    pub async fn register_bot(&self, bot: RegisterBot) -> Result<BotRef> {
        self.typed("bots.register", bot).await
    }
    /// Replace the complete subscription set; an empty list unsubscribes.
    pub async fn subscribe(&self, subscription: Subscribe) -> Result<Value> {
        self.typed("events.subscribe", subscription).await
    }
    /// Publish one event, returning best-effort queue admission counts.
    pub async fn publish(&self, event: PublishEvent) -> Result<DeliveryReport> {
        self.typed("events.publish", event).await
    }
    /// Call the exact adapter generation in BotRef; never automatically retried.
    pub async fn call_action(&self, action: ActionCall) -> Result<Value> {
        self.typed("actions.call", action).await
    }
    /// Register a versioned service under this provider instance.
    pub async fn register_service(&self, registration: ServiceRegistration) -> Result<Value> {
        self.typed("services.register", registration).await
    }
    /// Resolve and call a named provider service through the host.
    pub async fn call_service(&self, call: ServiceCall) -> Result<Value> {
        self.typed("services.call", call).await
    }
    /// Calls an allowlisted host management method under an exact read/write grant.
    /// Mutations return operation ACKs; poll operations for completion. Cancellation
    /// drops this waiter and never retries the operation. Do not assume cancellation
    /// reverses a mutation already admitted by the host.
    pub async fn management(&self, call: ManagementCall) -> Result<Value> {
        self.typed("management.call", call).await
    }
    /// Query host health; requires the `host.health` capability.
    pub async fn health(&self) -> Result<Value> {
        self.typed("host.health", json!({})).await
    }
    async fn typed<T: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        payload: T,
    ) -> Result<R> {
        let payload = serde_json::to_value(payload)
            .map_err(|e| error(ErrorCode::InvalidRequest, e.to_string()))?;
        decode(self.call(method, payload).await?)
    }
    async fn call(&self, method: &str, payload: Value) -> Result<Value> {
        if self.is_cancelled() {
            return Err(error(ErrorCode::Cancelled, "runtime stopped"));
        }
        let sequence = self
            .0
            .next
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| error(ErrorCode::Overloaded, "request ID exhausted"))?;
        let id = format!("sdk-{sequence}");
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.0.pending.lock().unwrap();
            if pending.len() >= self.0.options.max_pending {
                return Err(error(ErrorCode::Overloaded, "pending call limit"));
            }
            pending.insert(id.clone(), tx);
        }
        let _guard = PendingGuard {
            context: self.clone(),
            id: id.clone(),
        };
        let deadline = Instant::now() + self.0.options.request_timeout;
        let admitted = AtomicBool::new(false);
        let local_failure = |code, message: &str| {
            error(
                if method == "actions.call" && admitted.load(Ordering::Relaxed) {
                    ErrorCode::UnknownOutcome
                } else {
                    code
                },
                message,
            )
        };
        let operation = async {
            self.0
                .requests
                .send(Message::request(id, method, payload))
                .await
                .map_err(|_| error(ErrorCode::PluginExited, "writer closed"))?;
            admitted.store(true, Ordering::Relaxed);
            rx.await
                .map_err(|_| local_failure(ErrorCode::PluginExited, "connection retired"))?
        };
        tokio::select! {
            biased;
            _ = self.cancelled() => Err(local_failure(ErrorCode::Cancelled, "runtime stopped")),
            result = timeout_at(deadline, operation) => result.unwrap_or_else(|_| Err(local_failure(ErrorCode::Timeout, "host call deadline elapsed"))),
        }
    }
}
struct PendingGuard {
    context: Context,
    id: String,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.context.0.pending.lock().unwrap().remove(&self.id);
    }
}

/// Generic reader bound accepted by [`run_io`].
pub trait PluginReader: AsyncRead + Unpin + Send + 'static {}
impl<T: AsyncRead + Unpin + Send + 'static> PluginReader for T {}
/// Generic writer bound accepted by [`run_io`].
pub trait PluginWriter: AsyncWrite + Unpin + Send + 'static {}
impl<T: AsyncWrite + Unpin + Send + 'static> PluginWriter for T {}

pub(crate) fn outcome(result: Result<Value>) -> Outcome {
    match result {
        Ok(v) => Outcome::Ok(v),
        Err(e) => Outcome::Err(e),
    }
}
