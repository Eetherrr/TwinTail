//! Opt-in manual receive -> business plugin -> private reply verification.
//!
//! Build with `cargo build -p adapter-snowluma --example real_flow` and separately
//! build the adapter. Set `TWINTAIL_REAL_FLOW=1`, `TWINTAIL_ADAPTER_BIN` (absolute
//! executable path), `URL`, `SELF_ID`, and `TARGET`. Optional `TOKEN_ENV` names an
//! environment variable containing the Bearer token; the token is never printed.
//! Run the example directly. Within `TIMEOUT` seconds (default 180, allowed 1–300) the target account must send a
//! private message consisting of the exact text `TwinTail P4 ping` to the configured
//! account. The feature attempts one `send_private_msg` with `TwinTail P4 pong`.
//! No action is retried, including unknown outcomes. This tool reports protocol
//! response evidence, not independent confirmation of QQ delivery.
//!
//! `--plugin` is an internal child-process mode, not a separate manual command.
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::time::{sleep, timeout};
use twintail_core::{Limits, Manifest, PluginSpec, Supervisor};
use twintail_plugin_sdk::{
    ActionCall, ConfigSnapshot, Context, ErrorCode, EventEnvelope, Hello, Plugin, Result, RpcError,
    Subscribe,
};

const FEATURE: &str = "p4-real-feature";
const ADAPTER: &str = "adapter-snowluma";
const PING: &str = "TwinTail P4 ping";
const PONG: &str = "TwinTail P4 pong";
fn failure(message: &str) -> RpcError {
    RpcError::new(ErrorCode::InvalidRequest, message)
}
fn env(name: &str) -> Result<String> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| failure("required environment setting missing"))
}
fn id(name: &str) -> Result<String> {
    let value = env(name)?;
    if value.len() > 128
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(failure("account setting must be canonical decimal string"));
    }
    Ok(value)
}
fn account(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(id) => Some(id.clone()),
        Value::Number(id) => id.as_u64().map(|v| v.to_string()),
        _ => None,
    }
}
fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Array(segments))
            if segments.iter().all(|segment| {
                segment["type"] == "text" && segment["data"]["text"].is_string()
            }) =>
        {
            segments
                .iter()
                .filter(|segment| segment["type"] == "text")
                .filter_map(|segment| segment["data"]["text"].as_str())
                .collect()
        }
        _ => String::new(),
    }
}
struct Feature {
    binding: Mutex<Option<(String, String)>>,
    matched: AtomicBool,
    report: Mutex<Value>,
}
impl Plugin for Feature {
    async fn start(&self, context: Context) -> Result {
        let snapshot = context
            .config()
            .ok_or_else(|| failure("feature binding missing"))?;
        let self_id = snapshot.values["self_id"]
            .as_str()
            .ok_or_else(|| failure("feature binding invalid"))?
            .to_owned();
        let target = snapshot.values["target"]
            .as_str()
            .ok_or_else(|| failure("feature binding invalid"))?
            .to_owned();
        if self_id == target {
            return Err(failure("target must differ from configured account"));
        }
        *self.binding.lock().unwrap() = Some((self_id, target));
        context
            .subscribe(Subscribe {
                event_types: vec!["message".into()],
            })
            .await?;
        Ok(())
    }
    async fn event(&self, context: Context, event: EventEnvelope) -> Result {
        let (self_id, target) = self
            .binding
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| failure("feature not started"))?;
        if target == self_id
            || event.bot.self_id != self_id
            || event.protocol != "onebot.v11"
            || event.event_type != "message"
            || event.payload["message_type"] != "private"
            || account(event.payload.get("user_id")).as_deref() != Some(target.as_str())
            || text(event.payload.get("message")) != PING
            || self
                .matched
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return Ok(());
        }
        *self.report.lock().unwrap() = json!({"matched":true,"status":"pending","retcode":null});
        let result = context
            .call_action(ActionCall {
                bot: event.bot,
                action: "send_private_msg".into(),
                params: json!({"user_id":target,"message":PONG}),
            })
            .await;
        *self.report.lock().unwrap() = match result {
            Ok(value) => json!({"matched":true,"status":value.get("status").and_then(Value::as_str)
                .filter(|status| ["ok","failed","async"].contains(status)).unwrap_or("invalid_response"),
                "retcode":value.get("retcode").and_then(Value::as_i64)}),
            Err(error) => {
                json!({"matched":true,"status":"rpc_error","retcode":null,"error_code":error.code})
            }
        };
        Ok(())
    }
    async fn request(&self, _context: Context, method: String, _payload: Value) -> Result<Value> {
        if method != "status" {
            return Err(failure("unknown manual feature method"));
        }
        Ok(self.report.lock().unwrap().clone())
    }
}
fn spec(
    id: &str,
    executable: PathBuf,
    capabilities: &[&str],
    args: Vec<String>,
) -> Result<PluginSpec> {
    let mut manifest: Manifest = toml::from_str(&format!("id='{id}'\nname='manual P4 flow'\nversion='0.1.0'\nprotocol_version=1\nhost_api='^0.1'\nentrypoint='manual'\nreload_policy='exclusive'\n"))
        .map_err(|_| failure("manual manifest invalid"))?;
    manifest.capabilities = capabilities.iter().map(|value| (*value).into()).collect();
    manifest.required_capabilities = manifest.capabilities.clone();
    manifest.args = args;
    Ok(PluginSpec {
        package_dir: executable
            .parent()
            .ok_or_else(|| failure("executable parent missing"))?
            .to_owned(),
        executable,
        grants: manifest.capabilities.clone(),
        manifest,
    })
}
async fn supervise() -> Result {
    let self_id = id("SELF_ID")?;
    let target = id("TARGET")?;
    if self_id == target {
        return Err(failure("target must differ from configured account"));
    }
    let binding = json!({"self_id":self_id,"target":target});
    let mut values = json!({"url":env("URL")?,"self_id":self_id});
    if let Ok(name) = std::env::var("TOKEN_ENV") {
        values["access_token"] = json!(env(&name)?);
    }
    // Validate before any process or network connection is started.
    adapter_snowluma::Config::from_value(values.clone())?;
    let executable = PathBuf::from(env("TWINTAIL_ADAPTER_BIN")?)
        .canonicalize()
        .map_err(|_| failure("adapter binary unavailable"))?;
    let own = std::env::current_exe().map_err(|_| failure("manual executable unavailable"))?;
    let data =
        tempfile::tempdir().map_err(|_| failure("private temporary directory unavailable"))?;
    let mut host = Supervisor::new(
        vec![
            spec(
                ADAPTER,
                executable,
                &["bots.register", "events.publish", "services.provide"],
                vec![],
            )?,
            spec(
                FEATURE,
                own,
                &["events.subscribe", "actions.call"],
                vec!["--plugin".into()],
            )?,
        ],
        data.path().to_owned(),
        Limits::default(),
    )?;
    let wait_seconds = std::env::var("TIMEOUT")
        .ok()
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|_| failure("TIMEOUT must be an integer from 1 to 300"))?
        .unwrap_or(180);
    if !(1..=300).contains(&wait_seconds) {
        return Err(failure("TIMEOUT must be from 1 to 300"));
    }
    let outcome = {
        let result = async {
            host.set_config(
                ADAPTER,
                Some(ConfigSnapshot {
                    config_version: 1,
                    revision: 1,
                    values,
                }),
            )?;
            host.set_config(
                FEATURE,
                Some(ConfigSnapshot {
                    config_version: 1,
                    revision: 1,
                    values: binding,
                }),
            )?;
            host.start(FEATURE).await?;
            host.start(ADAPTER).await?;
            eprintln!(
                "Manual flow active; waiting for the authorized trigger. No automatic send or retry."
            );
            timeout(Duration::from_secs(wait_seconds), async {
            let mut previous = Value::Null;
            loop {
                let connection = host.call(ADAPTER, "connection.status".into(), json!({})).await?;
                let diagnostic = json!({"online":connection["connected"],"generation":connection["generation"],"connection_attempts":connection["connection_attempts"]});
                if diagnostic != previous { println!("{diagnostic}"); previous = diagnostic; }
                let report = host.call(FEATURE, "status".into(), json!({})).await?;
                if report["matched"] == true && report["status"] != "pending" {
                    println!("{report}");
                    return if report["status"] == "ok" && report["retcode"] == 0 { Ok(()) }
                        else { Err(failure("manual action was not confirmed successful; no retry")) };
                }
                sleep(Duration::from_millis(100)).await;
            }
        }).await.map_err(|_| failure("manual flow timed out without confirmed completion"))?
        };
        tokio::pin!(result);
        tokio::select! {
            result = &mut result => result,
            signal = tokio::signal::ctrl_c() => {
                Err(failure(if signal.is_ok() { "manual flow interrupted" } else { "shutdown signal failed" }))
            }
        }
    }; // Drop the in-flight operation before shutdown/reap.
    host.shutdown().await;
    outcome
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let result = if std::env::args().nth(1).as_deref() == Some("--plugin") {
        twintail_plugin_sdk::run_stdio(
            Feature {
                binding: Mutex::new(None),
                matched: AtomicBool::new(false),
                report: Mutex::new(json!({"matched":false,"status":"waiting","retcode":null})),
            },
            Hello {
                plugin_id: FEATURE.into(),
                version: env!("CARGO_PKG_VERSION").into(),
                required_capabilities: vec!["events.subscribe".into(), "actions.call".into()],
            },
        )
        .await
    } else {
        if std::env::var("TWINTAIL_REAL_FLOW").as_deref() != Ok("1") {
            eprintln!(
                "Disabled. Set TWINTAIL_REAL_FLOW=1 only after authorizing this manual receive/reply run."
            );
            return;
        }
        supervise().await
    };
    if let Err(error) = result {
        // Fixed diagnostics only: never log endpoint, accounts, messages or token.
        eprintln!("Manual flow ended with code: {:?}", error.code);
        std::process::exit(1);
    }
}
