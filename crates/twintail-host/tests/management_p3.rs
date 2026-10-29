//! Management P3 contracts through the actual host executable and Unix endpoint.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use twintail_core::PackageStore;
use twintail_protocol::{Message, Outcome};
fn write_json(stream: &mut UnixStream, message: &Message) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(message)?;
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)
}
fn read_json(stream: &mut UnixStream) -> std::io::Result<Message> {
    let mut prefix = [0; 4];
    stream.read_exact(&mut prefix)?;
    let length = u32::from_be_bytes(prefix) as usize;
    assert!(length <= 1024 * 1024);
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

struct Host {
    child: Child,
    data: PathBuf,
}
impl Host {
    fn start(data: &Path, packages: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_twintail-host"))
            .arg("--data-dir")
            .arg(data)
            .arg("--install-dir")
            .arg(packages)
            .env("TWINTAIL_P3_SECRET", "never-record-resolved-secret")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stderr = child.stderr.take().unwrap();
        let host_pid = child.id();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
            {
                eprintln!("host {host_pid}: {line}");
                let _ = tx.send(line);
            }
        });
        let mut host = Self {
            child,
            data: data.to_owned(),
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let line = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|_| panic!("host startup timeout: {:?}", host.child.try_wait()));
            if line.contains("host ready") {
                break;
            }
        }
        host
    }
    fn raw(&self, method: &str, payload: Value) -> Value {
        let stream = UnixStream::connect(self.data.join("management.sock")).unwrap();
        self.raw_connected(stream, method, payload)
    }
    fn raw_connected(&self, mut stream: UnixStream, method: &str, payload: Value) -> Value {
        stream
            .set_read_timeout(Some(Duration::from_secs(8)))
            .unwrap();
        write_json(&mut stream, &Message::request("p3-test", method, payload)).unwrap();
        let response: Message = read_json(&mut stream).unwrap_or_else(|error| {
            panic!(
                "host {} management {method} read failed: {error}",
                self.child.id()
            )
        });
        match response.body {
            twintail_protocol::Body::Response {
                outcome: Outcome::Ok(value),
                ..
            } => value,
            other => panic!("management response: {other:?}"),
        }
    }
    fn cli(&self, arguments: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_twintail-host"))
            .args(arguments)
            .arg("--data-dir")
            .arg(&self.data)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        match serde_json::from_slice::<Outcome>(&output.stdout).unwrap() {
            Outcome::Ok(value) => value,
            Outcome::Err(error) => panic!("CLI error: {error}"),
        }
    }
    fn terminal(&self, id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let operation = self.raw("operations.get", json!({"id":id}));
            if operation["state"] != "running" && operation["state"] != "pending" {
                return operation;
            }
            assert!(
                Instant::now() < deadline,
                "operation did not finish: {operation}"
            );
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    fn shutdown(&mut self) {
        assert!(
            Command::new("kill")
                .arg("-TERM")
                .arg(self.child.id().to_string())
                .status()
                .unwrap()
                .success()
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(15));
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn source(root: &Path, name: &str, mode: &str, version: &str) -> PathBuf {
    let dir = root.join(name);
    fs::create_dir(&dir).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_p1-fixture"), dir.join("fixture")).unwrap();
    fs::set_permissions(dir.join("fixture"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(dir.join("plugin.toml"),format!("id='plugin'\nname='management fixture'\nversion='{version}'\nprotocol_version=1\nhost_api='^0.1'\nentrypoint='fixture'\nargs=['{mode}','plugin','1200']\n")).unwrap();
    dir
}
fn plan(data: &Path, version: &str) {
    fs::create_dir_all(data).unwrap();
    fs::set_permissions(data, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        data.join("host.toml"),
        format!("[[plugins]]\npath='plugin/{version}'\ngrants=[]\n"),
    )
    .unwrap();
    fs::set_permissions(data.join("host.toml"), fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn cli_install_is_disk_only_and_damaged_disabled_package_can_be_uninstalled() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let packages = temp.path().join("packages");
    let source = source(temp.path(), "source", "normal", "0.1.0");
    let mut host = Host::start(&data, &packages);
    let operation = host.cli(&["plugins", "install", source.to_str().unwrap()]);
    assert!(operation["id"].as_str().unwrap().starts_with("op-"));
    assert_eq!(
        host.terminal(operation["id"].as_str().unwrap())["state"],
        "completed"
    );
    assert_eq!(host.cli(&["plugins", "list"]), json!([]));
    assert_eq!(host.cli(&["plugins", "installed"])[0]["id"], "plugin");
    let enable = host.cli(&["plugins", "enable", "plugin", "0.1.0", "[]"]);
    assert_eq!(
        host.terminal(enable["id"].as_str().unwrap())["state"],
        "completed"
    );
    assert_eq!(
        host.cli(&["plugins", "status", "plugin"])["state"],
        "active"
    );
    let disable = host.cli(&["plugins", "disable", "plugin"]);
    assert_eq!(
        host.terminal(disable["id"].as_str().unwrap())["state"],
        "completed"
    );
    fs::write(packages.join("plugin/0.1.0/fixture"), b"damaged executable").unwrap();
    let remove = host.cli(&["plugins", "uninstall", "plugin", "0.1.0"]);
    assert_eq!(
        host.terminal(remove["id"].as_str().unwrap())["state"],
        "completed"
    );
    assert!(!packages.join("plugin/0.1.0").exists());
    assert!(data.join("plugins/plugin").exists());
    assert_eq!(
        fs::metadata(data.join("operations.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    host.shutdown();
}

#[test]
fn disconnected_reload_runs_in_background_queries_and_idempotent_retry_remain_available() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let packages = temp.path().join("packages");
    let a = source(temp.path(), "a", "normal", "0.1.0");
    let b = source(temp.path(), "b", "slow-handshake-v2", "0.2.0");
    let store = PackageStore::new(&packages).unwrap();
    store.install_local(a).unwrap();
    store.install_local(b).unwrap();
    plan(&data, "0.1.0");
    let mut host = Host::start(&data, &packages);
    let payload = json!({"id":"plugin","version":"0.2.0","request_key":"reload-once"});
    let mut stream = UnixStream::connect(data.join("management.sock")).unwrap();
    write_json(
        &mut stream,
        &Message::request("disconnect", "plugins.reload", payload.clone()),
    )
    .unwrap();
    drop(stream);
    let deadline = Instant::now() + Duration::from_secs(3);
    let operation = loop {
        let operations = host.raw("operations.list", json!({}));
        if let Some(operation) = operations
            .as_array()
            .unwrap()
            .iter()
            .find(|op| op["request_key"] == "reload-once")
        {
            break operation.clone();
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(operation["state"], "running");
    let started = Instant::now();
    let retry = host.raw("plugins.reload", payload.clone());
    assert_eq!(retry["id"], operation["id"]);
    assert!(started.elapsed() < Duration::from_millis(500));
    let mut stream = UnixStream::connect(data.join("management.sock")).unwrap();
    write_json(
        &mut stream,
        &Message::request(
            "mismatch",
            "plugins.reload",
            json!({"id":"plugin","version":"0.1.0","request_key":"reload-once"}),
        ),
    )
    .unwrap();
    let mismatch: Message = read_json(&mut stream).unwrap();
    assert!(matches!(
        mismatch.body,
        twintail_protocol::Body::Response {
            outcome: Outcome::Err(twintail_protocol::RpcError {
                code: twintail_protocol::ErrorCode::Conflict,
                ..
            }),
            ..
        }
    ));
    assert_eq!(
        host.raw("plugins.status", json!({"id":"plugin"}))["version"],
        "0.1.0"
    );
    let done = host.terminal(operation["id"].as_str().unwrap());
    assert_eq!(done["result"]["outcome"], "reloaded");
    assert_eq!(host.raw("plugins.reload", payload)["id"], operation["id"]);
    assert_eq!(
        host.cli(&["plugins", "status", "plugin"])["version"],
        "0.2.0"
    );
    assert!(
        fs::read_to_string(data.join("host.toml"))
            .unwrap()
            .contains("plugin/0.2.0")
    );
    host.shutdown();
}

#[test]
fn offline_config_keeps_references_unapplied_and_journal_recovery_is_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let packages = temp.path().join("packages");
    let package = source(temp.path(), "source", "normal", "0.1.0");
    PackageStore::new(&packages)
        .unwrap()
        .install_local(package)
        .unwrap();
    plan(&data, "0.1.0");
    let mut host = Host::start(&data, &packages);
    host.cli(&["plugins", "stop", "plugin"]);
    let update = host.cli(&[
        "config",
        "update",
        "plugin",
        "0",
        r#"{"token":{"$secret":"TWINTAIL_P3_SECRET"}}"#,
    ]);
    let operation = host.terminal(update["id"].as_str().unwrap());
    assert_eq!(operation["state"], "completed");
    let config = host.cli(&["config", "get", "plugin"]);
    assert_eq!(
        config["snapshot"]["values"]["token"]["$secret"],
        "TWINTAIL_P3_SECRET"
    );
    assert_eq!(config["status"]["desired_revision"], 1);
    assert!(config["status"]["applied_revision"].is_null());
    let unsupported = Command::new(env!("CARGO_BIN_EXE_twintail-host"))
        .args(["plugins", "start", "plugin", "--data-dir"])
        .arg(&data)
        .output()
        .unwrap();
    assert!(!unsupported.status.success());
    assert_eq!(
        host.cli(&["plugins", "status", "plugin"])["state"],
        "stopped"
    );
    assert!(host.cli(&["config", "get", "plugin"])["status"]["applied_revision"].is_null());
    let journal = fs::read_to_string(data.join("operations.json")).unwrap();
    assert!(!journal.contains("never-record-resolved-secret"));
    assert!(!journal.contains("$secret"));
    host.shutdown();
    drop(host);
    // Simulate process loss after admission; startup must classify pending records.
    let journal = (0..128).map(|index| json!({"id":format!("old-{index}"),"state":"running","stage":"preparing","method":"config.update","target":"plugin"})).collect::<Vec<_>>();
    fs::write(
        data.join("operations.json"),
        serde_json::to_vec(&journal).unwrap(),
    )
    .unwrap();
    // Keep configured package enabled but clear desired revision for legacy startup.
    fs::remove_file(data.join("plugins/plugin/config.toml")).unwrap();
    let mut host = Host::start(&data, &packages);
    let operations = host.cli(&["operations", "list"]);
    assert_eq!(operations.as_array().unwrap().len(), 128);
    assert_eq!(operations[0]["state"], "interrupted");
    assert_eq!(
        operations.as_array().unwrap().last().unwrap()["method"],
        "plugins.start"
    );
    host.shutdown();
}

#[test]
fn startup_queries_are_responsive_and_interrupted_bootstrap_is_recovered() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let packages = temp.path().join("packages");
    let held = temp.path().join("handshake-held");
    let release = held.with_extension("release");
    let package = source(temp.path(), "source", "held-handshake", "0.1.0");
    let manifest = package.join("plugin.toml");
    let mut value: toml::Value = toml::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
    value["args"] = toml::Value::try_from(vec![
        "held-handshake",
        "plugin",
        "unused",
        held.to_str().unwrap(),
    ])
    .unwrap();
    fs::write(manifest, toml::to_string(&value).unwrap()).unwrap();
    PackageStore::new(&packages)
        .unwrap()
        .install_local(package)
        .unwrap();
    plan(&data, "0.1.0");
    let mut child = Command::new(env!("CARGO_BIN_EXE_twintail-host"))
        .arg("--data-dir")
        .arg(&data)
        .arg("--install-dir")
        .arg(&packages)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let host_pid = child.id();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            eprintln!("handshake host {host_pid}: {line}");
        }
    });
    let mut host = Host {
        child,
        data: data.clone(),
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    let stream = loop {
        assert!(
            host.child.try_wait().unwrap().is_none(),
            "host exited before endpoint became connectable"
        );
        match UnixStream::connect(data.join("management.sock")) {
            Ok(stream) => break stream,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                assert!(
                    Instant::now() < deadline,
                    "management endpoint readiness timeout: {error}"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("management endpoint connection failed: {error}"),
        }
    };
    // The fixture cannot send ready until this test releases the barrier.
    // Await explicit entry into handshake, then require a bounded query response
    // while startup remains blocked, independently of scheduler wall-clock speed.
    let deadline = Instant::now() + Duration::from_secs(8);
    while !held.is_file() {
        assert!(
            host.child.try_wait().unwrap().is_none(),
            "host exited before handshake barrier"
        );
        assert!(
            Instant::now() < deadline,
            "fixture did not enter handshake barrier"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!release.exists());
    let operations = host.raw_connected(stream, "operations.list", json!({}));
    assert!(
        !release.exists(),
        "query must finish before handshake is released"
    );
    let operation = operations.as_array().unwrap().last().unwrap();
    assert_eq!(operation["state"], "running");
    let id = operation["id"].as_str().unwrap().to_owned();
    host.shutdown();
    drop(host);
    fs::write(release, b"allow restarted bootstrap").unwrap();
    let mut host = Host::start(&data, &packages);
    assert_eq!(
        host.raw("operations.get", json!({"id":id}))["state"],
        "interrupted"
    );
    host.shutdown();
}
