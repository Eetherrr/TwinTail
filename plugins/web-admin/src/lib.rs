//! Independent loopback web administration plugin. HTTP requests use host IPC only.
//! Authentication and connection state are memory-only and disappear on restart.
mod config;
mod http;
mod server;
mod specification;
pub use config::Config;
pub use http::{Management, router};
use serde_json::Value;
pub use server::Server;
pub use specification::openapi;
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::Mutex;
use twintail_plugin_sdk::{ConfigSnapshot, Context, ErrorCode, Plugin, Result, RpcError};
struct Host(Context);
impl Management for Host {
    fn call(
        &self,
        method: String,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + '_>> {
        Box::pin(async move {
            self.0
                .management(twintail_plugin_sdk::ManagementCall { method, payload })
                .await
        })
    }
}
/// SDK lifecycle owner; listener binding is part of startup readiness.
#[derive(Default)]
pub struct WebAdmin {
    server: Mutex<Option<Server>>,
}
impl Plugin for WebAdmin {
    async fn start(&self, context: Context) -> Result {
        let snapshot = context.config().ok_or_else(|| {
            RpcError::new(
                ErrorCode::InvalidConfig,
                "web administrator configuration required",
            )
        })?;
        if snapshot.config_version != 1 {
            return Err(RpcError::new(
                ErrorCode::InvalidConfig,
                "unsupported web administrator config version",
            ));
        }
        let config = Config::from_value(snapshot.values)?;
        let server = Server::start(config, Arc::new(Host(context))).await?;
        *self.server.lock().await = Some(server);
        Ok(())
    }
    async fn prepare_config(&self, _context: Context, _snapshot: ConfigSnapshot) -> Result {
        Err(RpcError::new(
            ErrorCode::Unsupported,
            "web administrator configuration requires restart and exclusive reload",
        ))
    }
    async fn stop(&self, _context: Context) -> Result {
        let mut owner = self.server.lock().await;
        if let Some(server) = owner.as_ref() {
            server.stop().await;
        }
        owner.take();
        Ok(())
    }
}
