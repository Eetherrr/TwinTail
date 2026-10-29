//! Opt-in, loopback-only HTTP → SDK IPC → actual host/CLI integration.
//!
//! Build `twintail-host`, `web-admin`, `mock-adapter`, and `echo-plugin`, then run:
//! `TWINTAIL_HOST_BIN=$PWD/target/debug/twintail-host cargo test -p web-admin --test cli_p5 -- --ignored --nocapture`.
//! The other binaries default to siblings of TWINTAIL_HOST_BIN; override them with
//! TWINTAIL_MOCK_BIN / TWINTAIL_ECHO_BIN when using a different build directory.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    process::{Child, Command},
    time::{sleep, timeout},
};

const DEADLINE: Duration = Duration::from_secs(20);
// Synthetic, deliberately unique per run; never passed in argv or printed.
const SECRET_ENV: &str = "TWINTAIL_P5_TEST_ADMIN_TOKEN";
struct Temporary(PathBuf);
impl Temporary {
    fn new() -> Self {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).unwrap();
        let path =
            std::env::temp_dir().join(format!("twintail-p5-{:x}", u128::from_ne_bytes(nonce)));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn private_dir(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn private_write(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
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
        // Never include stdout/stderr: these assertions also exercise secret-safe failure reporting.
        assert!(
            output.status.success(),
            "CLI process failed for {}",
            args[0]
        );
        let envelope: Value = serde_json::from_slice(&output.stdout).expect("CLI JSON envelope");
        assert_eq!(envelope["status"], "ok", "CLI returned an error");
        envelope["value"].clone()
    }
    async fn terminal(&self, id: &str) -> Value {
        timeout(DEADLINE, async {
            loop {
                let operation = self.call(&["operations", "get", id]).await;
                if !matches!(operation["state"].as_str(), Some("running" | "pending")) {
                    return operation;
                }
                sleep(Duration::from_millis(15)).await;
            }
        })
        .await
        .expect("operation deadline")
    }
    async fn mutation(&self, args: &[&str]) -> Value {
        let ack = self.call(args).await;
        let operation = self
            .terminal(ack["id"].as_str().expect("operation ID"))
            .await;
        assert_eq!(operation["state"], "completed", "CLI operation failed");
        operation
    }
}
async fn host(cli: &Cli, packages: &Path, secret: &str) -> (Child, tokio::task::JoinHandle<()>) {
    let mut child = Command::new(&cli.binary)
        .arg("--data-dir")
        .arg(&cli.data)
        .arg("--install-dir")
        .arg(packages)
        .env(SECRET_ENV, secret)
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
                .expect("host exited before readiness");
            assert!(!line.contains(secret), "host diagnostics leaked credential");
            if line.contains("host ready") {
                break;
            }
        }
    })
    .await
    .expect("host startup deadline");
    let secret = secret.to_owned();
    let diagnostics = tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            assert!(
                !line.contains(&secret),
                "host diagnostics leaked credential"
            );
        }
    });
    (child, diagnostics)
}
fn package(
    root: &Path,
    id: &str,
    binary: &Path,
    manifest: &Path,
    schema: Option<&Path>,
) -> PathBuf {
    let source = root.join(id);
    private_dir(&source);
    fs::copy(binary, source.join(id)).expect("build required integration binary first");
    fs::set_permissions(source.join(id), fs::Permissions::from_mode(0o700)).unwrap();
    fs::copy(manifest, source.join("plugin.toml")).unwrap();
    if let Some(schema) = schema {
        fs::copy(schema, source.join("config.schema.json")).unwrap();
    }
    source
}
struct Response {
    status: u16,
    headers: String,
    body: Value,
}
struct Browser {
    address: SocketAddr,
    cookie: String,
    csrf: String,
    secret: String,
}
impl Browser {
    async fn request(
        &self,
        path: &str,
        body: &Value,
        authenticated: bool,
        origin: &str,
        authority: &str,
        csrf: &str,
    ) -> std::io::Result<Response> {
        timeout(DEADLINE, async {
            let mut stream = TcpStream::connect(self.address).await?;
            let text = body.to_string();
            let auth = if authenticated { format!("Cookie: {}\r\nX-CSRF-Token: {csrf}\r\n", self.cookie) } else { String::new() };
            let request = format!("POST {path} HTTP/1.1\r\nHost: {authority}\r\nOrigin: {origin}\r\nContent-Type: application/json\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
            stream.write_all(request.as_bytes()).await?;
            let mut bytes = Vec::new();
            stream.take(2 * 1024 * 1024).read_to_end(&mut bytes).await?;
            let text = String::from_utf8(bytes).expect("HTTP UTF-8");
            assert!(!text.contains(&self.secret), "HTTP response leaked admin credential");
            let (headers, body) = text.split_once("\r\n\r\n").ok_or_else(|| std::io::Error::other("HTTP disconnected before response"))?;
            let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
            Ok(Response {status, headers: headers.into(), body: serde_json::from_str(body).unwrap_or(Value::Null)})
        }).await.map_err(|_| std::io::Error::other("HTTP deadline"))?
    }
    async fn login(&mut self) {
        let response = self
            .request(
                "/api/v1/session",
                &json!({"token":self.secret}),
                false,
                &format!("http://{}", self.address),
                &self.address.to_string(),
                "",
            )
            .await
            .unwrap();
        assert_eq!(response.status, 200);
        self.cookie = response
            .headers
            .lines()
            .find_map(|line| {
                line.split_once(':')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
                    .map(|(_, value)| value.trim().split(';').next().unwrap().to_owned())
            })
            .expect("session cookie");
        assert!(response.headers.to_ascii_lowercase().contains("httponly"));
        assert!(
            response
                .headers
                .to_ascii_lowercase()
                .contains("samesite=strict")
        );
        self.csrf = response.body["csrf_token"]
            .as_str()
            .expect("CSRF token")
            .into();
    }
    async fn manage(&self, method: &str, payload: Value) -> Response {
        self.request(
            "/api/v1/manage",
            &json!({"method":method,"payload":payload}),
            true,
            &format!("http://{}", self.address),
            &self.address.to_string(),
            &self.csrf,
        )
        .await
        .unwrap()
    }
    async fn read(&self, method: &str, payload: Value) -> Value {
        let response = self.manage(method, payload).await;
        assert_eq!(response.status, 200, "management query {method}");
        response.body["result"].clone()
    }
    async fn operation(&self, method: &str, payload: Value) -> Value {
        let ack = self.read(method, payload).await;
        assert_eq!(ack["state"], "running");
        let id = ack["id"].as_str().expect("HTTP operation ID");
        timeout(DEADLINE, async {
            loop {
                let operation = self.read("operations.get", json!({"id":id})).await;
                assert!(operation["stage"].is_string(), "operation progress stage");
                if !matches!(operation["state"].as_str(), Some("running" | "pending")) {
                    return operation;
                }
                sleep(Duration::from_millis(15)).await;
            }
        })
        .await
        .expect("HTTP operation deadline")
    }
}
fn web_values(address: SocketAddr) -> Value {
    json!({"bind":address.to_string(),"admin_token":{"$secret":SECRET_ENV}})
}

#[tokio::test]
#[ignore = "requires explicitly built host, web-admin, mock-adapter and echo-plugin; TWINTAIL_HOST_BIN opts in"]
async fn authenticated_http_management_and_independent_cli_rescue() {
    let binary =
        PathBuf::from(std::env::var_os("TWINTAIL_HOST_BIN").expect("set TWINTAIL_HOST_BIN"))
            .canonicalize()
            .unwrap();
    let bin_dir = binary.parent().unwrap();
    let mock_binary = std::env::var_os("TWINTAIL_MOCK_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| bin_dir.join("mock-adapter"));
    let echo_binary = std::env::var_os("TWINTAIL_ECHO_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| bin_dir.join("echo-plugin"));
    let plugin = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo = plugin.parent().unwrap().parent().unwrap();
    let temp = Temporary::new();
    let data = temp.0.join("data");
    private_dir(&data);
    private_dir(&data.join("plugins/web-admin"));
    let packages = temp.0.join("packages");
    private_dir(&packages);
    let sources = temp.0.join("source");
    private_dir(&sources);
    let reserve = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = reserve.local_addr().unwrap();
    let mut entropy = [0; 32];
    getrandom::fill(&mut entropy).unwrap();
    let secret: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
    private_write(
        &data.join("plugins/web-admin/config.toml"),
        &format!(
            "config_version = 1\nrevision = 1\n[values]\nbind = '{address}'\n[values.admin_token]\n'$secret' = '{SECRET_ENV}'\n"
        ),
    );
    let web = package(
        &sources,
        "web-admin",
        Path::new(env!("CARGO_BIN_EXE_web-admin")),
        &plugin.join("plugin.toml"),
        Some(&plugin.join("config.schema.json")),
    );
    let mock = package(
        &sources,
        "mock-adapter",
        &mock_binary,
        &repo.join("examples/mock-adapter/plugin.toml"),
        None,
    );
    let echo = package(
        &sources,
        "echo-plugin",
        &echo_binary,
        &repo.join("examples/echo-plugin/plugin.toml"),
        None,
    );
    let cli = Cli { binary, data };
    let (mut child, diagnostics) = host(&cli, &packages, &secret).await;
    for source in [&web, &mock, &echo] {
        cli.mutation(&["plugins", "install", source.to_str().unwrap()])
            .await;
    }
    drop(reserve);
    cli.mutation(&[
        "plugins",
        "enable",
        "web-admin",
        "0.1.0",
        r#"["management.read","management.write"]"#,
    ])
    .await;
    let mut browser = Browser {
        address,
        cookie: String::new(),
        csrf: String::new(),
        secret,
    };
    let authority = address.to_string();
    let origin = format!("http://{address}");
    let query = json!({"method":"plugins.list","payload":{}});
    assert_eq!(
        browser
            .request("/api/v1/manage", &query, false, &origin, &authority, "")
            .await
            .unwrap()
            .status,
        401
    );
    assert_eq!(
        browser
            .request(
                "/api/v1/session",
                &json!({"token":"wrong"}),
                false,
                &origin,
                &authority,
                ""
            )
            .await
            .unwrap()
            .status,
        401
    );
    browser.login().await;
    assert_eq!(
        browser
            .request(
                "/api/v1/manage",
                &query,
                true,
                "http://evil.invalid",
                &authority,
                &browser.csrf
            )
            .await
            .unwrap()
            .status,
        403
    );
    assert_eq!(
        browser
            .request(
                "/api/v1/manage",
                &query,
                true,
                &origin,
                "evil.invalid",
                &browser.csrf
            )
            .await
            .unwrap()
            .status,
        403
    );
    assert_eq!(
        browser
            .request("/api/v1/manage", &query, true, &origin, &authority, "wrong")
            .await
            .unwrap()
            .status,
        403
    );
    assert!(browser.read("plugins.list", json!({})).await.is_array());
    let schema = browser
        .read("config.schema", json!({"id":"web-admin"}))
        .await;
    assert!(schema["schema"].is_object());
    assert_eq!(
        schema["snapshot"]["values"]["admin_token"],
        json!({"$secret":SECRET_ENV})
    );
    for (id, grants) in [
        (
            "mock-adapter",
            json!(["bots.register", "events.publish", "services.provide"]),
        ),
        ("echo-plugin", json!(["events.subscribe", "actions.call"])),
    ] {
        assert_eq!(
            browser
                .operation(
                    "plugins.enable",
                    json!({"id":id,"version":"0.1.0","grants":grants})
                )
                .await["state"],
            "completed"
        );
    }
    let before = browser
        .read("plugins.status", json!({"id":"echo-plugin"}))
        .await;
    assert_eq!(
        browser
            .operation("plugins.stop", json!({"id":"echo-plugin"}))
            .await["state"],
        "completed"
    );
    assert_eq!(
        browser
            .read("plugins.status", json!({"id":"echo-plugin"}))
            .await["state"],
        "stopped"
    );
    assert_eq!(
        browser
            .operation("plugins.start", json!({"id":"echo-plugin"}))
            .await["state"],
        "completed"
    );
    assert_eq!(
        browser
            .operation(
                "plugins.reload",
                json!({"id":"echo-plugin","version":"0.1.0"})
            )
            .await["state"],
        "completed"
    );
    let after = browser
        .read("plugins.status", json!({"id":"echo-plugin"}))
        .await;
    assert!(after["generation"].as_u64().unwrap() > before["generation"].as_u64().unwrap());
    browser
        .operation("plugins.stop", json!({"id":"echo-plugin"}))
        .await;
    let snapshot = browser
        .read("config.get", json!({"id":"echo-plugin"}))
        .await;
    let revision = snapshot["snapshot"]["revision"].as_u64().unwrap();
    let update = browser
        .operation(
            "config.update",
            json!({"id":"echo-plugin","expected_revision":revision,"values":{}}),
        )
        .await;
    assert_eq!(update["state"], "completed");
    let conflict = browser
        .operation(
            "config.update",
            json!({"id":"echo-plugin","expected_revision":revision,"values":{}}),
        )
        .await;
    assert_eq!(conflict["state"], "failed");
    assert_eq!(conflict["result"]["code"], "conflict");
    assert_eq!(
        browser
            .read("config.get", json!({"id":"echo-plugin"}))
            .await["snapshot"]["revision"],
        revision + 1
    );
    // Stopping the HTTP owner may interrupt its own response. Recovery uses CLI only.
    let _ = browser
        .request(
            "/api/v1/manage",
            &json!({"method":"plugins.stop","payload":{"id":"web-admin"}}),
            true,
            &origin,
            &authority,
            &browser.csrf,
        )
        .await;
    timeout(DEADLINE, async {
        loop {
            if cli.call(&["plugins", "status", "web-admin"]).await["state"] == "stopped" {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(TcpStream::connect(address).await.is_err());
    // Occupied listener causes actual startup failure; CLI remains available to repair it.
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bad_values = web_values(occupied.local_addr().unwrap()).to_string();
    cli.mutation(&["config", "update", "web-admin", "1", &bad_values])
        .await;
    // start is a legacy synchronous CLI call, so inspect the failure envelope explicitly.
    let failed = timeout(
        DEADLINE,
        Command::new(&cli.binary)
            .args(["plugins", "start", "web-admin", "--data-dir"])
            .arg(&cli.data)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!String::from_utf8_lossy(&failed.stdout).contains(&browser.secret));
    let envelope: Value = serde_json::from_slice(&failed.stdout).unwrap();
    assert_eq!(envelope["status"], "err");
    assert_eq!(
        cli.call(&["plugins", "status", "web-admin"]).await["state"],
        "failed"
    );
    let repaired = web_values(address).to_string();
    cli.mutation(&["config", "update", "web-admin", "2", &repaired])
        .await;
    cli.call(&["plugins", "start", "web-admin"]).await;
    assert_eq!(
        browser.manage("plugins.list", json!({})).await.status,
        401,
        "restart must invalidate the old in-memory session"
    );
    browser.login().await;
    assert!(
        browser
            .read("management.overview", json!({}))
            .await
            .is_object()
    );
    // Re-enable the same binary with read-only host grants; HTTP authentication does
    // not confer the absent management.write capability across the IPC boundary.
    cli.mutation(&["plugins", "disable", "web-admin"]).await;
    cli.mutation(&[
        "plugins",
        "enable",
        "web-admin",
        "0.1.0",
        r#"["management.read"]"#,
    ])
    .await;
    browser.login().await;
    assert!(browser.read("plugins.list", json!({})).await.is_array());
    let denied = browser
        .manage("plugins.start", json!({"id":"echo-plugin"}))
        .await;
    assert_eq!(denied.status, 403);
    assert_eq!(denied.body["error"]["code"], "permission_denied");
    assert_eq!(
        cli.call(&["plugins", "status", "echo-plugin"]).await["state"],
        "stopped"
    );
    // Secrets remain references on disk and in management responses/operation journals.
    for path in [
        cli.data.join("plugins/web-admin/config.toml"),
        cli.data.join("operations.json"),
    ] {
        assert!(!fs::read_to_string(path).unwrap().contains(&browser.secret));
    }
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().unwrap().to_string())
        .status()
        .await
        .unwrap();
    assert!(status.success());
    assert!(
        timeout(DEADLINE, child.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    timeout(DEADLINE, diagnostics)
        .await
        .unwrap()
        .expect("diagnostic secret check");
}
