//! Exercise the actual executable without plugins or external services.

use std::process::Command;

fn host() -> Command {
    Command::new(env!("CARGO_BIN_EXE_twintail-host"))
}

#[test]
fn startup_check_exits_without_plugins() {
    let output = host().arg("--check").output().expect("run host");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no plugins loaded"));
}

#[test]
fn help_and_version_exit_successfully() {
    for arg in ["--help", "--version"] {
        let output = host().arg(arg).output().expect("run host");
        assert!(output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("TwinTail")
                || String::from_utf8_lossy(&output.stdout).contains("twintail-host")
        );
    }
}

#[test]
fn malformed_arguments_fail_without_starting() {
    for args in [
        vec!["--unknown"],
        vec!["--data-dir"],
        vec!["--data-dir", "--check"],
        vec!["--install-dir", ""],
        vec!["--check", "--check"],
        vec!["plugins", "call"],
        vec!["plugins", "call", "mock-adapter"],
        vec!["plugins", "call", "mock-adapter", "mock.emit"],
        vec!["plugins", "call", "mock-adapter", "mock.emit", "{"],
        vec!["plugins", "call", "mock-adapter", "", "{}"],
        vec![
            "plugins",
            "call",
            "mock-adapter",
            "mock.emit",
            "{}",
            "--check",
        ],
    ] {
        let output = host().args(args).output().expect("run host");
        assert_eq!(output.status.code(), Some(2));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("host ready"));
    }
}
