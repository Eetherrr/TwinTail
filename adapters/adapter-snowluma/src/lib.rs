//! Independent SnowLuma OneBot v11 adapter, forward and reverse Universal WebSocket.
//!
//! The host owns lifecycle/configuration. A registered IPC-ready adapter may still
//! be network-offline: query `connection.status`. All configuration changes require
//! restart and exclusive reload. This adapter never retries admitted actions.
mod config;
mod transport;
pub use config::{Config, TransportMode};
use serde_json::{Value, json};
use tokio::{sync::Mutex, task::JoinHandle};
pub use transport::{Status, Transport};
use twintail_plugin_sdk::{
    ActionCall, ConfigSnapshot, Context, ErrorCode, Plugin, RegisterBot, Result, RpcError,
    ServiceCall, ServiceRegistration,
};

struct Running {
    transport: Transport,
    publisher: JoinHandle<()>,
}
/// SDK plugin owning one account, one connection task and one bounded publisher.
#[derive(Default)]
pub struct SnowLuma {
    running: Mutex<Option<Running>>,
}
impl SnowLuma {
    async fn transport(&self) -> Result<Transport> {
        self.running
            .lock()
            .await
            .as_ref()
            .map(|r| r.transport.clone())
            .ok_or_else(|| RpcError::new(ErrorCode::Offline, "adapter not running"))
    }
}
impl Plugin for SnowLuma {
    async fn start(&self, context: Context) -> Result {
        let snapshot = context.config().ok_or_else(|| {
            RpcError::new(ErrorCode::InvalidConfig, "adapter configuration required")
        })?;
        if snapshot.config_version != 1 {
            return Err(RpcError::new(
                ErrorCode::InvalidConfig,
                "unsupported adapter config version",
            ));
        }
        let config = Config::from_value(snapshot.values)?;
        let bot = context
            .register_bot(RegisterBot {
                self_id: config.self_id.clone(),
            })
            .await?;
        context
            .register_service(ServiceRegistration {
                name: "onebot.v11".into(),
                version: "1.0.0".into(),
            })
            .await?;
        let (transport, mut events) = Transport::spawn(config, bot)?;
        let actor = transport.clone();
        let publisher = tokio::spawn(async move {
            loop {
                tokio::select! {biased;
                    _=context.cancelled()=>break,
                    event=events.recv()=>{let Some(event)=event else{break;};tokio::select!{biased;_=context.cancelled()=>break,_=context.publish(event)=>{}}}
                }
            }
            actor.stop().await;
        });
        *self.running.lock().await = Some(Running {
            transport,
            publisher,
        });
        Ok(())
    }
    async fn action(&self, _context: Context, call: ActionCall) -> Result<Value> {
        let transport = self.transport().await?;
        // The host also validates identity; check account binding at the network boundary.
        if call.bot.self_id != transport.bot().self_id
            || call.bot.adapter_instance_id != transport.bot().adapter_instance_id
        {
            return Err(RpcError::new(
                ErrorCode::InvalidRequest,
                "action account mismatch",
            ));
        }
        transport.action(call.action, call.params).await
    }
    async fn request(&self, _context: Context, method: String, _payload: Value) -> Result<Value> {
        if method != "connection.status" {
            return Err(RpcError::new(
                ErrorCode::Unsupported,
                "adapter request unsupported",
            ));
        }
        Ok(json!(self.transport().await?.status()))
    }
    async fn service(&self, _context: Context, call: ServiceCall) -> Result<Value> {
        if call.service != "onebot.v11" || call.method != "connection.status" {
            return Err(RpcError::new(
                ErrorCode::Unsupported,
                "adapter service method unsupported",
            ));
        }
        Ok(json!(self.transport().await?.status()))
    }
    async fn prepare_config(&self, _context: Context, _snapshot: ConfigSnapshot) -> Result {
        Err(RpcError::new(
            ErrorCode::Unsupported,
            "adapter configuration requires restart and exclusive reload",
        ))
    }
    async fn stop(&self, _context: Context) -> Result {
        let mut owner = self.running.lock().await;
        if let Some(running) = owner.as_mut() {
            running.transport.stop().await;
            running.publisher.abort();
            let _ = (&mut running.publisher).await;
        }
        owner.take();
        Ok(())
    }
}
