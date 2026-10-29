//! Opt-in CLI first-install smoke, using only an isolated loopback OneBot mock.
//!
//! Build twintail-host, then run with TWINTAIL_HOST_BIN set to its absolute path:
//! `cargo test -p adapter-snowluma --test cli_p4 -- --ignored --nocapture`.
#![cfg(unix)]
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::TcpListener,
    process::{Child, Command},
    time::{sleep, timeout},
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

const DEADLINE: Duration = Duration::from_secs(15);
struct Cli {
    binary: PathBuf,
    data: PathBuf,
}
impl Cli {
    async fn call(&self, args: &[&str]) -> Value {
        let output = timeout(
            DEADLINE,
            Command::new(&self.binary)
                .args(args)
                .arg("--data-dir")
                .arg(&self.data)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .expect("CLI deadline")
        .unwrap();
        assert!(
            output.status.success(),
            "CLI {args:?}: {} {}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(envelope["status"], "ok", "CLI result: {envelope}");
        envelope["value"].clone()
    }
    async fn mutation(&self, args: &[&str]) -> Value {
        let ack = self.call(args).await;
        assert_eq!(ack["state"], "running");
        let id = ack["id"].as_str().expect("operation ID");
        let operation = timeout(DEADLINE, async {
            loop {
                let operation = self.call(&["operations", "get", id]).await;
                if operation["state"] != "running" && operation["state"] != "pending" {
                    break operation;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("operation must reach terminal state");
        assert_eq!(operation["state"], "completed", "CLI {args:?}: {operation}");
        operation
    }
    async fn connection(&self) -> Value {
        self.call(&[
            "plugins",
            "call",
            "adapter-snowluma",
            "connection.status",
            "{}",
        ])
        .await
    }
}
fn private_dir(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn package(root: &Path, id: &str, binary: &str, manifest: &Path, schema: Option<&Path>) -> PathBuf {
    let source = root.join(id);
    private_dir(&source);
    fs::copy(binary, source.join(id)).unwrap();
    fs::set_permissions(source.join(id), fs::Permissions::from_mode(0o700)).unwrap();
    fs::copy(manifest, source.join("plugin.toml")).unwrap();
    if let Some(schema) = schema {
        fs::copy(schema, source.join("config.schema.json")).unwrap();
    }
    source
}
async fn host(cli: &Cli, packages: &Path) -> Child {
    let mut child = Command::new(&cli.binary)
        .arg("--data-dir")
        .arg(&cli.data)
        .arg("--install-dir")
        .arg(packages)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    timeout(DEADLINE, async {
        loop {
            let line = lines
                .next_line()
                .await
                .unwrap()
                .expect("host exited before ready");
            if line.contains("host ready") {
                break;
            }
        }
    })
    .await
    .expect("host startup deadline");
    // Keep draining diagnostics so a full pipe cannot block the host.
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    child
}

#[tokio::test]
#[ignore = "requires an explicitly built twintail-host via TWINTAIL_HOST_BIN"]
async fn documented_private_config_install_enable_status_and_echo_via_actual_cli() {
    let host_binary = std::env::var_os("TWINTAIL_HOST_BIN")
        .expect("set TWINTAIL_HOST_BIN to the built host executable");
    let adapter_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo = adapter_root.parent().unwrap().parent().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    private_dir(&data);
    private_dir(&data.join("plugins"));
    private_dir(&data.join("plugins/adapter-snowluma"));
    let packages = temp.path().join("packages");
    private_dir(&packages);
    let source = temp.path().join("source");
    private_dir(&source);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = data.join("plugins/adapter-snowluma/config.toml");
    fs::write(&config, format!("config_version = 1\nrevision = 1\n\n[values]\nurl = 'ws://{}/onebot/v11/ws'\nself_id = '10001'\n", listener.local_addr().unwrap())).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let adapter = package(
        &source,
        "adapter-snowluma",
        env!("CARGO_BIN_EXE_adapter-snowluma"),
        &adapter_root.join("plugin.toml"),
        Some(&adapter_root.join("config.schema.json")),
    );
    let echo = package(
        &source,
        "echo-plugin",
        env!("CARGO_BIN_EXE_p4-echo-fixture"),
        &repo.join("examples/echo-plugin/plugin-snowluma.toml"),
        None,
    );
    let cli = Cli {
        binary: PathBuf::from(host_binary).canonicalize().unwrap(),
        data,
    };
    let mut child = host(&cli, &packages).await;
    cli.mutation(&["plugins", "install", adapter.to_str().unwrap()])
        .await;
    cli.mutation(&[
        "plugins",
        "enable",
        "adapter-snowluma",
        "0.1.0",
        r#"["bots.register","events.publish","services.provide"]"#,
    ])
    .await;
    let mut socket = timeout(DEADLINE, async {
        let (stream, _) = listener.accept().await.unwrap();
        accept_async(stream).await.unwrap()
    })
    .await
    .unwrap();
    timeout(DEADLINE, async {
        while cli.connection().await["connected"] != true {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("adapter connection");
    let config = cli.call(&["config", "get", "adapter-snowluma"]).await;
    assert_eq!(config["status"]["desired_revision"], 1);
    assert_eq!(config["status"]["applied_revision"], 1);
    cli.mutation(&["plugins", "install", echo.to_str().unwrap()])
        .await;
    cli.mutation(&[
        "plugins",
        "enable",
        "echo-plugin",
        "0.1.0",
        r#"["events.subscribe","actions.call"]"#,
    ])
    .await;
    assert_eq!(
        cli.call(&["plugins", "call", "echo-plugin", "echo.ready", "{}"])
            .await["ready"],
        true
    );
    socket.send(Message::Text(json!({"time":1,"self_id":10001,"post_type":"message","message_type":"group","sub_type":"normal","message_id":1,"group_id":12345,"user_id":67890,"message":[{"type":"text","data":{"text":"CLI loopback only"}}]}).to_string().into())).await.unwrap();
    let frame = timeout(DEADLINE, socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(text) = frame else {
        panic!("expected echo action, got {frame:?}")
    };
    let action: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(action["action"], "send_msg");
    assert_eq!(action["params"]["group_id"], 12345);
    assert_eq!(
        action["params"]["message"],
        json!([{"type":"text","data":{"text":"CLI loopback only"}}])
    );
    socket
        .send(Message::Text(
            json!({"status":"ok","retcode":0,"data":{"message_id":2},"echo":action["echo"]})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    timeout(Duration::from_secs(1), async {
        while cli.connection().await["pending"] != 0 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("response correlation before 3s action expiry");
    assert_eq!(cli.connection().await["rejected_frames"], 0);
    assert_eq!(
        fs::metadata(cli.data.join("plugins/adapter-snowluma/config.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(cli.data.join("operations.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(
        Command::new("kill")
            .arg("-TERM")
            .arg(child.id().unwrap().to_string())
            .status()
            .await
            .unwrap()
            .success()
    );
    let status = timeout(DEADLINE, child.wait()).await.unwrap().unwrap();
    assert!(status.success(), "host must reap SDK processes on shutdown");
    eprintln!(
        "CLI smoke passed: private preset -> install -> enable -> desired/applied=1 -> connected -> echo send_msg/response -> graceful shutdown; loopback only"
    );
}
