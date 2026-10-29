//! Reproducible synthetic overload and paced echo soak. Build p6-fixture first.
//! Args: --fixture PATH --duration-secs 1..300 --event-count 1..10000
//!       --batch-size 1..256 (ceil(event-count/batch-size)<=128) --queue-policy drop-newest (only supported policy).
//! Counts are observed at API boundaries; internal allocations/tasks are not measured.
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::time::{sleep_until, timeout};
use twintail_core::{Limits, Manifest, PluginSpec, Supervisor};
use twintail_protocol::{ErrorCode, RpcError};
fn failure() -> RpcError {
    RpcError::new(ErrorCode::InvalidRequest, "soak invariant failed")
}
fn spec(mode: &str, fixture: &Path, pid: PathBuf) -> PluginSpec {
    let text = match mode {
        "mock" => include_str!("../../../examples/mock-adapter/plugin.toml"),
        "echo" => include_str!("../../../examples/echo-plugin/plugin.toml"),
        _ => {
            "id='soak-slow'\nname='soak-slow'\nversion='0.1.0'\nprotocol_version=1\nhost_api='^0.1.0'\nentrypoint='unused'\ncapabilities=['events.subscribe']\nrequired_capabilities=['events.subscribe']\n"
        }
    };
    let mut manifest: Manifest = toml::from_str(text).unwrap();
    manifest.args = vec![mode.into(), pid.to_string_lossy().into()];
    PluginSpec {
        package_dir: fixture.parent().unwrap().into(),
        executable: fixture.into(),
        grants: manifest.capabilities.clone(),
        manifest,
    }
}
async fn call(
    host: &mut Supervisor,
    id: &str,
    method: &str,
    value: Value,
) -> Result<Value, RpcError> {
    host.call(id, method.into(), value).await
}
async fn actions(host: &mut Supervisor, count: u64, first: u64) -> Result<(), RpcError> {
    timeout(Duration::from_secs(3), async {
        loop {
            let values = call(host, "mock-adapter", "mock.actions", json!({})).await?;
            if values.as_array().is_some_and(|v| v.len() as u64 == count) {
                let mut indices = values
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| {
                        if v["action"] != "send_msg" || v["bot"]["self_id"] != "10001" {
                            return None;
                        }
                        v["params"]["message"][0]["data"]["text"]
                            .as_str()?
                            .strip_prefix("soak-")?
                            .parse::<u64>()
                            .ok()
                    })
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(failure)?;
                indices.sort_unstable();
                if indices != (first..first + count).collect::<Vec<_>>() {
                    return Err(failure());
                }
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .map_err(|_| failure())?
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (report, code) = run(std::env::args().skip(1).collect()).await;
    println!("{report}");
    std::process::exit(code);
}
pub(crate) async fn run(args: Vec<String>) -> (Value, i32) {
    let mut fixture = None;
    let mut duration = 1u64;
    let mut count = 512u64;
    let mut batch = 128u64;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let Some(value) = iter.next() else {
            return usage();
        };
        match arg.as_str() {
            "--fixture" => fixture = Some(PathBuf::from(value)),
            "--duration-secs" => match value.parse() {
                Ok(n) if (1..=300).contains(&n) => duration = n,
                _ => return usage(),
            },
            "--event-count" => match value.parse() {
                Ok(n) if (1..=10000).contains(&n) => count = n,
                _ => return usage(),
            },
            "--batch-size" => match value.parse() {
                Ok(n) if (1..=256).contains(&n) => batch = n,
                _ => return usage(),
            },
            "--queue-policy" if value == "drop-newest" => {}
            _ => return usage(),
        }
    }
    if count.div_ceil(batch) > 128 {
        return usage();
    }
    let Some(fixture) = fixture
        .and_then(|p| p.canonicalize().ok())
        .filter(|p| p.is_file())
    else {
        return usage();
    };
    let data = tempfile::tempdir().unwrap();
    let pids: Vec<_> = (0..3)
        .map(|i| data.path().join(format!("pid-{i}")))
        .collect();
    let specs = vec![
        spec("mock", &fixture, pids[0].clone()),
        spec("echo", &fixture, pids[1].clone()),
        spec("slow", &fixture, pids[2].clone()),
    ];
    let mut host = Supervisor::new(
        specs,
        data.path().into(),
        Limits {
            handshake: Duration::from_secs(3),
            request: Duration::from_secs(3),
            stop: Duration::from_millis(100),
        },
    )
    .unwrap();
    let clock = Instant::now();
    let mut offered = 0u64;
    let mut delivered = 0u64;
    let mut dropped = 0u64;
    let mut completed = 0u64;
    let mut rounds = 0u64;
    let mut health_checks = 0u64;
    let mut peak = 0u64;
    let mut observed_pids = Vec::new();
    let mut stage = "startup_mock";
    let result: Result<(), RpcError> = timeout(Duration::from_secs(duration + 30), async {
        host.start("mock-adapter").await?;
        stage = "startup_slow";
        host.start("soak-slow").await?;
        stage = "park_slow";
        loop {
            let ready = call(&mut host, "soak-slow", "slow.ready", json!({})).await?;
            if ready["ready"] == true {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        let parked = call(&mut host, "soak-slow", "slow.park", json!({})).await?;
        if parked["parked"] != true {
            return Err(failure());
        }
        // Force the nonreader pipe and 32-entry event lane to fill before echo starts.
        // 128x128KiB stays below frame limits, with bounded sequential publisher RPCs.
        stage = "overload";
        let padding = "x".repeat(131072);
        for _ in 0..128 {
            let report = call(
                &mut host,
                "mock-adapter",
                "mock.emit",
                json!({"padding":padding}),
            )
            .await?;
            offered += 1;
            delivered += report["delivered"].as_u64().ok_or_else(failure)?;
            dropped += report["dropped"].as_u64().ok_or_else(failure)?;
        }
        if dropped == 0 {
            return Err(failure());
        }
        // Retire the deliberate nonreader after proving drop-newest. Healthy routes
        // must remain usable after this subscriber alone fills or fails its pipe.
        host.stop("soak-slow").await?;
        stage = "start_echo";
        host.start("echo-plugin").await?;
        stage = "steady";
        rounds = 1;
        let steady = tokio::time::Instant::now();
        let mut in_generation = 0u64;
        for index in 0..count {
            let report = call(
                &mut host,
                "mock-adapter",
                "mock.emit",
                json!({"message":[{"type":"text","data":{"text":format!("soak-{index}")}}]}),
            )
            .await?;
            if report["delivered"] != 1 || report["dropped"] != 0 {
                return Err(failure());
            }
            offered += 1;
            delivered += 1;
            in_generation += 1;
            // Per-generation limit explicitly refreshes mock history1024 and SDK
            // request-ID4096. No automatic replay and no hidden unbounded history.
            if in_generation == batch || index + 1 == count {
                actions(&mut host, in_generation, index + 1 - in_generation).await?;
                completed += in_generation;
                peak = peak.max(in_generation);
                let health = call(&mut host, "echo-plugin", "host.health", json!({})).await?;
                if health["status"] != "ok" {
                    return Err(failure());
                }
                health_checks += 1;
                for path in &pids {
                    if let Ok(text) = std::fs::read_to_string(path)
                        && let Ok(pid) = text.parse::<u32>()
                        && !observed_pids.contains(&pid)
                    {
                        observed_pids.push(pid);
                    }
                }
                if index + 1 < count {
                    host.stop("echo-plugin").await?;
                    host.stop("mock-adapter").await?;
                    host.start("mock-adapter").await?;
                    host.start("echo-plugin").await?;
                    rounds += 1;
                    in_generation = 0;
                }
            }
            // Pace actual work across the requested duration, rather than merely
            // sleeping after a burst and labelling that idle wait as a soak.
            sleep_until(
                steady + Duration::from_secs(duration).mul_f64((index + 1) as f64 / count as f64),
            )
            .await;
        }
        Ok(())
    })
    .await
    .unwrap_or_else(|_| Err(RpcError::new(ErrorCode::Timeout, "soak total deadline")));
    for path in &pids {
        if let Ok(text) = std::fs::read_to_string(path)
            && let Ok(pid) = text.parse::<u32>()
            && !observed_pids.contains(&pid)
        {
            observed_pids.push(pid);
        }
    }
    host.shutdown().await;
    let no_active = host
        .list()
        .iter()
        .all(|s| s.state == "stopped" || s.state == "failed");
    #[cfg(target_os = "linux")]
    let pids_gone = observed_pids
        .iter()
        .all(|pid| !Path::new(&format!("/proc/{pid}")).exists());
    #[cfg(not(target_os = "linux"))]
    let pids_gone = true;
    let ok = result.is_ok() && no_active && pids_gone && completed == count;
    (
        json!({"schema_version":1,"tool":"bounded_soak","ok":ok,"stage":stage,"error_code":result.err().map(|e|e.code),"limits":{"duration_secs":duration,"event_count":count,"batch_size":batch,"queue_policy":"drop-newest","host_event_queue":32,"sdk_event_queue":32,"sdk_handlers":16,"maximum_generations":128,"total_work_deadline_secs":duration+30,"request_id_budget_per_connection":4096,"mock_history_limit":1024,"generation_refresh_every_events":batch,"overload_events":128,"overload_payload_bytes":131072},"observed":{"offered":offered,"delivered":delivered,"dropped":dropped,"unrouted_after_slow_retirement":offered-delivered-dropped,"echo_actions":completed,"generations":rounds,"health_checks":health_checks,"peak_observed_history":peak,"elapsed_ms":clock.elapsed().as_millis()},"cleanup":{"shutdown_returned":true,"no_active_instances":no_active,"recorded_child_pids":observed_pids.len(),"recorded_pids_gone":pids_gone,"pid_measurement":if cfg!(target_os="linux"){ "linux_proc" }else{ "unavailable; shutdown_join_only" },"internal_pending_tasks_measured":false},"scope":"slow consumer may be retired on pipe-write timeout; unrouted counts have no subscribers. synthetic local IPC only; counters are API observations, not memory/task metrics or SnowLuma certification"}),
        if ok { 0 } else { 1 },
    )
}
fn usage() -> (Value, i32) {
    (
        json!({"schema_version":1,"ok":false,"stage":"arguments","usage":"bounded_soak --fixture TRUSTED_P6_FIXTURE --duration-secs 1..300 --event-count 1..10000 --batch-size 1..256 (ceil(event-count/batch-size)<=128) --queue-policy drop-newest"}),
        2,
    )
}
