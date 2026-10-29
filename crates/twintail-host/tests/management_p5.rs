//! P5 bridge contracts through real plugin IPC and local CLI recovery.
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
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
            {
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
        write_json(&mut stream, &Message::request("p5-test", method, payload)).unwrap();
        let response: Message = read_json(&mut stream).unwrap();
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
fn setup(root: &Path, grants: &[&str], method: &str, payload: Value) -> (Host, PathBuf) {
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_p1-fixture"), source.join("fixture")).unwrap();
    fs::set_permissions(source.join("fixture"), fs::Permissions::from_mode(0o700)).unwrap();
    let trigger = root.join("trigger");
    let args = vec![
        "management".to_owned(),
        "plugin".to_owned(),
        json!({"method":method,"payload":payload}).to_string(),
        trigger.to_str().unwrap().to_owned(),
    ];
    fs::write(
        source.join("schema.json"),
        r#"{"type":"object","properties":{"token":{"type":"string","writeOnly":true}}}"#,
    )
    .unwrap();
    fs::write(source.join("plugin.toml"),format!("id='plugin'\nname='P5 bridge fixture'\nversion='0.1.0'\nprotocol_version=1\nhost_api='^0.1'\nentrypoint='fixture'\nconfig_schema='schema.json'\nargs={}\ncapabilities=['management.read','management.write']\n",serde_json::to_string(&args).unwrap())).unwrap();
    let packages = root.join("packages");
    PackageStore::new(&packages)
        .unwrap()
        .install_local(source)
        .unwrap();
    let data = root.join("data");
    fs::create_dir(&data).unwrap();
    fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        data.join("host.toml"),
        format!(
            "[[plugins]]\npath='plugin/0.1.0'\ngrants={}\n",
            serde_json::to_string(grants).unwrap()
        ),
    )
    .unwrap();
    fs::set_permissions(data.join("host.toml"), fs::Permissions::from_mode(0o600)).unwrap();
    (Host::start(&data, &packages), trigger)
}
fn wait_result(host: &Host) -> Value {
    let path = host
        .data
        .join("plugins/plugin/data/fixture-management-result.json");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(bytes) = fs::read(&path)
            && let Ok(value) = serde_json::from_slice(&bytes)
        {
            return value;
        }
        assert!(Instant::now() < deadline, "bridge result missing");
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn plugin_bridge_requires_exact_read_grant_and_never_exposes_arbitrary_call() {
    for (grants, method, expected) in [
        (vec![], "management.overview", "permission_denied"),
        (
            vec!["management.write"],
            "management.overview",
            "permission_denied",
        ),
        (vec!["management.read"], "plugins.call", "permission_denied"),
        (vec!["management.read"], "management.overview", "ok"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (mut host, trigger) = setup(temp.path(), &grants, method, json!({}));
        fs::write(&trigger, b"go").unwrap();
        let result = wait_result(&host);
        if expected == "ok" {
            assert_eq!(result["status"], "ok");
            assert!(result["value"]["plugins"].is_array());
            assert_eq!(
                result["value"]["permissions"],
                json!({"read":true,"write":false})
            );
            assert_eq!(result["value"]["readonly"], true);
        } else {
            assert_eq!(result["status"], "err", "{result}");
            assert_eq!(result["value"]["code"], expected, "{result}");
        }
        host.shutdown();
    }
}
#[test]
fn self_stop_is_background_and_cli_can_recover_while_schema_and_logs_stay_safe() {
    let temp = tempfile::tempdir().unwrap();
    let (mut host, trigger) = setup(
        temp.path(),
        &["management.write"],
        "plugins.stop",
        json!({"id":"plugin"}),
    );
    fs::write(&trigger, b"go").unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let status = host.raw("plugins.status", json!({"id":"plugin"}));
        if status["state"] == "stopped" {
            break;
        }
        assert!(Instant::now() < deadline, "self-stop deadlocked: {status}");
        std::thread::sleep(Duration::from_millis(10));
    }
    let operations = host.raw("operations.list", json!({}));
    assert!(
        operations
            .as_array()
            .unwrap()
            .iter()
            .any(|op| op["method"] == "plugins.stop"
                && op["actor"] == "plugin:plugin"
                && op["state"] == "completed"),
        "{operations}"
    );
    fs::remove_file(trigger).unwrap();
    let started = host.cli(&["plugins", "start", "plugin"]);
    assert!(started["generation"].as_u64().unwrap() > 1);
    assert_eq!(
        host.raw("plugins.status", json!({"id":"plugin"}))["state"],
        "active"
    );
    host.cli(&["plugins", "stop", "plugin"]);
    let update = host.cli(&[
        "config",
        "update",
        "plugin",
        "0",
        r#"{"token":{"$secret":"TWINTAIL_P3_SECRET"}}"#,
    ]);
    assert_eq!(
        host.terminal(update["id"].as_str().unwrap())["state"],
        "completed"
    );
    let schema = host.raw("config.schema", json!({"id":"plugin"}));
    assert_eq!(schema["schema"]["type"], "object");
    assert_eq!(
        schema["snapshot"]["values"]["token"]["$secret"],
        "TWINTAIL_P3_SECRET"
    );
    let logs = host.raw("diagnostics.logs", json!({}));
    assert_eq!(logs["limit"], 128);
    assert!(!logs.to_string().contains("TWINTAIL_P3_SECRET"));
    let inspect = host.raw("plugins.inspect", json!({"id":"plugin","version":"0.1.0"}));
    assert_eq!(
        inspect["manifest"]["capabilities"],
        json!(["management.read", "management.write"])
    );
    for value in [schema, logs, operations] {
        assert!(!value.to_string().contains("never-record-resolved-secret"));
    }
    host.shutdown();
}

#[test]
fn bridge_rejects_inline_write_only_secrets_but_cli_can_migrate_them() {
    for method in [
        "config.get",
        "config.schema",
        "config.validate",
        "config.update",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (mut host, trigger) = setup(
            temp.path(),
            &["management.read", "management.write"],
            method,
            json!({"id":"plugin","expected_revision":1,"values":{"token":"inline-must-never-leak"}}),
        );
        host.cli(&["plugins", "stop", "plugin"]);
        let update = host.cli(&[
            "config",
            "update",
            "plugin",
            "0",
            r#"{"token":"inline-must-never-leak"}"#,
        ]);
        assert_eq!(
            host.terminal(update["id"].as_str().unwrap())["state"],
            "completed"
        );
        host.cli(&["plugins", "start", "plugin"]);
        fs::write(trigger, b"go").unwrap();
        let result = wait_result(&host);
        assert_eq!(result["status"], "err", "{method}: {result}");
        assert_eq!(
            result["value"]["code"], "invalid_config",
            "{method}: {result}"
        );
        assert!(!result.to_string().contains("inline-must-never-leak"));
        assert_eq!(
            host.cli(&["config", "get", "plugin"])["snapshot"]["values"]["token"],
            "inline-must-never-leak"
        );
        let logs = host.raw("diagnostics.logs", json!({}));
        assert!(!logs.to_string().contains("inline-must-never-leak"));
        host.shutdown();
    }
}

#[test]
fn installed_inventory_ignores_bulk_hashing_and_preserves_plugin_ipc() {
    let temp = tempfile::tempdir().unwrap();
    let (mut host, trigger) = setup(
        temp.path(),
        &["management.read"],
        "management.overview",
        json!({}),
    );
    // Inventory describes installed receipt metadata; byte verification stays explicit.
    // This bulk asset must not be hashed on every UI inventory refresh.
    fs::write(
        temp.path().join("packages/plugin/0.1.0/bulk-data"),
        vec![0_u8; 24 * 1024 * 1024],
    )
    .unwrap();
    let mut inventory = UnixStream::connect(host.data.join("management.sock")).unwrap();
    inventory
        .set_read_timeout(Some(Duration::from_secs(12)))
        .unwrap();
    write_json(
        &mut inventory,
        &Message::request("slow-inventory", "plugins.installed", json!({})),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let metadata = host.raw("plugins.inspect", json!({"id":"plugin","version":"0.1.0"}));
    assert_eq!(metadata["verified"], false);
    let started = Instant::now();
    let overview = host.raw("management.overview", json!({}));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "overview blocked behind checksum scan"
    );
    assert!(overview["plugins"].is_array());
    fs::write(trigger, b"go").unwrap();
    let result = wait_result(&host);
    assert_eq!(
        result["status"], "ok",
        "plugin IPC stalled during inventory: {result}"
    );
    let response = read_json(&mut inventory).unwrap();
    match response.body {
        twintail_protocol::Body::Response {
            outcome: Outcome::Ok(packages),
            ..
        } => assert_eq!(packages[0]["verified"], false),
        other => panic!("inventory metadata response: {other:?}"),
    }
    assert_eq!(
        host.raw("plugins.status", json!({"id":"plugin"}))["state"],
        "active"
    );
    host.shutdown();
}
