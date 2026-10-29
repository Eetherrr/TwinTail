//! Synthetic adapter for local routing demonstrations; never connects to QQ.
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use twintail_plugin_sdk::{
    ActionCall, Context, Hello, Plugin, PublishEvent, RegisterBot, Result, ServiceCall,
    ServiceRegistration, run_stdio,
};
#[derive(Default)]
struct Mock {
    actions: Mutex<Vec<ActionCall>>,
    ready: AtomicBool,
}
impl Plugin for Mock {
    async fn start(&self, context: Context) -> Result {
        context
            .register_bot(RegisterBot {
                self_id: "10001".into(),
            })
            .await?;
        context
            .register_service(ServiceRegistration {
                name: "mock.inspect".into(),
                version: "1.0.0".into(),
            })
            .await?;
        self.ready.store(true, Ordering::Release);
        Ok(())
    }
    async fn request(&self, context: Context, method: String, payload: Value) -> Result<Value> {
        match method.as_str() {
            "mock.ready" => Ok(
                json!({"ready":self.ready.load(Ordering::Acquire),"bot":{"adapter_instance_id":context.initialize().instance_id,"self_id":"10001"}}),
            ),
            "mock.emit" => {
                let report = context
                    .publish(PublishEvent {
                        self_id: "10001".into(),
                        protocol: "mock.v1".into(),
                        event_type: "message".into(),
                        payload,
                    })
                    .await?;
                Ok(serde_json::to_value(report).unwrap())
            }
            "mock.service" => {
                context
                    .call_service(serde_json::from_value(payload).map_err(|e| {
                        twintail_plugin_sdk::RpcError::new(
                            twintail_plugin_sdk::ErrorCode::InvalidRequest,
                            e.to_string(),
                        )
                    })?)
                    .await
            }
            "mock.action" => {
                context
                    .call_action(serde_json::from_value(payload).map_err(|e| {
                        twintail_plugin_sdk::RpcError::new(
                            twintail_plugin_sdk::ErrorCode::InvalidRequest,
                            e.to_string(),
                        )
                    })?)
                    .await
            }
            "mock.health" => context.health().await,
            "mock.actions" => Ok(serde_json::to_value(&*self.actions.lock().await).unwrap()),
            _ => Err(twintail_plugin_sdk::RpcError::new(
                twintail_plugin_sdk::ErrorCode::Unsupported,
                "unknown mock method",
            )),
        }
    }
    async fn action(&self, _context: Context, call: ActionCall) -> Result<Value> {
        let mut actions = self.actions.lock().await;
        if actions.len() >= 1024 {
            return Err(twintail_plugin_sdk::RpcError::new(
                twintail_plugin_sdk::ErrorCode::Overloaded,
                "mock action history full",
            ));
        }
        actions.push(call);
        Ok(json!({"status":"ok", "retcode":0,"data":{"message_id":actions.len()}}))
    }
    async fn service(&self, _context: Context, call: ServiceCall) -> Result<Value> {
        Ok(call.params)
    }
}
#[tokio::main(flavor = "current_thread")]
pub(crate) async fn main() {
    let result = run_stdio(
        Mock::default(),
        Hello {
            plugin_id: "mock-adapter".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            required_capabilities: vec![
                "bots.register".into(),
                "events.publish".into(),
                "services.provide".into(),
            ],
        },
    )
    .await;
    if let Err(error) = result {
        eprintln!("mock-adapter: {error}");
        std::process::exit(1);
    }
}
