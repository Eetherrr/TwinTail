//! Shared routing state. Every operation under the lock is synchronous and bounded.
use super::{InstanceCommand, error};
use semver::{Version, VersionReq};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{mpsc, oneshot},
    time::{Instant, timeout_at},
};
use twintail_protocol::{
    ActionCall, Body, BotRef, DeliveryReport, ErrorCode, EventEnvelope, MAX_FRAME_BYTES, Message,
    PublishEvent, RegisterBot, RpcError, ServiceCall, ServiceRegistration, Subscribe, VERSION,
};

const REGISTRATIONS: usize = 128;
pub(super) type SharedRouter = Arc<Mutex<Router>>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    pub id: String,
    pub instance_id: String,
    pub generation: u64,
}
struct Peer {
    identity: Identity,
    accepting: bool,
    management_active: Arc<AtomicBool>,
    commands: mpsc::Sender<InstanceCommand>,
    events: mpsc::Sender<Message>,
    bots: BTreeMap<String, u64>,
    subscriptions: BTreeSet<String>,
    services: BTreeMap<String, Version>,
}
#[derive(Default)]
pub(super) struct Router {
    peers: BTreeMap<String, Peer>,
    pub management: Option<mpsc::Sender<crate::ManagementRequest>>,
}
// Owned by the instance future itself, including before its first poll.
pub(super) struct Registration {
    router: SharedRouter,
    identity: Identity,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.router.lock().unwrap().retire(&self.identity);
    }
}
pub(super) enum Dispatch {
    Ready(Value),
    Forward(Forward),
}
pub(super) struct Forward {
    reply: oneshot::Receiver<Result<Value, RpcError>>,
    action: bool,
    deadline: Instant,
}
impl Forward {
    pub async fn complete(self, deadline: Duration) -> Result<Value, RpcError> {
        let result =
            match timeout_at(self.deadline.min(Instant::now() + deadline), self.reply).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err(error(ErrorCode::PluginExited, "provider retired")),
                Err(_) => Err(error(ErrorCode::Timeout, "provider deadline exceeded")),
            };
        result.map_err(|e| {
            if self.action
                && matches!(
                    e.code,
                    ErrorCode::Timeout
                        | ErrorCode::PluginExited
                        | ErrorCode::Cancelled
                        | ErrorCode::Offline
                )
            {
                error(
                    ErrorCode::UnknownOutcome,
                    "action was enqueued; execution outcome is unknown",
                )
            } else {
                e
            }
        })
    }
}
impl Router {
    pub fn is_active(&self, identity: &Identity) -> bool {
        self.peers
            .get(&identity.id)
            .is_some_and(|p| p.identity == *identity && p.accepting)
    }
    pub fn management(
        &self,
        source: &Identity,
        grants: &[String],
        payload: Value,
        duration: Duration,
    ) -> Result<Dispatch, RpcError> {
        if !self.is_active(source) {
            return Err(error(
                ErrorCode::Offline,
                "management source generation is inactive",
            ));
        }
        let call: twintail_protocol::ManagementCall = decode(payload)?;
        let capability = crate::management::capability(&call.method, &call.payload)?;
        crate::authorize(grants, capability)?;
        let sender = self.management.as_ref().ok_or_else(|| {
            error(
                ErrorCode::Unsupported,
                "host management bridge is unavailable",
            )
        })?;
        let (reply, rx) = oneshot::channel();
        let deadline = Instant::now() + duration;
        sender
            .try_send(crate::ManagementRequest {
                active: self
                    .peers
                    .get(&source.id)
                    .unwrap()
                    .management_active
                    .clone(),
                plugin_id: source.id.clone(),
                instance_id: source.instance_id.clone(),
                generation: source.generation,
                method: call.method,
                payload: call.payload,
                deadline,
                reply,
            })
            .map_err(|_| error(ErrorCode::Overloaded, "host management queue unavailable"))?;
        Ok(Dispatch::Forward(Forward {
            reply: rx,
            action: false,
            deadline,
        }))
    }
    pub fn activate(
        router: &SharedRouter,
        identity: Identity,
        commands: mpsc::Sender<InstanceCommand>,
        events: mpsc::Sender<Message>,
    ) -> Registration {
        let previous = router.lock().unwrap().peers.insert(
            identity.id.clone(),
            Peer {
                identity: identity.clone(),
                accepting: true,
                management_active: Arc::new(AtomicBool::new(true)),
                commands,
                events,
                bots: BTreeMap::new(),
                subscriptions: BTreeSet::new(),
                services: BTreeMap::new(),
            },
        );
        if let Some(peer) = previous {
            peer.management_active.store(false, Ordering::Release);
        }
        Registration {
            router: router.clone(),
            identity,
        }
    }
    pub fn quiesce(&mut self, id: &str) {
        if let Some(peer) = self.peers.get_mut(id) {
            peer.accepting = false;
            peer.management_active.store(false, Ordering::Release);
        }
    }
    pub fn retire(&mut self, identity: &Identity) {
        if self
            .peers
            .get(&identity.id)
            .is_some_and(|p| p.identity == *identity)
            && let Some(peer) = self.peers.remove(&identity.id)
        {
            peer.management_active.store(false, Ordering::Release);
        }
    }
    pub fn dispatch(
        &mut self,
        source: &Identity,
        method: &str,
        payload: Value,
        deadline: Duration,
    ) -> Result<Dispatch, RpcError> {
        if self
            .peers
            .get(&source.id)
            .is_none_or(|p| p.identity != *source)
        {
            return Err(error(ErrorCode::Offline, "source generation is retired"));
        }
        match method {
            "bots.register" => {
                let request: RegisterBot = decode(payload)?;
                text(&request.self_id, 128)?;
                let peer = self.peers.get_mut(&source.id).unwrap();
                if !peer.bots.contains_key(&request.self_id) && peer.bots.len() >= REGISTRATIONS {
                    return Err(error(ErrorCode::Overloaded, "bot registration limit"));
                }
                peer.bots.entry(request.self_id.clone()).or_default();
                Ok(Dispatch::Ready(
                    serde_json::to_value(BotRef {
                        adapter_instance_id: source.instance_id.clone(),
                        self_id: request.self_id,
                    })
                    .unwrap(),
                ))
            }
            "events.subscribe" => {
                let request: Subscribe = decode(payload)?;
                if request.event_types.len() > REGISTRATIONS {
                    return Err(error(ErrorCode::Overloaded, "subscription limit"));
                }
                for name in &request.event_types {
                    name_token(name)?;
                }
                self.peers.get_mut(&source.id).unwrap().subscriptions =
                    request.event_types.into_iter().collect();
                Ok(Dispatch::Ready(json!({})))
            }
            "events.publish" => {
                let request: PublishEvent = decode(payload)?;
                text(&request.self_id, 128)?;
                name_token(&request.protocol)?;
                name_token(&request.event_type)?;
                let sequence = self
                    .peers
                    .get_mut(&source.id)
                    .unwrap()
                    .bots
                    .get_mut(&request.self_id)
                    .ok_or_else(|| error(ErrorCode::NotFound, "source bot is not registered"))?;
                *sequence = sequence
                    .checked_add(1)
                    .ok_or_else(|| error(ErrorCode::Overloaded, "event sequence exhausted"))?;
                let event = EventEnvelope {
                    event_id: format!("{}:{}:{}", source.instance_id, request.self_id, sequence),
                    bot: BotRef {
                        adapter_instance_id: source.instance_id.clone(),
                        self_id: request.self_id,
                    },
                    received_at_ms: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                        .min(u64::MAX as u128) as u64,
                    protocol: request.protocol,
                    event_type: request.event_type,
                    payload: request.payload,
                };
                let mut report = DeliveryReport {
                    delivered: 0,
                    dropped: 0,
                };
                let message = Message {
                    protocol_version: VERSION,
                    body: Body::Event {
                        method: "events.deliver".into(),
                        payload: serde_json::to_value(&event).unwrap(),
                    },
                };
                frame_fits(&message)?;
                for peer in self.peers.values() {
                    if peer.accepting && peer.subscriptions.contains(&event.event_type) {
                        if peer.events.try_send(message.clone()).is_ok() {
                            report.delivered += 1;
                        } else {
                            report.dropped += 1;
                        }
                    }
                }
                Ok(Dispatch::Ready(serde_json::to_value(report).unwrap()))
            }
            "actions.call" => {
                let request: ActionCall = decode(payload)?;
                text(&request.bot.adapter_instance_id, 256)?;
                text(&request.bot.self_id, 128)?;
                name_token(&request.action)?;
                let target = self
                    .peers
                    .values()
                    .find(|p| {
                        p.accepting && p.identity.instance_id == request.bot.adapter_instance_id
                    })
                    .ok_or_else(|| error(ErrorCode::Offline, "adapter is offline"))?;
                if !target.bots.contains_key(&request.bot.self_id) {
                    return Err(error(ErrorCode::NotFound, "bot is not registered"));
                }
                forward(
                    target,
                    "actions.handle",
                    serde_json::to_value(request).unwrap(),
                    true,
                    deadline,
                )
            }
            "services.register" => {
                let request: ServiceRegistration = decode(payload)?;
                name_token(&request.name)?;
                text(&request.version, 128)?;
                let version = Version::parse(&request.version)
                    .map_err(|_| error(ErrorCode::InvalidRequest, "invalid service version"))?;
                let services = &mut self.peers.get_mut(&source.id).unwrap().services;
                if !services.contains_key(&request.name) && services.len() >= REGISTRATIONS {
                    return Err(error(ErrorCode::Overloaded, "service registration limit"));
                }
                services.insert(request.name, version);
                Ok(Dispatch::Ready(json!({})))
            }
            "services.call" => {
                let request: ServiceCall = decode(payload)?;
                name_token(&request.provider)?;
                name_token(&request.service)?;
                name_token(&request.method)?;
                text(&request.version_req, 256)?;
                let requirement = VersionReq::parse(&request.version_req).map_err(|_| {
                    error(
                        ErrorCode::InvalidRequest,
                        "invalid service version requirement",
                    )
                })?;
                let target = self
                    .peers
                    .get(&request.provider)
                    .filter(|p| p.accepting)
                    .ok_or_else(|| error(ErrorCode::Offline, "service provider is offline"))?;
                let version = target
                    .services
                    .get(&request.service)
                    .ok_or_else(|| error(ErrorCode::NotFound, "service is not registered"))?;
                if !requirement.matches(version) {
                    return Err(error(
                        ErrorCode::IncompatibleVersion,
                        "service version does not match",
                    ));
                }
                forward(
                    target,
                    "services.handle",
                    serde_json::to_value(request).unwrap(),
                    false,
                    deadline,
                )
            }
            _ => Err(error(ErrorCode::Unsupported, "unknown host method")),
        }
    }
}
fn forward(
    peer: &Peer,
    method: &str,
    payload: Value,
    action: bool,
    deadline: Duration,
) -> Result<Dispatch, RpcError> {
    frame_fits(&Message::request(
        "h:18446744073709551615:18446744073709551615",
        method,
        payload.clone(),
    ))?;
    let (reply, rx) = oneshot::channel();
    let deadline = Instant::now() + deadline;
    peer.commands
        .try_send(InstanceCommand::Call {
            method: method.into(),
            payload,
            reply,
            deadline,
        })
        .map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => {
                error(ErrorCode::Overloaded, "provider queue full")
            }
            mpsc::error::TrySendError::Closed(_) => error(ErrorCode::Offline, "provider retired"),
        })?;
    Ok(Dispatch::Forward(Forward {
        reply: rx,
        action,
        deadline,
    }))
}
fn frame_fits(message: &Message) -> Result<(), RpcError> {
    if serde_json::to_vec(message)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid routed frame"))?
        .len()
        > MAX_FRAME_BYTES
    {
        Err(error(
            ErrorCode::InvalidRequest,
            "routed frame exceeds size limit",
        ))
    } else {
        Ok(())
    }
}
fn decode<T: DeserializeOwned>(payload: Value) -> Result<T, RpcError> {
    serde_json::from_value(payload)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid method payload"))
}
fn text(value: &str, max: usize) -> Result<(), RpcError> {
    if value.is_empty()
        || value.len() > max
        || value.chars().any(char::is_control)
        || value.trim() != value
    {
        Err(error(
            ErrorCode::InvalidRequest,
            "invalid or overlong identifier",
        ))
    } else {
        Ok(())
    }
}
fn name_token(value: &str) -> Result<(), RpcError> {
    text(value, 128)?;
    if value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"._:-/".contains(&b))
    {
        Ok(())
    } else {
        Err(error(ErrorCode::InvalidRequest, "invalid name"))
    }
}

pub(super) fn capability(method: &str) -> Option<&'static str> {
    match method {
        "bots.register" => Some("bots.register"),
        "events.subscribe" => Some("events.subscribe"),
        "events.publish" => Some("events.publish"),
        "actions.call" => Some("actions.call"),
        "services.register" => Some("services.provide"),
        "services.call" => Some("services.call"),
        _ => None,
    }
}

#[cfg(test)]
#[path = "routing_tests.rs"]
mod tests;
