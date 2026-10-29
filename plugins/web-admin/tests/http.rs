//! Authentication, boundary limits and real TCP ownership tests without a host process.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;
use twintail_plugin_sdk::{ErrorCode, Result, RpcError};
use web_admin::{Config, Management, Server};
const TOKEN: &str = "0c65e8affd352a19c1d7b0a683eeb2ef491e8c2afe23f107a34f3be792040ade";
struct Backend {
    calls: Mutex<Vec<(String, Value)>>,
}
impl Management for Backend {
    fn call(
        &self,
        method: String,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + '_>> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push((method.clone(), payload.clone()));
            match method.as_str() {
                "slow" => {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    Ok(json!({}))
                }
                "failure" => Err(RpcError::new(
                    ErrorCode::Conflict,
                    format!("sensitive upstream {TOKEN}"),
                )),
                "leak" => Ok(json!({"nested":[{"credential":TOKEN}]})),
                _ => Ok(payload),
            }
        })
    }
}
fn config() -> Config {
    Config::from_value(json!({"admin_token":TOKEN,"bind":"127.0.0.1:0"})).unwrap()
}
fn setup(config: Config) -> (Router, Arc<Backend>) {
    let backend = Arc::new(Backend {
        calls: Mutex::new(vec![]),
    });
    (
        web_admin::router(config, "127.0.0.1:17280".parse().unwrap(), backend.clone()).unwrap(),
        backend,
    )
}
fn req(method: &str, path: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:17280")
        .header("origin", "http://127.0.0.1:17280")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}
async fn data(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1048576).await.unwrap()).unwrap()
}
async fn login(app: &Router) -> (String, String) {
    let response = app
        .clone()
        .oneshot(req("POST", "/api/v1/session", json!({"token":TOKEN})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    let csrf = data(response).await["csrf_token"]
        .as_str()
        .unwrap()
        .to_owned();
    (cookie.split(';').next().unwrap().to_owned(), csrf)
}
fn authenticated(
    method: &str,
    path: &str,
    value: Value,
    cookie: &str,
    csrf: &str,
) -> Request<Body> {
    let mut request = req(method, path, value);
    request
        .headers_mut()
        .insert("cookie", cookie.parse().unwrap());
    request
        .headers_mut()
        .insert("x-csrf-token", csrf.parse().unwrap());
    request
}
#[tokio::test]
async fn session_csrf_host_origin_and_revocation() {
    let (app, backend) = setup(config());
    let response = app
        .clone()
        .oneshot(req("GET", "/api/v1/session", json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(
        response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
    let (cookie, csrf) = login(&app).await;
    let response = app
        .clone()
        .oneshot(authenticated(
            "GET",
            "/api/v1/session",
            json!({}),
            &cookie,
            &csrf,
        ))
        .await
        .unwrap();
    assert_eq!(data(response).await["csrf_token"], csrf);
    for kind in ["csrf", "host", "origin", "fetch", "query", "missing-origin"] {
        let mut request = authenticated(
            "POST",
            if kind == "query" {
                "/api/v1/manage?token=secret"
            } else {
                "/api/v1/manage"
            },
            json!({"method":"overview","payload":{}}),
            &cookie,
            &csrf,
        );
        match kind {
            "csrf" => {
                request.headers_mut().remove("x-csrf-token");
            }
            "host" => {
                request
                    .headers_mut()
                    .insert("host", "evil.example".parse().unwrap());
            }
            "origin" => {
                request
                    .headers_mut()
                    .insert("origin", "http://evil.example".parse().unwrap());
            }
            "fetch" => {
                request
                    .headers_mut()
                    .insert("sec-fetch-site", "cross-site".parse().unwrap());
            }
            "missing-origin" => {
                request.headers_mut().remove("origin");
            }
            _ => {}
        }
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            403,
            "{kind}"
        );
    }
    assert!(backend.calls.lock().unwrap().is_empty());
    let response = app
        .clone()
        .oneshot(authenticated(
            "POST",
            "/api/v1/manage",
            json!({"method":"overview","payload":{"ok":true}}),
            &cookie,
            &csrf,
        ))
        .await
        .unwrap();
    assert_eq!(data(response).await, json!({"result":{"ok":true}}));
    assert_eq!(
        app.clone()
            .oneshot(authenticated(
                "DELETE",
                "/api/v1/session",
                json!({}),
                &cookie,
                &csrf
            ))
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        app.clone()
            .oneshot(authenticated(
                "POST",
                "/api/v1/manage",
                json!({"method":"overview","payload":{}}),
                &cookie,
                &csrf
            ))
            .await
            .unwrap()
            .status(),
        401
    );
}
#[tokio::test]
async fn session_rotation_and_capacity() {
    let mut settings = config();
    settings.max_sessions = 1;
    let (app, _) = setup(settings);
    let (cookie, csrf) = login(&app).await;
    assert_eq!(
        app.clone()
            .oneshot(req("POST", "/api/v1/session", json!({"token":TOKEN})))
            .await
            .unwrap()
            .status(),
        429
    );
    let response = app
        .clone()
        .oneshot(authenticated(
            "POST",
            "/api/v1/session",
            json!({"token":TOKEN}),
            &cookie,
            &csrf,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        app.clone()
            .oneshot(authenticated(
                "GET",
                "/api/v1/session",
                json!({}),
                &cookie,
                &csrf
            ))
            .await
            .unwrap()
            .status(),
        401
    );
}
#[tokio::test]
async fn login_rate_body_limits_safe_errors_and_credentials() {
    let (app, _) = setup(config());
    let mut missing = req("POST", "/api/v1/session", json!({"token":TOKEN}));
    missing.headers_mut().remove("content-type");
    assert_eq!(app.clone().oneshot(missing).await.unwrap().status(), 415);
    assert_eq!(
        app.clone()
            .oneshot(req(
                "POST",
                "/api/v1/session",
                json!({"token":"x".repeat(2000)})
            ))
            .await
            .unwrap()
            .status(),
        413
    );
    let (cookie, csrf) = login(&app).await;
    for (method, status) in [("failure", 409), ("leak", 400)] {
        let response = app
            .clone()
            .oneshot(authenticated(
                "POST",
                "/api/v1/manage",
                json!({"method":method,"payload":{}}),
                &cookie,
                &csrf,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        let value = data(response).await.to_string();
        assert!(!value.contains(TOKEN));
        assert!(!value.contains("sensitive upstream"));
    }
    let mut settings = config();
    settings.max_body_bytes = 1024;
    let (limited, _) = setup(settings);
    let (cookie, csrf) = login(&limited).await;
    assert_eq!(
        limited
            .oneshot(authenticated(
                "POST",
                "/api/v1/manage",
                json!({"method":"overview","payload":{"large":"x".repeat(2048)}}),
                &cookie,
                &csrf
            ))
            .await
            .unwrap()
            .status(),
        413
    );
    let (rate, _) = setup(config());
    for _ in 0..10 {
        assert_eq!(
            rate.clone()
                .oneshot(req("POST", "/api/v1/session", json!({"token":"wrong"})))
                .await
                .unwrap()
                .status(),
            401
        );
    }
    assert_eq!(
        rate.oneshot(req("POST", "/api/v1/session", json!({"token":TOKEN})))
            .await
            .unwrap()
            .status(),
        429
    );
}
#[tokio::test]
async fn request_timeout_and_concurrency_are_bounded() {
    let mut settings = config();
    settings.max_concurrent_requests = 1;
    settings.request_timeout_ms = 100;
    let (app, backend) = setup(settings);
    let (cookie, csrf) = login(&app).await;
    let first = tokio::spawn(app.clone().oneshot(authenticated(
        "POST",
        "/api/v1/manage",
        json!({"method":"slow","payload":{}}),
        &cookie,
        &csrf,
    )));
    for _ in 0..100 {
        if !backend.calls.lock().unwrap().is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(!backend.calls.lock().unwrap().is_empty());
    assert_eq!(
        app.clone()
            .oneshot(authenticated(
                "GET",
                "/api/v1/session",
                json!({}),
                &cookie,
                &csrf
            ))
            .await
            .unwrap()
            .status(),
        429
    );
    assert_eq!(first.await.unwrap().unwrap().status(), 504);
    assert_eq!(
        app.oneshot(authenticated(
            "GET",
            "/api/v1/session",
            json!({}),
            &cookie,
            &csrf
        ))
        .await
        .unwrap()
        .status(),
        200
    );
}
#[tokio::test]
async fn assets_and_exact_host_policy() {
    let (app, _) = setup(config());
    let response = app
        .clone()
        .oneshot(req("GET", "/", json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
    assert_eq!(
        app.clone()
            .oneshot(req("GET", "/api/unknown", json!({})))
            .await
            .unwrap()
            .status(),
        404
    );
    let mut request = req("GET", "/", json!({}));
    request
        .headers_mut()
        .insert("host", "localhost:17280".parse().unwrap());
    request
        .headers_mut()
        .insert("x-forwarded-host", "127.0.0.1:17280".parse().unwrap());
    assert_eq!(app.oneshot(request).await.unwrap().status(), 403);
}
#[tokio::test]
async fn real_listener_exclusive_bind_stop_and_restart() {
    let backend = Arc::new(Backend {
        calls: Mutex::new(vec![]),
    });
    let server = Server::start(config(), backend.clone()).await.unwrap();
    let address = server.address();
    let connection = tokio::net::TcpStream::connect(address).await.unwrap();
    let mut same = config();
    same.bind = address;
    assert!(Server::start(same.clone(), backend.clone()).await.is_err());
    tokio::time::timeout(Duration::from_secs(2), server.stop())
        .await
        .unwrap();
    drop(connection);
    let next = Server::start(same, backend).await.unwrap();
    assert_eq!(next.address(), address);
    next.stop().await;
}
#[test]
fn config_rejects_unsafe_credentials_and_binding_without_echo() {
    for value in [
        json!({"admin_token":TOKEN,"bind":"0.0.0.0:17280"}),
        json!({"admin_token":"x".repeat(64)}),
        json!({"admin_token":"short"}),
        json!({"admin_token":TOKEN,"max_connections":10000}),
        json!({"admin_token":TOKEN,"unknown":true}),
    ] {
        let error = match Config::from_value(value) {
            Err(error) => error,
            Ok(_) => panic!("unsafe config accepted"),
        };
        assert_eq!(error.code, ErrorCode::InvalidConfig);
        assert!(!error.message.contains(TOKEN));
    }
}
#[test]
fn generated_openapi_has_no_drift() {
    let expected = serde_json::to_string_pretty(&web_admin::openapi()).unwrap() + "\n";
    assert_eq!(
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/openapi.json")).unwrap(),
        expected
    );
}

async fn raw(
    address: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: &str,
    auth: Option<(&str, &str)>,
) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let credentials = auth
        .map(|(cookie, csrf)| format!("Cookie: {cookie}\r\nX-CSRF-Token: {csrf}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{credentials}\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.unwrap();
    String::from_utf8(bytes).unwrap()
}
#[tokio::test]
async fn logout_during_body_upload_prevents_backend_admission() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let backend = Arc::new(Backend {
        calls: Mutex::new(vec![]),
    });
    let server = Server::start(config(), backend.clone()).await.unwrap();
    let address = server.address();
    let response = raw(
        address,
        "POST",
        "/api/v1/session",
        &json!({"token":TOKEN}).to_string(),
        None,
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 200"));
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    let cookie = headers
        .lines()
        .find_map(|line| line.strip_prefix("set-cookie: "))
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let value: Value = serde_json::from_str(body).unwrap();
    let csrf = value["csrf_token"].as_str().unwrap();
    let payload = json!({"method":"overview","payload":{}}).to_string();
    let mut slow = tokio::net::TcpStream::connect(address).await.unwrap();
    let headers = format!(
        "POST /api/v1/manage HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nContent-Type: application/json\r\nCookie: {cookie}\r\nX-CSRF-Token: {csrf}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    slow.write_all(headers.as_bytes()).await.unwrap();
    slow.write_all(&payload.as_bytes()[..1]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let response = raw(
        address,
        "DELETE",
        "/api/v1/session",
        "",
        Some((cookie, csrf)),
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 204"));
    slow.write_all(&payload.as_bytes()[1..]).await.unwrap();
    let mut response = String::new();
    slow.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 401"), "{response}");
    assert!(backend.calls.lock().unwrap().is_empty());
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn session_expiry_is_fixed_and_rate_window_recovers() {
    let mut settings = config();
    settings.session_ttl_secs = 60;
    let (app, _) = setup(settings);
    let (cookie, csrf) = login(&app).await;
    tokio::time::advance(Duration::from_secs(59)).await;
    assert_eq!(
        app.clone()
            .oneshot(authenticated(
                "GET",
                "/api/v1/session",
                json!({}),
                &cookie,
                &csrf
            ))
            .await
            .unwrap()
            .status(),
        200
    );
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(
        app.clone()
            .oneshot(authenticated(
                "GET",
                "/api/v1/session",
                json!({}),
                &cookie,
                &csrf
            ))
            .await
            .unwrap()
            .status(),
        401
    );
    for _ in 0..10 {
        assert_eq!(
            app.clone()
                .oneshot(req("POST", "/api/v1/session", json!({"token":"wrong"})))
                .await
                .unwrap()
                .status(),
            401
        );
    }
    assert_eq!(
        app.clone()
            .oneshot(req("POST", "/api/v1/session", json!({"token":TOKEN})))
            .await
            .unwrap()
            .status(),
        429
    );
    tokio::time::advance(Duration::from_secs(61)).await;
    assert_eq!(
        app.oneshot(req("POST", "/api/v1/session", json!({"token":TOKEN})))
            .await
            .unwrap()
            .status(),
        200
    );
}
