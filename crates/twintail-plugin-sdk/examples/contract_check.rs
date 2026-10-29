//! Runs only trusted local packages: process isolation is not a sandbox.
//! `--trusted-package DIR` repeats for dependencies; `--grant CAP` is explicit.
//! Optional `--config ID=JSON_FILE` supplies a ConfigSnapshot, never printed.
//! Startup can perform package-defined side effects; no business RPC is invoked.
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Read,
    path::PathBuf,
    time::{Duration, Instant},
};
use twintail_core::{ConfigStore, Limits, Supervisor, load_package};
use twintail_protocol::{ConfigSnapshot, ErrorCode};
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (report, code) = run(std::env::args().skip(1).collect()).await;
    println!("{report}");
    std::process::exit(code);
}
pub(crate) async fn run(args: Vec<String>) -> (Value, i32) {
    let mut paths = Vec::new();
    let mut grants = Vec::new();
    let mut configs = BTreeMap::new();
    let mut millis = 5000u64;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let Some(value) = iter.next() else {
            return usage();
        };
        match arg.as_str() {
            "--trusted-package" => paths.push(PathBuf::from(value)),
            "--grant" => grants.push(value),
            "--timeout-ms" => match value.parse() {
                Ok(n) if (100..=30000).contains(&n) => millis = n,
                _ => return usage(),
            },
            "--config" => {
                let Some((id, path)) = value.split_once('=') else {
                    return usage();
                };
                configs.insert(id.to_owned(), PathBuf::from(path));
            }
            _ => return usage(),
        }
    }
    if paths.is_empty() || paths.len() > 32 || grants.len() > 128 || configs.len() > 32 {
        return usage();
    }
    let mut specs = Vec::new();
    for path in paths {
        match load_package(&path, &grants) {
            Ok(spec) => specs.push(spec),
            Err(e) => {
                return (
                    json!({"schema_version":1,"ok":false,"stage":"validate","code":e.code}),
                    1,
                );
            }
        }
    }
    let data = match tempfile::tempdir() {
        Ok(data) => data,
        Err(_) => return (json!({"ok":false,"stage":"temporary_data"}), 1),
    };
    let limit = Duration::from_millis(millis);
    let mut host = match Supervisor::new(
        specs,
        data.path().into(),
        Limits {
            handshake: limit,
            request: limit,
            stop: limit,
        },
    ) {
        Ok(host) => host,
        Err(e) => return (json!({"ok":false,"stage":"plan","code":e.code}), 1),
    };
    for (id, path) in configs {
        let snapshot = std::fs::File::open(path)
            .ok()
            .and_then(|file| {
                let mut bytes = Vec::new();
                file.take(1048577).read_to_end(&mut bytes).ok()?;
                Some(bytes)
            })
            .filter(|b| b.len() <= 1048576)
            .and_then(|b| serde_json::from_slice::<ConfigSnapshot>(&b).ok());
        let Some(mut snapshot) = snapshot else {
            return (
                json!({"ok":false,"stage":"config","code":"invalid_config"}),
                1,
            );
        };
        let Some(spec) = host.spec(&id) else {
            return (json!({"ok":false,"stage":"config","code":"not_found"}), 1);
        };
        if snapshot.config_version != spec.manifest.config_version {
            return (
                json!({"ok":false,"stage":"config","code":"invalid_config"}),
                1,
            );
        }
        let resolved = ConfigStore::open(
            &data.path().join(&id),
            &spec.package_dir,
            spec.manifest.config_schema.as_deref(),
            spec.manifest.config_version,
        )
        .and_then(|store| {
            store
                .prepare(0, snapshot.values.clone())
                .map(|candidate| candidate.resolved().clone())
        });
        match resolved {
            Ok(values) => snapshot.values = values,
            Err(_) => {
                return (
                    json!({"ok":false,"stage":"config","code":"invalid_config"}),
                    1,
                );
            }
        }
        if let Err(e) = host.set_config(&id, Some(snapshot)) {
            return (json!({"ok":false,"stage":"config","code":e.code}), 1);
        }
    }
    let clock = Instant::now();
    host.start_all().await;
    let mut checks = Vec::new();
    let mut ok = true;
    for state in host.list() {
        let active = state.state == "active";
        ok &= active;
        let health = if active {
            match host.call(&state.id, "host.health".into(), json!({})).await {
                Ok(value) if value["status"] == "ok" => json!({"status":"supported"}),
                Ok(_) => {
                    ok = false;
                    json!({"status":"invalid_response"})
                }
                Err(e) if e.code == ErrorCode::Unsupported => {
                    json!({"status":"optional_unsupported"})
                }
                Err(e) => {
                    ok = false;
                    json!({"status":"failed","code":e.code})
                }
            }
        } else {
            json!({"status":"not_run"})
        };
        checks.push(json!({"id":state.id,"startup":state.state,"startup_error_code":state.error.map(|e|e.code),"health":health}));
    }
    host.shutdown().await;
    let stopped = host
        .list()
        .iter()
        .all(|s| s.state == "stopped" || s.state == "failed");
    ok &= stopped;
    (
        json!({"schema_version":1,"tool":"contract_check","ok":ok,"checks":checks,"cleanup":{"shutdown_returned":true,"no_active_instances":stopped,"graceful_stop_distinguished_from_forced_kill":false,"internal_pending_tasks_measured":false},"elapsed_ms":clock.elapsed().as_millis(),"trust":"trusted native code; startup/stop may have package-defined side effects; no business RPC invoked","limits":{"timeout_ms_per_step":millis,"maximum_packages":32,"maximum_snapshot_bytes":1048576,"grants_scope":"all listed packages; intersection with each manifest"}}),
        if ok { 0 } else { 1 },
    )
}
fn usage() -> (Value, i32) {
    (
        json!({"schema_version":1,"ok":false,"stage":"arguments","usage":"contract_check --trusted-package DIR [--trusted-package DEP_DIR] --grant CAP [--grant CAP] [--config ID=SNAPSHOT_JSON] [--timeout-ms 100..30000]"}),
        2,
    )
}
