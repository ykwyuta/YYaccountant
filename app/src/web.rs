//! Password-authenticated Web UI. The browser calls the same tools as MCP; Rust owns
//! parsing, decimal arithmetic, validation and persistence.
use crate::{auth, mcp};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use yy_accountant::Store;

struct Session {
    name: String,
    fingerprint: String,
    csrf: String,
    seen: Instant,
    created: Instant,
}
#[derive(Default)]
struct Sessions {
    clients: BTreeMap<String, Session>,
    failed: Vec<Instant>,
}
#[derive(Clone)]
struct Service {
    store: Arc<Mutex<Store>>,
    sessions: Arc<Mutex<Sessions>>,
    config: PathBuf,
    hosts: BTreeSet<String>,
    origins: BTreeSet<String>,
    capacity: Arc<tokio::sync::Semaphore>,
    secure_cookies: bool,
    read_only: bool,
}

fn reply(status: StatusCode, value: Value) -> Response {
    (status, Json(value)).into_response()
}
fn failure(status: StatusCode, message: &str) -> Response {
    reply(status, json!({"error":message}))
}
fn secure(mut r: Response) -> Response {
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "same-origin"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
        ),
    ] {
        r.headers_mut().insert(
            header::HeaderName::from_static(name),
            header::HeaderValue::from_static(value),
        );
    }
    r
}
const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="#1f6f5c"/><path d="M14 46h36M20 40V28m12 12V18m12 22V24" stroke="#fff" stroke-width="6" stroke-linecap="round"/></svg>"##;

async fn endpoint(State(s): State<Service>, r: Request) -> Response {
    let (parts, body) = r.into_parts();
    let host = parts
        .headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if !s.hosts.contains(host) || parts.headers.get_all(header::HOST).iter().count() != 1 {
        return secure(failure(StatusCode::FORBIDDEN, "Host not allowed"));
    }
    let path = parts.uri.path().to_owned();
    if parts.method == Method::GET {
        let (content, mime) = match path.as_str() {
            "/" => (include_str!("web/index.html"), "text/html; charset=utf-8"),
            "/web/app.js" => (include_str!("web/app.js"), "text/javascript; charset=utf-8"),
            "/web/app.css" => (include_str!("web/app.css"), "text/css; charset=utf-8"),
            "/favicon.svg" => (ICON, "image/svg+xml"),
            "/web/session" => ("", ""),
            _ => return secure(failure(StatusCode::NOT_FOUND, "Not found")),
        };
        if !content.is_empty() {
            return secure(([(header::CONTENT_TYPE, mime)], content).into_response());
        }
    } else if parts.method != Method::POST {
        return secure(failure(StatusCode::METHOD_NOT_ALLOWED, "Use POST"));
    }
    if parts.method == Method::POST {
        let origin = parts
            .headers
            .get(header::ORIGIN)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        if !s.origins.contains(origin) || parts.headers.get_all(header::ORIGIN).iter().count() != 1
        {
            return secure(failure(
                StatusCode::FORBIDDEN,
                "Same-origin request required",
            ));
        }
        if parts
            .headers
            .get(header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            != "application/json"
        {
            return secure(failure(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Use application/json",
            ));
        }
    }
    let Ok(permit) = s.capacity.clone().try_acquire_owned() else {
        return secure(failure(StatusCode::SERVICE_UNAVAILABLE, "Busy"));
    };
    let bytes = match tokio::time::timeout(Duration::from_secs(15), to_bytes(body, 2 * 1024 * 1024))
        .await
    {
        Ok(Ok(b)) => b.to_vec(),
        _ => {
            return secure(failure(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Invalid or oversized body",
            ));
        }
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        process(&s, &path, &parts.headers, &bytes)
    })
    .await
    {
        Ok(r) => secure(r),
        Err(_) => secure(failure(StatusCode::INTERNAL_SERVER_ERROR, "Request failed")),
    }
}
fn cookie(s: &Service, id: &str) -> String {
    format!(
        "yya_session={id}; Path=/web; HttpOnly; SameSite=Strict; Max-Age={}{}",
        if id.is_empty() { 0 } else { 43200 },
        if s.secure_cookies { "; Secure" } else { "" }
    )
}
fn process(s: &Service, path: &str, headers: &HeaderMap, bytes: &[u8]) -> Response {
    let Ok(config) = auth::Config::load(&s.config) else {
        return failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "Authentication configuration unavailable",
        );
    };
    let value = if path == "/web/session" {
        json!({})
    } else {
        match serde_json::from_slice::<Value>(bytes) {
            Ok(v) if v.is_object() => v,
            _ => return failure(StatusCode::BAD_REQUEST, "Expected JSON object"),
        }
    };
    let Ok(mut sessions) = s.sessions.lock() else {
        return failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Session worker unavailable",
        );
    };
    sessions.clients.retain(|_, c| {
        c.seen.elapsed() < Duration::from_secs(1800)
            && c.created.elapsed() < Duration::from_secs(43200)
    });
    if path == "/web/login" {
        sessions
            .failed
            .retain(|at| at.elapsed() < Duration::from_secs(60));
        if sessions.failed.len() >= 30 {
            return failure(
                StatusCode::TOO_MANY_REQUESTS,
                "しばらく待って再試行してください",
            );
        }
        let Some(user) = config.authenticate(value["password"].as_str().unwrap_or("")) else {
            sessions.failed.push(Instant::now());
            return failure(StatusCode::UNAUTHORIZED, "パスワードを確認してください");
        };
        if sessions.clients.len() >= 256 {
            return failure(StatusCode::SERVICE_UNAVAILABLE, "Session limit reached");
        }
        let (Ok(id), Ok(csrf)) = (auth::random_id(), auth::random_id()) else {
            return failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Random generator unavailable",
            );
        };
        let mut r = reply(
            StatusCode::OK,
            json!({"user":user.name,"read_only":user.read_only||s.read_only,"csrf":csrf}),
        );
        sessions.clients.insert(
            id.clone(),
            Session {
                name: user.name,
                fingerprint: user.password_sha256,
                csrf,
                seen: Instant::now(),
                created: Instant::now(),
            },
        );
        r.headers_mut()
            .insert(header::SET_COOKIE, cookie(s, &id).parse().unwrap());
        return r;
    }
    let ids: Vec<&str> = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|h| h.split(';'))
        .filter_map(|h| h.trim().strip_prefix("yya_session="))
        .collect();
    if ids.len() != 1 {
        return failure(StatusCode::UNAUTHORIZED, "ログインしてください");
    }
    let id = ids[0];
    let Some(session) = sessions.clients.get_mut(id) else {
        return failure(StatusCode::UNAUTHORIZED, "セッションが終了しました");
    };
    let Some(user) = config
        .web_users
        .iter()
        .find(|u| u.name == session.name && u.password_sha256 == session.fingerprint)
    else {
        sessions.clients.remove(id);
        return failure(StatusCode::UNAUTHORIZED, "ログインし直してください");
    };
    if path == "/web/session" {
        return reply(
            StatusCode::OK,
            json!({"user":user.name,"read_only":user.read_only||s.read_only,"csrf":session.csrf}),
        );
    }
    if headers.get_all("x-yy-csrf").iter().count() != 1
        || headers.get("x-yy-csrf").and_then(|v| v.to_str().ok()) != Some(session.csrf.as_str())
    {
        return failure(StatusCode::FORBIDDEN, "Invalid CSRF token");
    }
    session.seen = Instant::now();
    if path == "/web/logout" {
        sessions.clients.remove(id);
        let mut r = reply(StatusCode::OK, json!({}));
        r.headers_mut()
            .insert(header::SET_COOKIE, cookie(s, "").parse().unwrap());
        return r;
    }
    let user = user.clone();
    drop(sessions);
    let Ok(mut store) = s.store.lock() else {
        return failure(StatusCode::INTERNAL_SERVER_ERROR, "Database unavailable");
    };
    if path == "/web/backup" {
        let backup = (|| -> Result<Vec<u8>, Box<dyn std::error::Error>> {
            let dir = tempfile::tempdir()?;
            let path = dir.path().join("backup.sqlite3");
            store.backup(&path)?;
            Ok(std::fs::read(path)?)
        })();
        return match backup {
            Ok(bytes) => (
                [
                    (header::CONTENT_TYPE, "application/vnd.sqlite3"),
                    (
                        header::CONTENT_DISPOSITION,
                        "attachment; filename=yyaccountant-backup.sqlite3",
                    ),
                ],
                bytes,
            )
                .into_response(),
            Err(e) => failure(StatusCode::BAD_REQUEST, &e.to_string()),
        };
    }
    if path != "/web/api" {
        return failure(StatusCode::NOT_FOUND, "Not found");
    }
    let tool = value["tool"].as_str().unwrap_or("");
    let mut arguments = value.get("arguments").cloned().unwrap_or(json!({}));
    if mcp::MUTATING.contains(&tool) {
        if user.read_only || s.read_only {
            return failure(StatusCode::FORBIDDEN, "読み取り専用ユーザーです");
        }
        if let Some(o) = arguments.as_object_mut() {
            o.insert("actor".into(), json!(user.name));
        }
    }
    if mcp::LEARNER.contains(&tool)
        && let Some(o) = arguments.as_object_mut()
    {
        // Progress always belongs to the signed-in user; read-only users practise without saving.
        o.insert("actor".into(), json!(user.name));
        if tool == "study_answer" && (user.read_only || s.read_only) {
            o.insert("record".into(), json!(false));
        }
    }
    match mcp::call(&mut store, tool, arguments) {
        Ok(v) => reply(StatusCode::OK, v),
        Err(e) => {
            let status = if matches!(
                e.downcast_ref::<yy_accountant::Error>(),
                Some(yy_accountant::Error::Conflict)
            ) {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            failure(status, &e.to_string())
        }
    }
}

pub fn serve(
    store: Store,
    listen: SocketAddr,
    config: PathBuf,
    read_only: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let auth = auth::Config::load(&config).map_err(|e| e.to_string())?;
    let socket = std::net::TcpListener::bind(listen)?;
    socket.set_nonblocking(true)?;
    let address = socket.local_addr()?;
    let mut hosts: BTreeSet<String> = auth.server.allowed_hosts.into_iter().collect();
    if address.ip().is_loopback() {
        hosts.extend([
            format!("127.0.0.1:{}", address.port()),
            format!("localhost:{}", address.port()),
            format!("[::1]:{}", address.port()),
        ]);
    } else if hosts.is_empty() {
        return Err("Non-loopback listen requires server.allowed_hosts in the auth config".into());
    }
    let origins = hosts
        .iter()
        .map(|h| format!("http://{h}"))
        .chain(auth.server.allowed_origins)
        .collect();
    let service = Service {
        store: Arc::new(Mutex::new(store)),
        sessions: Arc::new(Mutex::new(Sessions::default())),
        config,
        hosts,
        origins,
        capacity: Arc::new(tokio::sync::Semaphore::new(64)),
        secure_cookies: auth.server.secure_cookies,
        read_only,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::from_std(socket)?;
        eprintln!("YYaccountant Web UI: http://{address}/");
        let router = Router::new()
            .route("/", any(endpoint))
            .route("/favicon.svg", any(endpoint))
            .route("/web/{*path}", any(endpoint));
        axum::serve(listener, router.with_state(service))
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
    })?;
    Ok(())
}
