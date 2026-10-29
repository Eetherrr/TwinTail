//! Contract CLI implementation and synthetic load with actual supervised children.
#[allow(dead_code)]
#[path = "../examples/contract_check.rs"]
mod contract;
#[allow(dead_code)]
#[path = "../examples/bounded_soak.rs"]
mod soak;
use serde_json::json;
fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}
fn package(mode: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let filename = if cfg!(windows) {
        "fixture.exe"
    } else {
        "fixture"
    };
    std::fs::copy(
        env!("CARGO_BIN_EXE_p6-fixture"),
        directory.path().join(filename),
    )
    .unwrap();
    std::fs::write(directory.path().join("plugin.toml"),format!("id='contract-fixture'\nname='contract-fixture'\nversion='0.1.0'\nprotocol_version=1\nhost_api='^0.1.0'\nentrypoint='{filename}'\nargs=['{mode}']\n")).unwrap();
    directory
}
#[tokio::test]
async fn contract_optional_health_is_not_failure_and_cleanup_completes() {
    let directory = package("optional");
    let (report, code) = contract::run(args(&[
        "--trusted-package",
        directory.path().to_str().unwrap(),
        "--timeout-ms",
        "1000",
    ]))
    .await;
    assert_eq!(code, 0, "{report}");
    assert_eq!(
        report["checks"][0]["health"]["status"],
        "optional_unsupported"
    );
    assert_eq!(report["cleanup"]["no_active_instances"], true);
}
#[tokio::test]
async fn contract_failure_and_argument_errors_are_nonzero_and_safe() {
    let directory = package("fail-start");
    let (report, code) = contract::run(args(&[
        "--trusted-package",
        directory.path().to_str().unwrap(),
        "--timeout-ms",
        "1000",
    ]))
    .await;
    assert_eq!(code, 1);
    assert_eq!(report["ok"], false);
    assert!(!report.to_string().contains("injected start failure"));
    assert_eq!(contract::run(args(&["--timeout-ms", "0"])).await.1, 2);
    assert_eq!(soak::run(args(&["--duration-secs", "301"])).await.1, 2);
    assert_eq!(soak::run(args(&["--queue-policy", "unbounded"])).await.1, 2);
    assert_eq!(
        soak::run(args(&["--event-count", "10000", "--batch-size", "1"]))
            .await
            .1,
        2
    );
    assert_eq!(report["cleanup"]["shutdown_returned"], json!(true));
}
#[tokio::test]
async fn bounded_short_soak_observes_drops_echo_progress_and_child_cleanup() {
    let (report, code) = soak::run(args(&[
        "--fixture",
        env!("CARGO_BIN_EXE_p6-fixture"),
        "--duration-secs",
        "1",
        "--event-count",
        "64",
        "--batch-size",
        "32",
        "--queue-policy",
        "drop-newest",
    ]))
    .await;
    assert_eq!(code, 0, "{report}");
    assert!(report["observed"]["dropped"].as_u64().unwrap() > 0);
    assert_eq!(report["observed"]["echo_actions"], 64);
    assert_eq!(report["observed"]["generations"], 2);
    assert_eq!(report["observed"]["health_checks"], 2);
    assert_eq!(report["cleanup"]["recorded_child_pids"], 5);
    assert_eq!(report["cleanup"]["no_active_instances"], true);
    assert_eq!(report["cleanup"]["recorded_pids_gone"], true);
}
