use super::*;
use std::collections::HashSet;
use tokio::{
    io::AsyncWriteExt,
    task::{JoinHandle, JoinSet},
    time::timeout,
};
use twintail_protocol::{Body, MAX_FRAME_BYTES, VERSION, read_message, write_message};

/// Run a plugin using process stdin/stdout and default resource limits.
///
/// # Errors
/// Returns handshake, transport, callback or protocol errors after cancelling and
/// joining all runtime tasks. stdout must not be used for any other purpose.
pub async fn run_stdio<P: Plugin>(plugin: P, hello: Hello) -> Result {
    run_io(
        plugin,
        hello,
        tokio::io::stdin(),
        tokio::io::stdout(),
        RuntimeOptions::default(),
    )
    .await
}

/// Run a plugin over owned asynchronous streams, useful for duplex contract tests.
///
/// Host must negotiate wire 1.1 or a compatible newer minor before
/// initialize/ready/activate. Unsupported
/// versions, invalid or duplicate IDs, exhausted identity budgets, unknown
/// responses and full inbound queues retire the connection conservatively.
/// Dropping this future aborts its owned tasks and retires pending clients.
/// Callback futures must yield; no async runtime can preempt blocking callbacks.
///
/// Configuration maintenance is serialized independently of business callbacks.
/// `config.prepare` retains at most one candidate; the host must commit it or send
/// `config.abort {revision}` after persistence failure. Abort is idempotent when no
/// candidate exists and refuses to clear a different revision. `config.status`
/// exposes only applied revision/version and waits for successful start.
///
/// `lifecycle.drain` closes business admission immediately, discards queued events,
/// rejects queued/new business requests with Cancelled, and waits for running
/// callbacks up to drain_timeout while the reader continues resolving RPC replies.
/// Success returns `{drained:true}`; timeout leaves admission closed. Maintenance
/// requests and lifecycle controls remain supported. The host should follow drain
/// with shutdown; independently spawned plugin tasks must be cancelled and joined
/// by Plugin::stop, since the SDK tracks callback futures rather than detached work.
///
/// # Errors
/// Returns invalid options, handshake timeout, wire errors, callback errors or
/// shutdown timeout. Host shutdown returns `Ok(())` when cleanup succeeds.
pub async fn run_io<P: Plugin, R: PluginReader, W: PluginWriter>(
    plugin: P,
    hello: Hello,
    mut reader: R,
    mut writer: W,
    options: RuntimeOptions,
) -> Result {
    if options.queue_capacity == 0
        || options.max_handlers == 0
        || options.max_pending == 0
        || options.max_request_ids == 0
    {
        return Err(error(
            ErrorCode::InvalidConfig,
            "runtime limits must be positive",
        ));
    }
    let init = timeout(options.handshake_timeout, async {
        write_message(
            &mut writer,
            &Message::control("hello", serde_json::to_value(&hello).unwrap()),
            MAX_FRAME_BYTES,
        )
        .await
        .map_err(io_error)?;
        writer.flush().await.map_err(io_error)?;
        let negotiated = control(&mut reader, "negotiate").await?;
        if negotiated.get("major").and_then(Value::as_u64) != Some(u64::from(VERSION.major))
            || !negotiated
                .get("minor")
                .and_then(Value::as_u64)
                .is_some_and(|minor| minor >= 1)
        {
            return Err(error(
                ErrorCode::IncompatibleVersion,
                "SDK requires negotiated wire 1.1",
            ));
        }
        let init: Initialize = decode(control(&mut reader, "initialize").await?)?;
        if hello
            .required_capabilities
            .iter()
            .any(|c| !init.granted_capabilities.contains(c))
        {
            return Err(error(
                ErrorCode::PermissionDenied,
                "required capability was not granted",
            ));
        }
        write_message(
            &mut writer,
            &Message::control("ready", json!({})),
            MAX_FRAME_BYTES,
        )
        .await
        .map_err(io_error)?;
        writer.flush().await.map_err(io_error)?;
        control(&mut reader, "activate").await?;
        Ok(init)
    })
    .await
    .map_err(|_| error(ErrorCode::Timeout, "handshake deadline elapsed"))??;
    let capacity = options.queue_capacity;
    let (request_tx, mut request_rx) = mpsc::channel(capacity);
    let (response_tx, mut response_rx) = mpsc::channel::<Message>(capacity);
    let (control_tx, mut control_rx) = mpsc::channel::<Message>(capacity);
    let (incoming_control_tx, mut incoming_control_rx) = mpsc::channel(capacity);
    let (event_tx, mut event_rx) = mpsc::channel(capacity);
    let (incoming_tx, mut incoming_rx) = mpsc::channel(capacity);
    let (maintenance_tx, mut maintenance_rx) = mpsc::channel(capacity);
    let (drain_tx, mut drain_rx) = mpsc::channel(capacity);
    let reader_responses = response_tx.clone();
    let (cancel, _) = watch::channel(false);
    let context = Context(Arc::new(Inner {
        config: Mutex::new(init.config.clone()),
        draining: AtomicBool::new(false),
        init,
        requests: request_tx,
        pending: Mutex::new(HashMap::new()),
        next: AtomicU64::new(1),
        cancel,
        options: options.clone(),
    }));
    let _retire = Retire(context.clone());
    let read_context = context.clone();
    let mut reader_task = OwnedTask(tokio::spawn(async move {
        let mut ids = HashSet::new();
        loop {
            let message = read_message(&mut reader, MAX_FRAME_BYTES)
                .await
                .map_err(io_error)?;
            validate_version(&message)?;
            match message.body {
                Body::Response {
                    request_id,
                    outcome,
                } => {
                    let sender = read_context
                        .0
                        .pending
                        .lock()
                        .unwrap()
                        .remove(&request_id)
                        .ok_or_else(|| {
                            error(ErrorCode::InvalidFrame, "unknown or retired response ID")
                        })?;
                    let _ = sender.send(match outcome {
                        Outcome::Ok(v) => Ok(v),
                        Outcome::Err(e) => Err(e),
                    });
                }
                Body::Request {
                    request_id,
                    method,
                    payload,
                } => {
                    if request_id.is_empty()
                        || request_id.len() > 256
                        || ids.len() >= options.max_request_ids
                        || !ids.insert(request_id.clone())
                    {
                        return Err(error(
                            ErrorCode::InvalidFrame,
                            "invalid, duplicate or exhausted request ID",
                        ));
                    }
                    if method == "lifecycle.drain" {
                        read_context.0.draining.store(true, Ordering::SeqCst);
                        drain_tx
                            .try_send(request_id)
                            .map_err(|_| error(ErrorCode::Overloaded, "drain queue full"))?;
                    } else if matches!(
                        method.as_str(),
                        "config.prepare"
                            | "config.commit"
                            | "config.abort"
                            | "config.status"
                            | "lifecycle.started"
                            | "host.health"
                    ) {
                        maintenance_tx
                            .try_send((request_id, method, payload))
                            .map_err(|_| error(ErrorCode::Overloaded, "maintenance queue full"))?;
                    } else if read_context.0.draining.load(Ordering::SeqCst) {
                        reader_responses
                            .try_send(Message::response(
                                request_id,
                                outcome(Err(error(ErrorCode::Cancelled, "plugin is draining"))),
                            ))
                            .map_err(|_| error(ErrorCode::Overloaded, "response writer full"))?;
                    } else {
                        incoming_tx
                            .try_send((request_id, method, payload))
                            .map_err(|_| error(ErrorCode::Overloaded, "request queue full"))?;
                    }
                }
                Body::Event { method, payload } => {
                    if method != "events.deliver" {
                        return Err(error(ErrorCode::InvalidRequest, "unsupported event"));
                    }
                    let event: EventEnvelope = decode(payload)?;
                    if read_context.0.draining.load(Ordering::SeqCst) {
                        continue;
                    }
                    event_tx
                        .try_send(event)
                        .map_err(|_| error(ErrorCode::Overloaded, "event queue full"))?;
                }
                Body::Control { method, payload } => {
                    incoming_control_tx
                        .try_send((method, payload))
                        .map_err(|_| error(ErrorCode::Overloaded, "control queue full"))?;
                }
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), RpcError>(())
    }));
    let mut writer_task = OwnedTask(tokio::spawn(async move {
        loop {
            let message = tokio::select! {
                biased;
                Some(message) = control_rx.recv() => message,
                Some(message) = response_rx.recv() => message,
                Some(message) = request_rx.recv() => message,
                else => return Ok::<(), RpcError>(()),
            };
            write_message(&mut writer, &message, MAX_FRAME_BYTES)
                .await
                .map_err(io_error)?;
            writer.flush().await.map_err(io_error)?;
        }
    }));
    let plugin = Arc::new(plugin);
    let mut handlers = JoinSet::new();
    let start_plugin = plugin.clone();
    let start_context = context.clone();
    handlers.spawn(async move { start_plugin.start(start_context).await });
    let mut started = false;
    let mut maintenance = JoinSet::new();
    let configuration = Arc::new(tokio::sync::Mutex::new(
        configuration::Configuration::default(),
    ));
    let mut drain: Option<(String, tokio::time::Instant)> = None;
    let result = loop {
        if drain.is_some() && handlers.is_empty() && maintenance.is_empty() {
            let (id, _) = drain.take().unwrap();
            if response_tx
                .try_send(Message::response(id, Outcome::Ok(json!({"drained":true}))))
                .is_err()
            {
                break Err(error(ErrorCode::Overloaded, "response writer full"));
            }
        }
        let drain_deadline = drain.as_ref().map(|(_, deadline)| *deadline);
        tokio::select! {
            biased;
            result = &mut reader_task.0 => break flatten(result),
            result = &mut writer_task.0 => break flatten(result),
            Some((method, payload)) = incoming_control_rx.recv() => {
                match method.as_str() {
                    "shutdown" => break Ok(()),
                    "ping" => { if control_tx.try_send(Message::control("pong",payload)).is_err() { break Err(error(ErrorCode::Overloaded,"control writer full")); } }
                    _ => break Err(error(ErrorCode::InvalidRequest,"unexpected lifecycle control")),
                }
            }
            Some(id) = drain_rx.recv() => {
                if drain.is_some() {
                    if response_tx.try_send(Message::response(id,outcome(Err(error(ErrorCode::Conflict,"drain already pending"))))).is_err() { break Err(error(ErrorCode::Overloaded,"response writer full")); }
                } else { drain = Some((id,tokio::time::Instant::now()+options.drain_timeout)); }
            }
            _ = async { tokio::time::sleep_until(drain_deadline.unwrap()).await }, if drain_deadline.is_some() => {
                let (id,_) = drain.take().unwrap();
                if response_tx.try_send(Message::response(id,outcome(Err(error(ErrorCode::Timeout,"drain deadline elapsed"))))).is_err() { break Err(error(ErrorCode::Overloaded,"response writer full")); }
            }
            Some(result) = maintenance.join_next(), if !maintenance.is_empty() => {
                if let Err(e) = flatten(result) { break Err(e); }
            }
            Some((id,method,payload)) = maintenance_rx.recv(), if started && maintenance.is_empty() && handlers.len() < options.max_handlers => {
                let configuration = configuration.clone(); let plugin = plugin.clone(); let context = context.clone(); let responses = response_tx.clone();
                maintenance.spawn(async move {
                    let result = configuration.lock().await.handle(plugin.as_ref(),context,&method,payload).await;
                    responses.try_send(Message::response(id,outcome(result))).map_err(|_| error(ErrorCode::Overloaded,"response writer full"))
                });
            }
            Some(result) = handlers.join_next(), if !handlers.is_empty() => {
                if let Err(e) = flatten(result) { break Err(e); }
                started = true;
            }
            Some((id, method, payload)) = incoming_rx.recv(), if started && (context.0.draining.load(Ordering::SeqCst) || handlers.len() + maintenance.len() < options.max_handlers) => {
                if context.0.draining.load(Ordering::SeqCst) {
                    if response_tx.try_send(Message::response(id,outcome(Err(error(ErrorCode::Cancelled,"plugin is draining"))))).is_err() { break Err(error(ErrorCode::Overloaded,"response writer full")); }
                    continue;
                }
                let plugin = plugin.clone(); let context = context.clone(); let responses = response_tx.clone();
                handlers.spawn(async move {
                    let result = match method.as_str() {
                        "actions.handle" => match decode(payload) { Ok(call) => plugin.action(context,call).await, Err(e) => Err(e) },
                        "services.handle" => match decode(payload) { Ok(call) => plugin.service(context,call).await, Err(e) => Err(e) },
                        "host.health" => Ok(json!({"status":"ok"})),
                        _ => plugin.request(context,method,payload).await,
                    };
                    responses.try_send(Message::response(id,outcome(result))).map_err(|_| error(ErrorCode::Overloaded,"response writer full"))
                });
            }
            Some(event) = event_rx.recv(), if started && (context.0.draining.load(Ordering::SeqCst) || handlers.len() + maintenance.len() < options.max_handlers) => {
                if context.0.draining.load(Ordering::SeqCst) { continue; }
                let plugin = plugin.clone(); let context = context.clone();
                handlers.spawn(async move { plugin.event(context,event).await });
            }
        }
    };
    context.0.cancel.send_replace(true);
    retire_pending(&context);
    handlers.abort_all();
    maintenance.abort_all();
    while handlers.join_next().await.is_some() {}
    while maintenance.join_next().await.is_some() {}
    let stop = timeout(options.shutdown_timeout, plugin.stop(context))
        .await
        .map_err(|_| error(ErrorCode::Timeout, "stop callback deadline elapsed"))
        .and_then(|r| r);
    reader_task.0.abort();
    writer_task.0.abort();
    // A completed JoinHandle must not be polled again.
    if !reader_task.0.is_finished() {
        let _ = (&mut reader_task.0).await;
    }
    if !writer_task.0.is_finished() {
        let _ = (&mut writer_task.0).await;
    }
    result.and(stop)
}
fn flatten(result: std::result::Result<Result, tokio::task::JoinError>) -> Result {
    result.map_err(|e| error(ErrorCode::PluginExited, e.to_string()))?
}
fn validate_version(message: &Message) -> Result {
    if message.protocol_version.major != VERSION.major || message.protocol_version.minor < 1 {
        Err(error(
            ErrorCode::IncompatibleVersion,
            "unexpected wire version",
        ))
    } else {
        Ok(())
    }
}
async fn control<R: AsyncRead + Unpin>(reader: &mut R, expected: &str) -> Result<Value> {
    let message = read_message(reader, MAX_FRAME_BYTES)
        .await
        .map_err(io_error)?;
    validate_version(&message)?;
    match message.body {
        Body::Control { method, payload } if method == expected => Ok(payload),
        _ => Err(error(
            ErrorCode::InvalidFrame,
            format!("expected {expected} control"),
        )),
    }
}
fn retire_pending(context: &Context) {
    context.0.pending.lock().unwrap().clear();
}
struct Retire(Context);
impl Drop for Retire {
    fn drop(&mut self) {
        self.0.0.cancel.send_replace(true);
        retire_pending(&self.0);
    }
}
struct OwnedTask<T>(JoinHandle<T>);
impl<T> Drop for OwnedTask<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
