//! Echo incoming message segments through their source adapter using OneBot v11 send_msg.
use serde_json::{Map, Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use twintail_plugin_sdk::{
    ActionCall, Context, EventEnvelope, Hello, Plugin, Result, Subscribe, run_stdio,
};
struct Echo {
    ready: AtomicBool,
}
impl Plugin for Echo {
    async fn start(&self, context: Context) -> Result {
        context
            .subscribe(Subscribe {
                event_types: vec!["message".into()],
            })
            .await?;
        self.ready.store(true, Ordering::Release);
        Ok(())
    }
    async fn request(&self, _context: Context, method: String, _payload: Value) -> Result<Value> {
        if method == "echo.ready" {
            Ok(json!({"ready":self.ready.load(Ordering::Acquire)}))
        } else {
            Err(twintail_plugin_sdk::RpcError::new(
                twintail_plugin_sdk::ErrorCode::Unsupported,
                "unknown echo method",
            ))
        }
    }
    async fn event(&self, context: Context, event: EventEnvelope) -> Result {
        let mut params = Map::new();
        for field in ["message", "message_type", "user_id", "group_id"] {
            if let Some(value) = event.payload.get(field) {
                params.insert(field.into(), value.clone());
            }
        }
        if !params.contains_key("message") {
            return Ok(());
        }
        context
            .call_action(ActionCall {
                bot: event.bot,
                action: "send_msg".into(),
                params: Value::Object(params),
            })
            .await?;
        Ok(())
    }
}
#[tokio::main(flavor = "current_thread")]
pub(crate) async fn main() {
    let result = run_stdio(
        Echo {
            ready: AtomicBool::new(false),
        },
        Hello {
            plugin_id: "echo-plugin".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            required_capabilities: vec!["events.subscribe".into(), "actions.call".into()],
        },
    )
    .await;
    if let Err(error) = result {
        eprintln!("echo-plugin: {error}");
        std::process::exit(1);
    }
}
