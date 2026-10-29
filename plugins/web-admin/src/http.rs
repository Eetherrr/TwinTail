//! HTTP authentication and management contract. No direct management socket or filesystem access.
use crate::Config;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::{sync::Semaphore, time::Instant};
use twintail_plugin_sdk::{ErrorCode, Result, RpcError};

include!(concat!(env!("OUT_DIR"), "/assets.rs"));
/// Host management transport; test implementations can exercise HTTP without SDK pipes.
/// Implementations must return safe, unresolved configuration references and bounded results.
pub trait Management: Send + Sync + 'static {
    /// Forwards one explicitly allowed management call. Never log its payload.
    fn call(
        &self,
        method: String,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + '_>>;
}
struct Session {
    csrf: [u8; 32],
    expires: Instant,
}
struct Rate {
    start: Instant,
    count: u32,
}
impl Rate {
    fn allow(&mut self, limit: u32) -> bool {
        if self.start.elapsed() >= Duration::from_secs(60) {
            self.start = Instant::now();
            self.count = 0;
        }
        if self.count >= limit {
            return false;
        }
        self.count += 1;
        true
    }
}
struct Auth {
    sessions: HashMap<[u8; 32], Session>,
    login: Rate,
    logout: Rate,
}
struct App {
    config: Config,
    token_hash: [u8; 32],
    authority: String,
    origin: String,
    auth: Mutex<Auth>,
    budget: Semaphore,
    backend: Arc<dyn Management>,
}
/// Builds the HTTP router for the actual bound address (including an assigned test port).
/// All origins/Host values are exact; localhost aliases and forwarded headers confer no trust.
pub fn router(
    config: Config,
    address: std::net::SocketAddr,
    backend: Arc<dyn Management>,
) -> Result<Router> {
    config.validate()?;
    if !address.ip().is_loopback() {
        return Err(RpcError::new(
            ErrorCode::InvalidConfig,
            "nonloopback HTTP address",
        ));
    }
    let now = Instant::now();
    let hash = Sha256::digest(config.admin_token.as_bytes()).into();
    let authority = address.to_string();
    let state = Arc::new(App {
        token_hash: hash,
        origin: format!("http://{authority}"),
        authority,
        auth: Mutex::new(Auth {
            sessions: HashMap::new(),
            login: Rate {
                start: now,
                count: 0,
            },
            logout: Rate {
                start: now,
                count: 0,
            },
        }),
        budget: Semaphore::new(config.max_concurrent_requests),
        config,
        backend,
    });
    Ok(Router::new()
        .route("/api/v1/session", post(login).get(session).delete(logout))
        .route("/api/v1/manage", post(manage))
        .fallback(get(asset))
        .layer(middleware::from_fn_with_state(state.clone(), boundary))
        .with_state(state))
}
fn fail(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        axum::Json(json!({"error":{"code":code,"message":message}})),
    )
        .into_response()
}
fn json(value: Value) -> Response {
    axum::Json(value).into_response()
}
fn unauthorized() -> Response {
    fail(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "session or credentials invalid",
    )
}
fn forbidden() -> Response {
    fail(
        StatusCode::FORBIDDEN,
        "permission_denied",
        "request origin, host or CSRF check failed",
    )
}
fn busy() -> Response {
    let mut response = fail(
        StatusCode::TOO_MANY_REQUESTS,
        "overloaded",
        "request capacity or rate limit exceeded",
    );
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
    response
}
fn single<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut all = headers.get_all(name).iter();
    let value = all.next()?.to_str().ok()?;
    if all.next().is_some() {
        return None;
    }
    Some(value)
}
fn valid_origin(app: &App, headers: &HeaderMap, required: bool) -> bool {
    if headers.contains_key(header::ORIGIN) {
        single(headers, "origin") == Some(app.origin.as_str())
    } else {
        !required
    }
}
async fn boundary(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let response = if single(request.headers(), "host") != Some(app.authority.as_str())
        || !valid_origin(
            &app,
            request.headers(),
            request.method() != Method::GET && request.method() != Method::HEAD,
        )
        || single(request.headers(), "sec-fetch-site")
            .is_some_and(|s| s != "same-origin" && s != "none")
        || request.uri().query().is_some()
    {
        forbidden()
    } else if let Ok(_permit) = app.budget.try_acquire() {
        match tokio::time::timeout(
            Duration::from_millis(app.config.request_timeout_ms),
            next.run(request),
        )
        .await
        {
            Ok(response) => response,
            Err(_) => fail(
                StatusCode::GATEWAY_TIMEOUT,
                "timeout",
                "HTTP request deadline elapsed; query operation status before retrying",
            ),
        }
    } else {
        busy()
    };
    let mut response = response;
    let headers = response.headers_mut();
    for (key, value) in [
        (
            "content-security-policy",
            "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cache-control", "no-store"),
        ("x-frame-options", "DENY"),
        ("cross-origin-resource-policy", "same-origin"),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=()",
        ),
    ] {
        headers.insert(key, HeaderValue::from_static(value));
    }
    response
}
async fn body<T: serde::de::DeserializeOwned>(
    request: Request,
    limit: usize,
) -> std::result::Result<T, Box<Response>> {
    if single(request.headers(), "content-type")
        .is_none_or(|v| v.split(';').next().map(str::trim) != Some("application/json"))
    {
        return Err(Box::new(fail(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "invalid_request",
            "application/json required",
        )));
    }
    let bytes = to_bytes(request.into_body(), limit).await.map_err(|_| {
        Box::new(fail(
            StatusCode::PAYLOAD_TOO_LARGE,
            "invalid_request",
            "request body exceeds limit",
        ))
    })?;
    serde_json::from_slice(&bytes).map_err(|_| {
        Box::new(fail(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "invalid JSON request",
        ))
    })
}
fn random() -> Result<[u8; 32]> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes)
        .map_err(|_| RpcError::new(ErrorCode::RemoteError, "credential generation unavailable"))?;
    Ok(bytes)
}
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(value, "{byte:02x}").unwrap();
    }
    value
}
fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut bytes = [0; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}
fn cookie(headers: &HeaderMap) -> Option<[u8; 32]> {
    let mut found = None;
    for header in headers.get_all(header::COOKIE) {
        for part in header.to_str().ok()?.split(';') {
            if let Some(value) = part.trim().strip_prefix("twintail_session=") {
                if found.is_some() {
                    return None;
                }
                found = Some(unhex(value)?);
            }
        }
    }
    found
}
fn authenticate(app: &App, headers: &HeaderMap, csrf: bool) -> Option<([u8; 32], String, u64)> {
    let id = cookie(headers)?;
    let mut auth = app.auth.lock().unwrap();
    let now = Instant::now();
    auth.sessions.retain(|_, s| s.expires > now);
    let session = auth.sessions.get(&id)?;
    if csrf {
        let supplied = unhex(single(headers, "x-csrf-token")?)?;
        if !bool::from(session.csrf.ct_eq(&supplied)) {
            return None;
        }
    }
    Some((
        id,
        hex(&session.csrf),
        session.expires.saturating_duration_since(now).as_secs(),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Login {
    token: String,
}
async fn login(State(app): State<Arc<App>>, request: Request) -> Response {
    if !app.auth.lock().unwrap().login.allow(10) {
        return busy();
    }
    let old_cookie = cookie(request.headers());
    let login: Login = match body(request, 1024.min(app.config.max_body_bytes)).await {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let candidate: [u8; 32] = Sha256::digest(login.token.as_bytes()).into();
    if !bool::from(app.token_hash.ct_eq(&candidate)) {
        return unauthorized();
    }
    let (id, csrf) = match (random(), random()) {
        (Ok(id), Ok(csrf)) => (id, csrf),
        _ => {
            return fail(
                StatusCode::SERVICE_UNAVAILABLE,
                "remote_error",
                "session creation unavailable",
            );
        }
    };
    let now = Instant::now();
    let mut auth = app.auth.lock().unwrap();
    auth.sessions.retain(|_, s| s.expires > now);
    if let Some(old) = old_cookie {
        auth.sessions.remove(&old);
    }
    if auth.sessions.len() >= app.config.max_sessions {
        return busy();
    }
    auth.sessions.insert(
        id,
        Session {
            csrf,
            expires: now + Duration::from_secs(app.config.session_ttl_secs),
        },
    );
    let mut response =
        json(json!({"csrf_token":hex(&csrf),"expires_in_secs":app.config.session_ttl_secs}));
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "twintail_session={}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
            hex(&id),
            app.config.session_ttl_secs
        ))
        .unwrap(),
    );
    response
}
async fn session(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    match authenticate(&app, &headers, false) {
        Some((_, csrf, ttl)) => json(json!({"csrf_token":csrf,"expires_in_secs":ttl})),
        None => unauthorized(),
    }
}
async fn logout(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if !app.auth.lock().unwrap().logout.allow(30) {
        return busy();
    }
    if authenticate(&app, &headers, false).is_none() {
        return unauthorized();
    }
    let Some((id, _, _)) = authenticate(&app, &headers, true) else {
        return forbidden();
    };
    app.auth.lock().unwrap().sessions.remove(&id);
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("twintail_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"),
    );
    response
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manage {
    method: String,
    payload: Value,
}
async fn manage(State(app): State<Arc<App>>, request: Request) -> Response {
    if authenticate(&app, request.headers(), false).is_none() {
        return unauthorized();
    }
    if authenticate(&app, request.headers(), true).is_none() {
        return forbidden();
    }
    let headers = request.headers().clone();
    let call: Manage = match body(request, app.config.max_body_bytes).await {
        Ok(value) => value,
        Err(response) => return *response,
    };
    if call.method.len() > 128 || !call.payload.is_object() {
        return fail(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "invalid management call",
        );
    }
    // Body reads yield: expiry or logout during upload must prevent admission.
    if authenticate(&app, &headers, false).is_none() {
        return unauthorized();
    }
    if authenticate(&app, &headers, true).is_none() {
        return forbidden();
    }
    match app.backend.call(call.method, call.payload).await {
        Ok(result) => {
            if contains_token(&result, &app.config.admin_token) {
                return fail(
                    StatusCode::BAD_REQUEST,
                    "invalid_config",
                    "credential literals must be migrated to secret references using the CLI",
                );
            }
            json(json!({"result":result}))
        }
        Err(error) => {
            let status = match error.code {
                ErrorCode::PermissionDenied => StatusCode::FORBIDDEN,
                ErrorCode::InvalidConfig | ErrorCode::InvalidRequest | ErrorCode::InvalidFrame => {
                    StatusCode::BAD_REQUEST
                }
                ErrorCode::NotFound => StatusCode::NOT_FOUND,
                ErrorCode::Conflict => StatusCode::CONFLICT,
                ErrorCode::Overloaded => StatusCode::TOO_MANY_REQUESTS,
                ErrorCode::Timeout | ErrorCode::UnknownOutcome => StatusCode::GATEWAY_TIMEOUT,
                ErrorCode::Unsupported => StatusCode::NOT_IMPLEMENTED,
                _ => StatusCode::BAD_GATEWAY,
            };
            let code = serde_json::to_value(error.code).unwrap();
            (status,axum::Json(json!({"error":{"code":code,"message":"management call failed; inspect safe operation status before retrying"}}))).into_response()
        }
    }
}
async fn asset(request: Request) -> Response {
    let path = if request.uri().path() == "/" {
        "/index.html"
    } else {
        request.uri().path()
    };
    if path.starts_with("/api/") {
        return fail(StatusCode::NOT_FOUND, "not_found", "endpoint not found");
    }
    let Some((_, bytes)) = ASSETS.iter().find(|(name, _)| *name == path) else {
        return fail(StatusCode::NOT_FOUND, "not_found", "asset not found");
    };
    let mime = if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    };
    let mut response = Response::new(Body::from(*bytes));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    response
}

fn contains_token(value: &Value, token: &str) -> bool {
    match value {
        Value::String(value) => value.contains(token),
        Value::Array(values) => values.iter().any(|value| contains_token(value, token)),
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| key.contains(token) || contains_token(value, token)),
        _ => false,
    }
}
