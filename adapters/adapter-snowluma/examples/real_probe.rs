//! Manual real-server probe. This example is compiled by all-target checks but
//! never executes as part of automated tests. It does not print endpoints,
//! credentials, account IDs, event payloads or action response data.
//!
//! Required environment: `TWINTAIL_ONEBOT_URL` and `TWINTAIL_ONEBOT_SELF_ID`.
//! Optional `TWINTAIL_ONEBOT_TOKEN_ENV` names another environment variable holding
//! the Bearer token; an unset name means no Authorization header. Tokens are not
//! accepted on the command line or embedded in the URL.
//!
//! `TWINTAIL_SEND_PROBE=1` explicitly enables exactly one `send_private_msg` call
//! to `TWINTAIL_ONEBOT_TARGET`, with fixed text `TwinTail P4 连通性测试`. Any other
//! value disables sending. No action is retried, including uncertain outcomes.
//!
//! Bounds: `TWINTAIL_PROBE_CONNECT_TIMEOUT_SECS` defaults to 10 (1..=60),
//! `TWINTAIL_PROBE_EVENT_TIMEOUT_SECS` to 60 (0..=300), and
//! `TWINTAIL_PROBE_ACTION_TIMEOUT_MS` to 3000 (10..=30000).
//! The example explicitly stops and joins the transport on every normal/error
//! exit after spawn. Ctrl-C while counting events also triggers orderly stop.
//!
//! Run manually with `cargo run -p adapter-snowluma --example real_probe` after
//! setting the environment. See the adapter author guide for operational scope.

use adapter_snowluma::{Config, Transport};
use serde_json::{Value, json};
use std::{env, process::ExitCode, time::Duration};
use tokio::{
    sync::mpsc,
    time::{Instant, sleep, timeout, timeout_at},
};
use twintail_onebot11::account_id;
use twintail_plugin_sdk::{BotRef, PublishEvent};

struct Settings {
    config: Config,
    target: Option<String>,
    connect_wait: Duration,
    event_wait: Duration,
}

fn setting(name: &str) -> Result<String, &'static str> {
    env::var(name).map_err(|_| "missing or non-Unicode required environment setting")
}
fn bounded(name: &str, default: u64, min: u64, max: u64) -> Result<u64, &'static str> {
    let value = match env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| "invalid probe timeout setting")?,
        Err(env::VarError::NotPresent) => default,
        Err(_) => return Err("invalid probe timeout setting"),
    };
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err("probe timeout setting outside allowed range")
    }
}
fn settings() -> Result<Settings, &'static str> {
    let url = setting("TWINTAIL_ONEBOT_URL")?;
    let self_id = account_id(&Value::String(setting("TWINTAIL_ONEBOT_SELF_ID")?))
        .map_err(|_| "invalid self account setting")?;
    let token = match env::var("TWINTAIL_ONEBOT_TOKEN_ENV") {
        Ok(name) if !name.is_empty() => Some(setting(&name)?),
        Ok(_) => return Err("empty token environment variable name"),
        Err(env::VarError::NotPresent) => None,
        Err(_) => return Err("invalid token environment variable name"),
    };
    let target = if env::var("TWINTAIL_SEND_PROBE").ok().as_deref() == Some("1") {
        Some(
            account_id(&Value::String(setting("TWINTAIL_ONEBOT_TARGET")?))
                .map_err(|_| "invalid target account setting")?,
        )
    } else {
        None
    };
    let action_timeout = bounded("TWINTAIL_PROBE_ACTION_TIMEOUT_MS", 3000, 10, 30000)?;
    let connect_wait =
        Duration::from_secs(bounded("TWINTAIL_PROBE_CONNECT_TIMEOUT_SECS", 10, 1, 60)?);
    let event_wait = Duration::from_secs(bounded("TWINTAIL_PROBE_EVENT_TIMEOUT_SECS", 60, 0, 300)?);
    let config=Config::from_value(json!({"url":url,"self_id":self_id,"access_token":token,"action_timeout_ms":action_timeout}))
        .map_err(|_| "invalid transport configuration")?;
    Ok(Settings {
        config,
        target,
        connect_wait,
        event_wait,
    })
}

// Only these version fields are selected. Restrict display text so a malicious
// remote field cannot inject lines or terminal escape sequences into evidence.
fn version_text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| {
            !text.is_empty()
                && text.len() <= 128
                && text
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b" ._+-/()".contains(&b))
        })
        .map(str::to_owned)
}
async fn action(transport: &Transport, name: &str, params: Value) -> Result<Value, &'static str> {
    match transport.action(name.into(), params).await {
        Ok(value) => {
            println!(
                "{}",
                json!({"action":name,"status":value.get("status").and_then(Value::as_str).filter(|s| matches!(*s,"ok"|"failed"|"async")).unwrap_or("other"),"retcode":value.get("retcode").and_then(Value::as_i64)})
            );
            if value.get("status").and_then(Value::as_str) != Some("ok")
                || value.get("retcode").and_then(Value::as_i64) != Some(0)
            {
                return Err("remote action reported failure; action was not retried");
            }
            Ok(value)
        }
        Err(error) => {
            // Only the protocol-defined error category, never remote text/data.
            println!("{}", json!({"action":name,"error_code":error.code}));
            Err("probe action failed; action was not retried")
        }
    }
}
async fn probe(
    transport: &Transport,
    events: &mut mpsc::Receiver<PublishEvent>,
    settings: &Settings,
) -> Result<(), &'static str> {
    timeout(settings.connect_wait, async {
        while !transport.status().connected {
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| "connection wait deadline exceeded")?;
    println!("{}", json!({"connected":true}));
    let version = action(transport, "get_version_info", json!({})).await?;
    println!(
        "{}",
        json!({"version":{"application":version_text(version.pointer("/data/app_name")),"version":version_text(version.pointer("/data/app_version"))}})
    );
    let login = action(transport, "get_login_info", json!({})).await?;
    let matches = login
        .pointer("/data/user_id")
        .and_then(|value| account_id(value).ok())
        .is_some_and(|id| id == settings.config.self_id);
    println!("{}", json!({"login_self_id_matches":matches}));
    if !matches {
        return Err("login account does not match configured account");
    }
    if let Some(target) = &settings.target {
        // Exactly one attempt. The transport never replays admitted actions.
        let _ = action(
            transport,
            "send_private_msg",
            json!({"user_id":target,"message":"TwinTail P4 连通性测试"}),
        )
        .await?;
    }
    let mut counts = [0u64; 4];
    let deadline = Instant::now() + settings.event_wait;
    loop {
        let event = tokio::select! {
            event=timeout_at(deadline,events.recv())=>match event {Ok(Some(event))=>event,_=>break},
            _=tokio::signal::ctrl_c()=>break,
        };
        // Only count family names. Payloads are neither printed nor persisted.
        let index = match event.event_type.as_str() {
            "message" => Some(0),
            "notice" => Some(1),
            "request" => Some(2),
            "meta_event" => Some(3),
            _ => None,
        };
        if let Some(index) = index {
            counts[index] += 1;
        }
    }
    println!(
        "{}",
        json!({"events":{"message":counts[0],"notice":counts[1],"request":counts[2],"meta_event":counts[3]}})
    );
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let settings = match settings() {
        Ok(settings) => settings,
        Err(reason) => {
            eprintln!("{reason}");
            return ExitCode::FAILURE;
        }
    };
    let bot = BotRef {
        adapter_instance_id: "manual-real-probe".into(),
        self_id: settings.config.self_id.clone(),
    };
    let (transport, mut events) = match Transport::spawn(settings.config.clone(), bot) {
        Ok(value) => value,
        Err(_) => {
            eprintln!("transport setup failed");
            return ExitCode::FAILURE;
        }
    };
    let result = tokio::select! {
        result=probe(&transport,&mut events,&settings)=>result,
        _=tokio::signal::ctrl_c()=>Err("probe interrupted; admitted actions were not retried"),
    };
    transport.stop().await;
    println!("{}", json!({"stopped":transport.status().phase=="stopped"}));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(reason) => {
            eprintln!("{reason}");
            ExitCode::FAILURE
        }
    }
}
