//! Internal configuration/reload integration fixture; not a public SDK executable.
use serde_json::{Value, json};
use std::sync::Mutex;
use twintail_plugin_sdk::*;

#[derive(Default)]
struct Configured {
    started: Mutex<Option<ConfigSnapshot>>,
}
impl Plugin for Configured {
    async fn start(&self, context: Context) -> Result {
        *self.started.lock().unwrap() = context.config();
        if context
            .config()
            .as_ref()
            .is_some_and(|s| s.values.get("reject_start") == Some(&json!(true)))
        {
            return Err(RpcError::new(
                ErrorCode::InvalidConfig,
                "fixture startup rejected",
            ));
        }
        Ok(())
    }
    async fn prepare_config(&self, _: Context, snapshot: ConfigSnapshot) -> Result {
        if snapshot.values.get("reject_prepare") == Some(&json!(true)) {
            return Err(RpcError::new(
                ErrorCode::InvalidConfig,
                "fixture prepare rejected",
            ));
        }
        Ok(())
    }
    async fn commit_config(&self, _: Context, snapshot: ConfigSnapshot) -> Result {
        if snapshot.values.get("reject_commit") == Some(&json!(true)) {
            return Err(RpcError::new(
                ErrorCode::InvalidConfig,
                "fixture commit rejected",
            ));
        }
        Ok(())
    }
    async fn request(&self, context: Context, method: String, _: Value) -> Result<Value> {
        if method != "probe" {
            return Err(RpcError::new(
                ErrorCode::Unsupported,
                "unknown fixture request",
            ));
        }
        let snapshot = context.config();
        let startup = self.started.lock().unwrap();
        // Explicitly allowlist nonsecret fixture fields. Never return all values.
        Ok(json!({
            "revision": snapshot.as_ref().map(|s|s.revision),
            "config_version": snapshot.as_ref().map(|s|s.config_version),
            "message": snapshot.as_ref().and_then(|s|s.values.get("message")).and_then(Value::as_str),
            "config_value": snapshot.as_ref().and_then(|s|s.values.get("config_value")).and_then(Value::as_str),
            "initial_revision": startup.as_ref().map(|s|s.revision),
        }))
    }
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let result = run_stdio(
        Configured::default(),
        Hello {
            plugin_id: "p3-config".into(),
            version: "1.0.0".into(),
            required_capabilities: vec![],
        },
    )
    .await;
    if result.is_err() {
        eprintln!("configuration fixture runtime exited");
        std::process::exit(1);
    }
}
