//! Host and local recovery CLI; paths remain OS strings.
use serde_json::{Value, json};
use std::{ffi::OsString, path::PathBuf};
pub(crate) const HELP: &str = "TwinTail host (P3)

Usage: twintail-host [--check|--validate] [--data-dir PATH] [--install-dir PATH]
       twintail-host plugins list|installed|status ID|start ID|stop ID
       twintail-host plugins call ID METHOD JSON
       twintail-host plugins install PATH
       twintail-host plugins enable ID VERSION GRANTS_JSON
       twintail-host plugins disable ID|uninstall ID VERSION|reload ID VERSION
       twintail-host config get ID|validate ID JSON|update ID EXPECTED_REVISION JSON
       twintail-host operations list|get ID
       twintail-host --help|--version

All management commands accept --data-dir PATH (default: ./data).
--install-dir PATH selects the package root for normal run (default: ./packages).
--check performs bootstrap checks without reads/writes; --validate validates the
host.toml plan without executing plugins. Normal run starts the enabled plan.
P3 mutations return an operation ID immediately. Poll operations get ID for the
result; a disconnected client does not cancel an admitted operation. Existing
plugins start/stop/call wait for their result. Configuration reads retain secret
references. Local management uses DATA/management.sock on Unix.";
#[derive(Debug, PartialEq)]
pub(crate) enum Command {
    Help,
    Version,
    Run(Options),
    Manage {
        options: Options,
        method: String,
        payload: Value,
    },
}
#[derive(Debug, PartialEq)]
pub(crate) struct Options {
    pub check: bool,
    pub validate: bool,
    pub data_dir: PathBuf,
    pub install_dir: PathBuf,
}
pub(crate) fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let mut options = Options {
        check: false,
        validate: false,
        data_dir: "data".into(),
        install_dir: "packages".into(),
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut manage = None;
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            return Ok(Command::Help);
        }
        if arg == "--version" || arg == "-V" {
            return Ok(Command::Version);
        }
        if matches!(arg.to_str(), Some("plugins" | "config" | "operations")) && manage.is_none() {
            let group = arg.to_str().unwrap();
            let action = text(&mut args, "command requires an action")?;
            let mut payload = json!({});
            match (group, action.as_str()) {
                ("plugins", "list" | "installed") | ("operations", "list") => {}
                ("plugins", "install") => {
                    let path = path_value(&mut args, "install")?;
                    payload["path"] = Value::String(
                        path.into_os_string()
                            .into_string()
                            .map_err(|_| "install path must be UTF-8 for management transport")?,
                    );
                }
                ("plugins", "status" | "start" | "stop" | "disable")
                | ("config", "get")
                | ("operations", "get") => {
                    payload["id"] = Value::String(text(&mut args, "action requires an ID")?);
                }
                ("plugins", "enable" | "reload" | "uninstall") => {
                    payload["id"] = Value::String(text(&mut args, "action requires a plugin ID")?);
                    payload["version"] =
                        Value::String(text(&mut args, "action requires a version")?);
                    if action == "enable" {
                        let grants = json_value(&mut args)?;
                        if !grants
                            .as_array()
                            .is_some_and(|v| v.iter().all(Value::is_string))
                        {
                            return Err("grants must be a JSON string array".into());
                        }
                        payload["grants"] = grants;
                    }
                }
                ("plugins", "call") => {
                    payload["id"] = Value::String(text(&mut args, "call requires a plugin ID")?);
                    let method = text(&mut args, "call requires a method")?;
                    if method.len() > 128 || method.chars().any(char::is_control) {
                        return Err("invalid call method".into());
                    }
                    payload["method"] = Value::String(method);
                    payload["payload"] = json_value(&mut args)?;
                }
                ("config", "validate" | "update") => {
                    payload["id"] = Value::String(text(&mut args, "config requires a plugin ID")?);
                    if action == "update" {
                        let revision = text(&mut args, "update requires expected revision")?
                            .parse::<u64>()
                            .map_err(|_| "expected revision must be an unsigned integer")?;
                        payload["expected_revision"] = json!(revision);
                    }
                    payload["values"] = json_value(&mut args)?;
                }
                _ => return Err("unknown management action".into()),
            }
            manage = Some((format!("{group}.{action}"), payload));
            continue;
        }
        let key = arg.to_str().ok_or("invalid argument")?;
        if !seen.insert(key.to_owned()) {
            return Err(format!("duplicate argument: {key}"));
        }
        match key {
            "--check" => options.check = true,
            "--validate" => options.validate = true,
            "--data-dir" => options.data_dir = path_value(&mut args, key)?,
            "--install-dir" => options.install_dir = path_value(&mut args, key)?,
            _ => return Err(format!("unknown argument: {key}")),
        }
    }
    if options.check && options.validate {
        return Err("--check and --validate are exclusive".into());
    }
    if let Some((method, payload)) = manage {
        if options.check || options.validate || seen.contains("--install-dir") {
            return Err("management only accepts --data-dir".into());
        }
        Ok(Command::Manage {
            options,
            method,
            payload,
        })
    } else {
        Ok(Command::Run(options))
    }
}
fn text(args: &mut impl Iterator<Item = OsString>, error: &str) -> Result<String, String> {
    args.next()
        .and_then(|s| s.into_string().ok())
        .filter(|s| !s.is_empty() && !s.starts_with('-'))
        .ok_or_else(|| error.into())
}
fn json_value(args: &mut impl Iterator<Item = OsString>) -> Result<Value, String> {
    let raw = args
        .next()
        .and_then(|s| s.into_string().ok())
        .ok_or("requires one JSON argument")?;
    serde_json::from_str(&raw).map_err(|_| "invalid JSON argument".into())
}
fn path_value(args: &mut impl Iterator<Item = OsString>, option: &str) -> Result<PathBuf, String> {
    match args.next() {
        Some(value) if !value.is_empty() && !value.to_string_lossy().starts_with('-') => {
            Ok(value.into())
        }
        _ => Err(format!(
            "{option} requires a path (prefix paths starting with '-' with './')"
        )),
    }
}
