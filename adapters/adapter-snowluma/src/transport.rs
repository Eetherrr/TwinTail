//! Single-owner Universal WebSocket actor with bounded correlation and no replay.
use crate::{Config, TransportMode};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    task::JoinHandle,
    time::{Instant, timeout, timeout_at},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, accept_hdr_async_with_config, connect_async_with_config,
    tungstenite::{
        client::IntoClientRequest,
        handshake::server::{Request, Response},
        http::StatusCode,
        http::header::{AUTHORIZATION, HeaderValue},
        protocol::{Message, WebSocketConfig},
    },
};
use twintail_onebot11::{ActionRequest, Frame, normalize};
use twintail_plugin_sdk::{BotRef, ErrorCode, PublishEvent, Result, RpcError};

/// Safe connection diagnostics; never includes endpoint, token, payloads or remote errors.
#[derive(Clone, Serialize)]
pub struct Status {
    /// True only while a completed WS handshake owns the socket.
    pub connected: bool,
    /// Stable phase: connecting, listening, connected, backoff, stopped.
    pub phase: &'static str,
    /// Monotonic socket generation, incremented after each successful handshake.
    pub generation: u64,
    /// Total attempted handshakes.
    pub connection_attempts: u64,
    /// Queued plus in-flight action count.
    pub pending: usize,
    /// Events lost to local publication backpressure.
    pub dropped_events: u64,
    /// Malformed/mismatched frames or uncorrelated replies.
    pub rejected_frames: u64,
    /// OneBot heartbeat count; does not satisfy WebSocket pong deadlines.
    pub heartbeats: u64,
    /// Fixed diagnostic category only, never remote text.
    pub last_error: Option<&'static str>,
}
struct Shared {
    status: Mutex<Status>,
    permits: Arc<Semaphore>,
    cancel: watch::Sender<bool>,
    join: tokio::sync::Mutex<Option<JoinHandle<()>>>,
    config: Config,
    bot: BotRef,
}
/// Cloneable action/status handle. Call stop and await it before replacing an instance.
#[derive(Clone)]
pub struct Transport {
    shared: Arc<Shared>,
    commands: mpsc::Sender<Command>,
}
struct Command {
    generation: u64,
    action: String,
    params: Value,
    deadline: Instant,
    reply: oneshot::Sender<Result<Value>>,
    _permit: OwnedSemaphorePermit,
}
struct Pending {
    deadline: Instant,
    reply: oneshot::Sender<Result<Value>>,
    _permit: OwnedSemaphorePermit,
}
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
fn error(code: ErrorCode, message: &str) -> RpcError {
    RpcError::new(code, message)
}
fn unknown() -> RpcError {
    error(
        ErrorCode::UnknownOutcome,
        "action admitted; remote outcome unknown; not retried",
    )
}
impl Transport {
    /// Starts connection management immediately and returns a bounded event receiver.
    /// Offline actions fail before admission. Admitted actions are never replayed.
    pub fn spawn(config: Config, bot: BotRef) -> Result<(Self, mpsc::Receiver<PublishEvent>)> {
        config.validate()?;
        if bot.self_id != config.self_id || bot.adapter_instance_id.is_empty() {
            return Err(error(ErrorCode::InvalidConfig, "invalid bot binding"));
        }
        // Bind before readiness so port conflicts fail startup rather than silently
        // leaving an IPC-ready adapter with no listener. The actor owns the socket.
        let listener = if config.transport == TransportMode::ReverseWs {
            let listener = std::net::TcpListener::bind(&config.reverse_bind)
                .map_err(|_| error(ErrorCode::InvalidConfig, "reverse listener unavailable"))?;
            listener
                .set_nonblocking(true)
                .map_err(|_| error(ErrorCode::InvalidConfig, "reverse listener unavailable"))?;
            Some(
                TcpListener::from_std(listener)
                    .map_err(|_| error(ErrorCode::InvalidConfig, "reverse listener unavailable"))?,
            )
        } else {
            None
        };
        let (commands, rx) = mpsc::channel(config.max_pending);
        let (events, event_rx) = mpsc::channel(config.event_queue);
        let (cancel, _) = watch::channel(false);
        let shared = Arc::new(Shared {
            status: Mutex::new(Status {
                connected: false,
                phase: "connecting",
                generation: 0,
                connection_attempts: 0,
                pending: 0,
                dropped_events: 0,
                rejected_frames: 0,
                heartbeats: 0,
                last_error: None,
            }),
            permits: Arc::new(Semaphore::new(config.max_pending)),
            cancel,
            join: tokio::sync::Mutex::new(None),
            config,
            bot: bot.clone(),
        });
        let transport = Self {
            shared: shared.clone(),
            commands,
        };
        // Store the join handle synchronously before returning; no ownerless task window.
        *shared.join.try_lock().expect("new task owner") = Some(tokio::spawn(actor(
            shared.clone(),
            rx,
            events,
            bot,
            listener,
        )));
        Ok((transport, event_rx))
    }
    pub(crate) fn bot(&self) -> &BotRef {
        &self.shared.bot
    }
    /// Returns only safe counters and fixed connection-state categories.
    pub fn status(&self) -> Status {
        let mut status = self.shared.status.lock().unwrap().clone();
        status.pending = self.shared.config.max_pending - self.shared.permits.available_permits();
        status
    }
    /// Attempts bounded admission and awaits the matching string-echo response.
    ///
    /// Remote nonzero retcodes are returned in full as successful RPC transport
    /// results. Check their status/retcode. Timeout or disconnect after admission
    /// returns UnknownOutcome; non-idempotent actions must not be retried.
    pub async fn action(&self, action: String, params: Value) -> Result<Value> {
        ActionRequest::new(action.clone(), params.clone(), "validation")
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid OneBot action"))?;
        // Bound queued JSON too, before allocating a slot or handing it to the actor.
        if serde_json::to_vec(&params)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid action params"))?
            .len()
            + action.len()
            + 256
            > self.shared.config.max_frame_bytes
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "action exceeds frame bound",
            ));
        }
        let status = self.status();
        if !status.connected || *self.shared.cancel.borrow() {
            return Err(error(ErrorCode::Offline, "OneBot connection offline"));
        }
        let permit = self
            .shared
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| error(ErrorCode::Overloaded, "OneBot pending limit"))?;
        let (reply, rx) = oneshot::channel();
        let deadline = Instant::now() + Duration::from_millis(self.shared.config.action_timeout_ms);
        self.commands
            .try_send(Command {
                generation: status.generation,
                action,
                params,
                deadline,
                reply,
                _permit: permit,
            })
            .map_err(|_| error(ErrorCode::Overloaded, "OneBot command queue unavailable"))?;
        timeout_at(deadline, rx)
            .await
            .map_err(|_| unknown())?
            .map_err(|_| unknown())?
    }
    /// Signals cancellation, closes the socket and joins the connection actor.
    /// Concurrent callers all wait for actor completion through the owner lock.
    pub async fn stop(&self) {
        self.shared.cancel.send_replace(true);
        let mut join = self.shared.join.lock().await;
        if let Some(handle) = join.as_mut() {
            let _ = handle.await;
        }
        join.take();
    }
}
async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    if !*cancel.borrow_and_update() {
        let _ = cancel.changed().await;
    }
}
fn phase(shared: &Shared, name: &'static str, last_error: Option<&'static str>) {
    let mut s = shared.status.lock().unwrap();
    s.connected = name == "connected";
    s.phase = name;
    s.last_error = last_error;
}
// tungstenite fixes the upgrade callback error to an unboxed HTTP response.
#[allow(clippy::result_large_err)]
async fn actor(
    shared: Arc<Shared>,
    mut commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<PublishEvent>,
    bot: BotRef,
    listener: Option<TcpListener>,
) {
    let config = &shared.config;
    let mut cancel = shared.cancel.subscribe();
    let mut backoff = config.reconnect_initial_ms;
    let mut sequence = 0u64;
    loop {
        if *cancel.borrow() {
            break;
        }
        phase(&shared, "connecting", None);
        shared.status.lock().unwrap().connection_attempts += 1;
        let ws_config = WebSocketConfig::default()
            .max_message_size(Some(config.max_frame_bytes))
            .max_frame_size(Some(config.max_frame_bytes))
            .write_buffer_size(0)
            .max_write_buffer_size(config.max_frame_bytes + 1024);
        let connection = if let Some(listener) = &listener {
            phase(&shared, "listening", None);
            let accepted = tokio::select! {biased;
                _=cancelled(&mut cancel)=>break,
                result=listener.accept()=>result,
            };
            match accepted {
                Ok((stream, _)) => tokio::select! {biased;
                    _=cancelled(&mut cancel)=>break,
                    result=timeout(Duration::from_millis(config.connect_timeout_ms),
                        accept_hdr_async_with_config(MaybeTlsStream::Plain(stream),
                            |request: &Request, response: Response| reverse_headers(config, request, response), Some(ws_config)))=>
                        result.map(|result| result.map(|socket| (socket, ()))),
                },
                Err(_) => break,
            }
        } else {
            let request = (|| {
                let mut request = config.url.as_str().into_client_request().ok()?;
                if let Some(token) = &config.access_token {
                    let mut value = HeaderValue::from_str(&format!("Bearer {token}")).ok()?;
                    value.set_sensitive(true);
                    request.headers_mut().insert(AUTHORIZATION, value);
                }
                Some(request)
            })();
            let Some(request) = request else {
                phase(&shared, "stopped", Some("invalid_configuration"));
                break;
            };
            tokio::select! {biased;_=cancelled(&mut cancel)=>break,
            result=timeout(Duration::from_millis(config.connect_timeout_ms),connect_async_with_config(request,Some(ws_config),false))=>
                result.map(|result| result.map(|(socket, _)| (socket, ())))}
        };
        match connection {
            Ok(Ok((mut socket, _))) => {
                {
                    let mut state = shared.status.lock().unwrap();
                    state.generation += 1;
                }
                phase(&shared, "connected", None);
                let began = Instant::now();
                let reason = connected(
                    &shared,
                    &mut socket,
                    &mut commands,
                    &events,
                    &bot,
                    &mut cancel,
                    &mut sequence,
                    listener.as_ref(),
                )
                .await;
                // Bound the close handshake. Dropping the socket also closes TCP/TLS.
                let _ = timeout(Duration::from_millis(100), SinkExt::close(&mut socket)).await;
                phase(&shared, "backoff", Some(reason));
                if began.elapsed() >= Duration::from_millis(config.reconnect_max_ms) {
                    backoff = config.reconnect_initial_ms;
                }
            }
            Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response)))
                if matches!(response.status().as_u16(), 401 | 403) =>
            {
                phase(&shared, "backoff", Some("authentication_failed"))
            }
            _ => phase(&shared, "backoff", Some("connection_failed")),
        }
        // Admission raced disconnect: acknowledge uncertainty, never carry to new socket.
        while let Ok(command) = commands.try_recv() {
            let _ = command.reply.send(Err(unknown()));
        }
        if listener.is_none() {
            tokio::select! {biased;_=cancelled(&mut cancel)=>break,_=tokio::time::sleep(Duration::from_millis(backoff))=>{}}
        }
        backoff = backoff.saturating_mul(2).min(config.reconnect_max_ms);
    }
    phase(&shared, "stopped", None);
    commands.close();
    while let Some(command) = commands.recv().await {
        let _ = command.reply.send(Err(unknown()));
    }
}
#[allow(clippy::too_many_arguments)]
async fn connected(
    shared: &Shared,
    socket: &mut Socket,
    commands: &mut mpsc::Receiver<Command>,
    events: &mpsc::Sender<PublishEvent>,
    bot: &BotRef,
    cancel: &mut watch::Receiver<bool>,
    sequence: &mut u64,
    listener: Option<&TcpListener>,
) -> &'static str {
    let config = &shared.config;
    let generation = shared.status.lock().unwrap().generation;
    let mut pending: HashMap<String, Pending> = HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_millis(5));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut ping_at = Instant::now() + Duration::from_millis(config.ping_interval_ms);
    let mut pong: Option<(Vec<u8>, Instant)> = None;
    let reason = loop {
        tokio::select! {
            _=cancelled(cancel)=>break "stopped",
            accepted=accept_duplicate(listener)=>{
                if let Ok((stream, _))=accepted {
                    // Best-effort rejection must not wait on an extra peer or
                    // interfere with the active socket's action/pong deadlines.
                    let _=stream.try_write(b"HTTP/1.1 409 Conflict\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
                }
            },
            _=tick.tick()=>{
                let now=Instant::now();
                for (_,p) in pending.extract_if(|_,p|p.reply.is_closed() || p.deadline<=now) {let _=p.reply.send(Err(unknown()));}
                if pong.as_ref().is_some_and(|(_,deadline)|*deadline<=now){break "pong_timeout";}
                if pong.is_none() && now>=ping_at {
                    let payload=format!("ws-{generation}-{sequence}").into_bytes();
                    if !write(socket,Message::Ping(payload.clone().into()),config,cancel).await {break "write_failed";}
                    pong=Some((payload,Instant::now()+Duration::from_millis(config.pong_timeout_ms)));ping_at=now+Duration::from_millis(config.ping_interval_ms);
                }
            }
            frame=socket.next()=>match frame {
                Some(Ok(Message::Text(text)))=>match Frame::parse(text.as_str()) {
                    Ok(Frame::Response(response))=>{
                        if let Some(echo)=response.echo_str() && let Some(p)=pending.remove(echo) {
                            let result=if Instant::now()>=p.deadline {Err(unknown())}else{serde_json::to_value(response).map_err(|_|error(ErrorCode::InvalidFrame,"invalid OneBot reply"))};let _=p.reply.send(result);
                        }else{shared.status.lock().unwrap().rejected_frames+=1;}
                    }
                    Ok(Frame::Event(event))=>{
                        if let Ok(event)=normalize(&event,bot) {
                            if event.payload.get("meta_event_type").and_then(Value::as_str)==Some("heartbeat"){shared.status.lock().unwrap().heartbeats+=1;}
                            if events.try_send(event).is_err(){shared.status.lock().unwrap().dropped_events+=1;}
                        }else{shared.status.lock().unwrap().rejected_frames+=1;}
                    }
                    Err(_)=>{shared.status.lock().unwrap().rejected_frames+=1;}
                },
                Some(Ok(Message::Ping(bytes)))=>{if !write(socket,Message::Pong(bytes),config,cancel).await{break "write_failed";}},
                Some(Ok(Message::Pong(bytes)))=>{if pong.as_ref().is_some_and(|(expected,_)|expected.as_slice()==bytes.as_ref()){pong=None;}},
                Some(Ok(Message::Close(_)))|None=>break "disconnected",
                Some(Err(_))=>break "read_failed",
                _=>{shared.status.lock().unwrap().rejected_frames+=1;}
            },
            command=commands.recv()=>{
                let Some(command)=command else {break "stopped";};
                if command.reply.is_closed(){continue;}
                if command.generation!=generation {let _=command.reply.send(Err(unknown()));continue;}
                if Instant::now()>=command.deadline{let _=command.reply.send(Err(unknown()));continue;}
                let Some(next)=sequence.checked_add(1)else{let _=command.reply.send(Err(unknown()));break "echo_exhausted";};*sequence=next;
                let echo=format!("ob-{generation}-{next}");
                let request=ActionRequest::new(command.action,command.params,echo.clone()).expect("admission validated action");
                let text=serde_json::to_string(&request).expect("JSON value serialization");
                pending.insert(echo,Pending{deadline:command.deadline,reply:command.reply,_permit:command._permit});
                // The write itself is bounded and shutdown-preemptible; uncertainty
                // begins at queue admission, including partial writes.
                if !write(socket,Message::Text(text.into()),config,cancel).await{break "write_failed";}
            }
        }
    };
    for (_, p) in pending {
        let _ = p.reply.send(Err(unknown()));
    }
    reason
}
async fn write(
    socket: &mut Socket,
    message: Message,
    config: &Config,
    cancel: &mut watch::Receiver<bool>,
) -> bool {
    tokio::select! {biased;_=cancelled(cancel)=>false,result=timeout(Duration::from_millis(config.connect_timeout_ms),socket.send(message))=>matches!(result,Ok(Ok(())))}
}

impl Drop for Transport {
    fn drop(&mut self) {
        if self.commands.strong_count() == 1 {
            self.shared.cancel.send_replace(true);
        }
    }
}

async fn accept_duplicate(
    listener: Option<&TcpListener>,
) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
    match listener {
        Some(listener) => listener.accept().await,
        None => std::future::pending().await,
    }
}
// tungstenite requires an unboxed HTTP error response.
#[allow(clippy::result_large_err)]
fn reverse_headers(
    config: &Config,
    request: &Request,
    response: Response,
) -> std::result::Result<Response, tokio_tungstenite::tungstenite::handshake::server::ErrorResponse>
{
    let header = |name: &str, expected: &str| {
        let mut values = request.headers().get_all(name).iter();
        values
            .next()
            .is_some_and(|value| value.as_bytes() == expected.as_bytes())
            && values.next().is_none()
    };
    let authenticated = match &config.access_token {
        Some(token) => header("authorization", &format!("Bearer {token}")),
        None => !request.headers().contains_key("authorization"),
    };
    let status = if !authenticated {
        Some(StatusCode::UNAUTHORIZED)
    } else if request.uri().path_and_query().map(|v| v.as_str())
        != Some(config.reverse_path.as_str())
        || !header("x-self-id", &config.self_id)
        || !header("x-client-role", "Universal")
    {
        Some(StatusCode::FORBIDDEN)
    } else {
        None
    };
    if let Some(status) = status {
        Err(tokio_tungstenite::tungstenite::http::Response::builder()
            .status(status)
            .body(None)
            .expect("static response"))
    } else {
        Ok(response)
    }
}
