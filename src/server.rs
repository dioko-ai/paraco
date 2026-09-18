use crate::{control, logs, manifest, runner};
use futures_util::StreamExt;
mod management;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use tokio::sync::Semaphore;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    apps: Vec<AppConfig>,
    #[serde(default = "default_max_apps")]
    max_apps: usize,
}
fn default_max_apps() -> usize {
    50
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AppConfig {
    path: PathBuf,
    ai_config: Option<PathBuf>,
    #[serde(default)]
    restart: RestartPolicy,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RestartPolicy {
    #[serde(default)]
    on_failure: bool,
    #[serde(default = "default_retries")]
    max_retries: u32,
    #[serde(default = "default_backoff")]
    backoff_ms: u64,
    #[serde(default = "default_max_backoff")]
    max_backoff_ms: u64,
}
fn default_retries() -> u32 {
    3
}
fn default_backoff() -> u64 {
    1000
}
fn default_max_backoff() -> u64 {
    30000
}
impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            on_failure: false,
            max_retries: default_retries(),
            backoff_ms: default_backoff(),
            max_backoff_ms: default_max_backoff(),
        }
    }
}

struct PreparedApp {
    restart: RestartPolicy,
    app: manifest::App,
    ai_config: Option<PathBuf>,
}

#[derive(Clone)]
enum AppState {
    Starting,
    // The authorization value exists only for the current child launch.
    Running(u16, String),
    Failed(String),
    Backoff(String),
    Stopping,
    Stopped,
}

#[derive(Clone, Copy, PartialEq)]
enum DesiredState {
    Running,
    Stopped,
}

struct AppRecord {
    root: PathBuf,
    desired: DesiredState,
    state: AppState,
    generation: u64,
    cancel: Arc<AtomicBool>,
    admission: Arc<Semaphore>,
}

type Inventory = Arc<RwLock<BTreeMap<String, AppRecord>>>;

#[derive(Clone)]
struct Gateway {
    apps: Inventory,
    client: reqwest::Client,
    port: u16,
    admission: Arc<Semaphore>,
}

/// Supervisors own their app resources; all stop concurrently before joining.
struct Supervisors {
    stop: Arc<AtomicBool>,
    threads: Vec<thread::JoinHandle<()>>,
    apps: Inventory,
}
impl Drop for Supervisors {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for app in self.apps.read().unwrap().values() {
            app.cancel.store(true, Ordering::Relaxed);
        }
        for worker in self.threads.drain(..) {
            let _ = worker.join();
        }
    }
}

fn load(path: &Path) -> Result<Vec<PreparedApp>, String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("cannot read server configuration: {e}"))?;
    let bytes =
        std::fs::read(&path).map_err(|e| format!("cannot read server configuration: {e}"))?;
    let config: Config =
        serde_json::from_slice(&bytes).map_err(|e| format!("invalid server configuration: {e}"))?;
    if config.max_apps == 0 || config.max_apps > 50 || config.apps.len() > config.max_apps {
        return Err("maxApps must be 1–50 and cover the configured applications".into());
    }
    let root = path.parent().unwrap();
    let mut names = BTreeSet::new();
    config
        .apps
        .into_iter()
        .map(|entry| {
            let app = manifest::load(&root.join(entry.path)).map_err(|e| e.to_string())?;
            if !names.insert(app.name.clone()) {
                return Err(format!("duplicate application name `{}`", app.name));
            }
            if entry.restart.max_retries > 100 || entry.restart.backoff_ms == 0
                || entry.restart.max_backoff_ms < entry.restart.backoff_ms
                || entry.restart.max_backoff_ms > 300_000 {
                return Err("restart policy requires maxRetries <= 100 and 1 <= backoffMs <= maxBackoffMs <= 300000".into());
            }
            Ok(PreparedApp {
                restart: entry.restart,
                app,
                ai_config: entry.ai_config.map(|p| root.join(p)),
            })
        })
        .collect()
}

pub fn serve(config: &Path, port: u16, log_dir: &Path) -> Result<(), String> {
    if port == 0 {
        return Err("port must be from 1 through 65535".into());
    }
    let apps = load(config)?;
    let store = logs::Store::open(log_dir)?;
    for prepared in &apps {
        store.outside(&prepared.app.root)?;
    }
    runner::announce(true, format!("paraco: logs at {}", store.path().display()));
    let stop = runner::install_interrupt_handler()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let result = runtime.block_on(host(apps, port, stop, store));
    // spawn_blocking work is intentionally bounded but may be waiting on a
    // filesystem. Never let it turn process shutdown into an unbounded wait.
    runtime.shutdown_timeout(Duration::from_secs(2));
    result
}

async fn host(
    apps: Vec<PreparedApp>,
    port: u16,
    stop: Arc<AtomicBool>,
    store: logs::Store,
) -> Result<(), String> {
    // Bind the gateway before launching apps: an occupied public port starts none.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("cannot bind gateway: {e}"))?;
    let inventory: Inventory = Arc::new(RwLock::new(
        apps.iter()
            .map(|a| {
                (
                    a.app.name.clone(),
                    AppRecord {
                        root: a.app.root.clone(),
                        desired: DesiredState::Running,
                        state: AppState::Starting,
                        generation: 0,
                        cancel: Arc::new(AtomicBool::new(false)),
                        admission: Arc::new(Semaphore::new(8)),
                    },
                )
            })
            .collect(),
    ));
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let mut supervisors = Supervisors {
        stop: stop.clone(),
        threads: vec![],
        apps: inventory.clone(),
    };
    // Bind management before starting children; a conflicting endpoint starts none.
    let control_apps = inventory.clone();
    let control = control::Server::start(port, move |command| manage(&control_apps, command))?;
    let (management_listener, management_router, management_url) =
        management::bind(inventory.clone(), port, store.clone()).await?;
    for prepared in apps {
        let states = inventory.clone();
        let app_stop = stop.clone();
        let app_store = store.clone();
        let lease = control.lease();
        supervisors.threads.push(thread::spawn(move || {
            supervise(prepared, states, app_stop, app_store, port, lease);
        }));
    }
    let gateway = Gateway {
        apps: inventory,
        client,
        port,
        admission: Arc::new(Semaphore::new(64)),
    };
    let app = Router::new().fallback(dispatch).with_state(gateway);
    let mut serving = tokio::spawn(async move { axum::serve(listener, app).await });
    let mut managing =
        tokio::spawn(async move { axum::serve(management_listener, management_router).await });
    runner::announce(
        false,
        format!("paraco: management dashboard {management_url}"),
    );
    runner::announce(
        false,
        format!("paraco: dashboard listening on http://127.0.0.1:{port}"),
    );
    let result = tokio::select! {
        result = &mut managing => result.map_err(|e| e.to_string())?.map_err(|e| e.to_string()),
        result = &mut serving => result.map_err(|e| e.to_string())?.map_err(|e| e.to_string()),
        _ = async { while !stop.load(Ordering::Relaxed) { tokio::time::sleep(Duration::from_millis(25)).await; } } => Ok(()),
    };
    // Stop accepting requests and cancel in-flight proxy calls on shutdown.
    serving.abort();
    managing.abort();
    // Dropping the owners reaps every child, including apps still starting.
    drop(supervisors);
    drop(control);
    result
}

async fn dispatch(State(gateway): State<Gateway>, request: Request) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let gateway_hosts = [
        format!("127.0.0.1:{}", gateway.port),
        format!("localhost:{}", gateway.port),
    ];
    let app_name = gateway
        .apps
        .read()
        .unwrap()
        .iter()
        .find(|(name, app)| host == app_host(name, &app.root, gateway.port))
        .map(|(name, _)| name.clone());
    if !gateway_hosts.contains(&host) && app_name.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            "Use an application or local gateway address",
        )
            .into_response();
    }
    let path = request.uri().path().to_owned();
    if let Some(name) = app_name {
        let suffix = path.trim_start_matches('/').to_owned();
        return proxy_app(&gateway, request, &name, &suffix).await;
    }
    if path == "/" {
        if request.method() != Method::GET && request.method() != Method::HEAD {
            return (
                StatusCode::METHOD_NOT_ALLOWED,
                [(header::ALLOW, "GET, HEAD")],
                "Read-only dashboard",
            )
                .into_response();
        }
        let mut response = dashboard(&gateway.apps, gateway.port).into_response();
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response.headers_mut().insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'"));
        if request.method() == Method::HEAD {
            *response.body_mut() = Body::empty();
        }
        return response;
    }
    let Some(tail) = path.strip_prefix("/apps/") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (name, suffix) = tail
        .split_once('/')
        .map(|(n, p)| (n, Some(p)))
        .unwrap_or((tail, None));
    let app = gateway
        .apps
        .read()
        .unwrap()
        .get(name)
        .map(|app| app.root.clone());
    let Some(root) = app else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // The shared host is a migration-only surface. It cannot proxy app content.
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let suffix = suffix.unwrap_or("");
    let query = request
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    (
        StatusCode::PERMANENT_REDIRECT,
        [(
            header::LOCATION,
            format!(
                "http://{}/{}{query}",
                app_host(name, &root, gateway.port),
                suffix
            ),
        )],
    )
        .into_response()
}

async fn proxy_app(gateway: &Gateway, request: Request, name: &str, suffix: &str) -> Response {
    let root = match gateway.apps.read().unwrap().get(name) {
        Some(app) => app.root.clone(),
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let origin = format!("http://{}", app_host(name, &root, gateway.port));
    // A hosted browser request must be same-origin. We deliberately do not emit
    // CORS headers: this is a browser boundary, not local-process authentication.
    if let Some(value) = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        && value != origin
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    // Fetch Metadata covers requests (notably navigations and no-cors loads)
    // that do not carry Origin.  `none` is a user-initiated navigation; absent
    // metadata remains available to ordinary loopback HTTP clients, which are
    // explicitly outside this browser-only boundary.
    if let Some(site) = request
        .headers()
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        && site != "same-origin"
        && site != "none"
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let state = gateway
        .apps
        .read()
        .unwrap()
        .get(name)
        .map(|app| (app.state.clone(), app.admission.clone()));
    let Some((state, app_admission)) = state else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (port, token) = match state {
        AppState::Running(port, token) => (port, token),
        AppState::Starting => {
            return (StatusCode::SERVICE_UNAVAILABLE, "Application is starting").into_response();
        }
        AppState::Stopping | AppState::Stopped => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Application is stopped or stopping",
            )
                .into_response();
        }
        AppState::Failed(_) | AppState::Backoff(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Application failed; see the dashboard",
            )
                .into_response();
        }
    };
    // Acquire both bounded permits before reading the body; permits remain held
    // for streaming completion/cancellation and are never queued.
    let Ok(_global_permit) = gateway.admission.clone().try_acquire_owned() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Gateway is busy; retry shortly",
        )
            .into_response();
    };
    let Ok(_app_permit) = app_admission.try_acquire_owned() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Application is busy; retry shortly",
        )
            .into_response();
    };
    if request.headers().contains_key(header::UPGRADE) {
        return (
            StatusCode::NOT_IMPLEMENTED,
            "Protocol upgrades are not supported yet",
        )
            .into_response();
    }
    let query = request
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let target = format!("http://127.0.0.1:{port}/{suffix}{query}");
    let (mut parts, body) = request.into_parts();
    strip_hop_headers(&mut parts.headers);
    // Do not let client-supplied forwarding metadata masquerade as host context.
    for name in [
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-forwarded-prefix",
        "content-length",
    ] {
        parts.headers.remove(name);
    }
    parts.headers.insert(
        header::HOST,
        HeaderValue::from_str(&app_host(name, &root, gateway.port)).unwrap(),
    );
    parts
        .headers
        .insert("x-paraco-internal", HeaderValue::from_str(&token).unwrap());
    let body =
        match tokio::time::timeout(Duration::from_secs(10), to_bytes(body, 1024 * 1024)).await {
            Ok(Ok(body)) => body,
            Ok(Err(_)) => {
                return (
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "Request body exceeds 1 MiB or is invalid",
                )
                    .into_response();
            }
            Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
        };
    let upstream = match gateway
        .client
        .request(parts.method, target)
        .headers(parts.headers)
        .body(body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return if error.is_timeout() {
                StatusCode::GATEWAY_TIMEOUT
            } else {
                StatusCode::BAD_GATEWAY
            }
            .into_response();
        }
    };
    let status = upstream.status();
    let mut headers = upstream.headers().clone();
    strip_hop_headers(&mut headers);
    // Apps may set host-only cookies, but cannot opt into the shared localhost
    // parent domain through proxied response headers.
    let cookies: Vec<_> = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter(|value| {
            // Cookie attributes are case-insensitive and optional whitespace
            // around `=` is valid, so a substring search is insufficient.
            !value
                .as_bytes()
                .split(|byte| *byte == b';')
                .skip(1)
                .any(|part| {
                    let part = String::from_utf8_lossy(part);
                    part.split_once('=')
                        .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case("domain"))
                })
        })
        .cloned()
        .collect();
    headers.remove(header::SET_COOKIE);
    for cookie in cookies {
        headers.append(header::SET_COOKIE, cookie);
    }
    // Keep admission permits in the response stream: they release only after
    // upstream completion, client cancellation, or a streaming error.
    let guarded_stream = futures_util::stream::unfold(
        (upstream.bytes_stream(), _global_permit, _app_permit),
        |(mut stream, global_permit, app_permit)| async move {
            stream
                .next()
                .await
                .map(|item| (item, (stream, global_permit, app_permit)))
        },
    );
    let mut response = Response::new(Body::from_stream(guarded_stream));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

fn app_host(name: &str, root: &Path, port: u16) -> String {
    // FNV-1a gives a single DNS-safe, stable label and avoids name normalization
    // collisions until durable deployment IDs are introduced.
    let mut hash = 0xcbf29ce484222325u64;
    for byte in format!("{}:{name}:{port}", root.display()).bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    format!("app-{hash:016x}.localhost:{port}")
}

fn strip_hop_headers(headers: &mut HeaderMap) {
    let nominated: Vec<String> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|v| v.split(',').map(|s| s.trim().to_owned()))
        .collect();
    for name in nominated {
        headers.remove(name);
    }
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn dashboard(apps: &Inventory, port: u16) -> Html<String> {
    let apps = apps.read().unwrap();
    let rows: String = apps.iter().map(|(name, app)| {
        let (label, detail) = match &app.state {
            AppState::Starting => ("starting", "Starting application".to_string()),
            AppState::Running(_, _) => ("running", "Application has an isolated .localhost origin".into()),
            AppState::Failed(error) => ("failed", escape(error)),
            AppState::Backoff(error) => ("backoff", escape(error)),
            AppState::Stopping => ("stopping", "Stopping application".into()),
            AppState::Stopped => ("stopped", "Application is stopped".into()),
        };
        let url = format!("http://{}", app_host(name, &app.root, port));
        format!("<li class=\"app\"><div><h2>{name}</h2><a class=\"path\" href=\"{url}\">{url}</a></div><span class=\"status {label}\">{label}</span><div class=\"detail\">{detail}</div></li>")
    }).collect();
    let running = apps
        .values()
        .filter(|s| matches!(s.state, AppState::Running(_, _)))
        .count();
    let rows = if rows.is_empty() {
        "<li class=\"empty\">No apps configured yet. Add an app directory to your server configuration.</li>".into()
    } else {
        rows
    };
    Html(
        include_str!("../runtime/dashboard.html")
            .replace(
                "{{summary}}",
                &format!("{running} of {} apps running", apps.len()),
            )
            .replace("{{apps}}", &rows),
    )
}

// Each worker remains alive after failure or stop and owns at most one process.
// Commands invalidate an attempt before cancelling it. Stale attempts must clean
// up their resources before the worker can launch another generation.
fn supervise(
    prepared: PreparedApp,
    apps: Inventory,
    stop: Arc<AtomicBool>,
    store: logs::Store,
    port: u16,
    _lease: Option<Arc<std::fs::File>>,
) {
    let name = prepared.app.name;
    let root = prepared.app.root;
    let base = "/".to_string();
    let mut handled = None;
    let mut retry_generation = None;
    let mut retries = 0;
    while !stop.load(Ordering::Relaxed) {
        let (generation, desired, cancel) = {
            let records = apps.read().unwrap();
            let app = &records[&name];
            (app.generation, app.desired, app.cancel.clone())
        };
        if handled == Some(generation) {
            thread::sleep(Duration::from_millis(25));
            continue;
        }
        handled = Some(generation);
        if retry_generation != Some(generation) {
            retry_generation = Some(generation);
            retries = 0;
        }
        if desired == DesiredState::Stopped {
            publish(&apps, &name, generation, AppState::Stopped);
            continue;
        }
        publish(&apps, &name, generation, AppState::Starting);
        let log = match store.app(&name, "serve", port) {
            Ok(log) => log,
            Err(error) => {
                if recover(
                    &apps,
                    &name,
                    generation,
                    &prepared.restart,
                    &mut retries,
                    &cancel,
                    &stop,
                    error,
                ) {
                    handled = None;
                }
                continue;
            }
        };
        let backend_token = backend_token(&name, generation);
        let result = manifest::load(&root).map_err(|e| e.to_string()).and_then(|app| {
            if app.name != name {
                return Err("manifest name changed; restore it or restart the server with updated configuration".into());
            }
            runner::RunningApp::start(app, 0, prepared.ai_config.as_deref(), &base, Some(&backend_token), &cancel, log.clone())
        });
        let mut running = match result {
            Ok(running) => running,
            Err(error) => {
                log.event("launch_failed", &error);
                if recover(
                    &apps,
                    &name,
                    generation,
                    &prepared.restart,
                    &mut retries,
                    &cancel,
                    &stop,
                    error,
                ) {
                    handled = None;
                }
                continue;
            }
        };
        if !cancel.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
            publish(
                &apps,
                &name,
                generation,
                AppState::Running(running.port, backend_token),
            );
        }
        let mut failure = None;
        while !cancel.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
            if let Err(error) = running.check() {
                failure = Some(error);
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        drop(running);
        if let Some(error) = failure
            && recover(
                &apps,
                &name,
                generation,
                &prepared.restart,
                &mut retries,
                &cancel,
                &stop,
                error,
            )
        {
            handled = None;
        }
    }
}

// Waiting happens only on this app's worker, after its old resources are reaped.
#[allow(clippy::too_many_arguments)]
fn backend_token(name: &str, generation: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{name}-{generation}-{now:032x}")
}

#[allow(clippy::too_many_arguments)] // Worker state is intentionally passed explicitly.
fn recover(
    apps: &Inventory,
    name: &str,
    generation: u64,
    policy: &RestartPolicy,
    retries: &mut u32,
    cancel: &AtomicBool,
    stop: &AtomicBool,
    error: String,
) -> bool {
    if cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
        return false;
    }
    if !policy.on_failure || *retries >= policy.max_retries {
        let detail = if policy.on_failure {
            format!(
                "{error}; retry limit reached ({retries}/{})",
                policy.max_retries
            )
        } else {
            error
        };
        publish(apps, name, generation, AppState::Failed(detail));
        return false;
    }
    let delay = policy
        .backoff_ms
        .saturating_mul(1u64 << (*retries).min(63))
        .min(policy.max_backoff_ms);
    *retries += 1;
    publish(
        apps,
        name,
        generation,
        AppState::Backoff(format!(
            "{error}; retry {retries}/{} in {delay} ms",
            policy.max_retries
        )),
    );
    let deadline = std::time::Instant::now() + Duration::from_millis(delay);
    while std::time::Instant::now() < deadline {
        if cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed) {
            return false;
        }
        thread::sleep(Duration::from_millis(25));
    }
    !cancel.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed)
}

fn publish(apps: &Inventory, name: &str, generation: u64, state: AppState) {
    let mut records = apps.write().unwrap();
    let app = records.get_mut(name).unwrap();
    if app.generation == generation {
        if let AppState::Failed(error) = &state {
            runner::announce(true, format!("paraco: {name}: {error}"));
        }
        app.state = state;
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)] // Keep focused origin tests near app_host helpers.
mod origin_tests {
    use super::app_host;
    use std::path::Path;

    #[test]
    fn application_hosts_are_stable_and_distinct() {
        assert_eq!(
            app_host("alpha", Path::new("/a"), 8787),
            app_host("alpha", Path::new("/a"), 8787)
        );
        assert_ne!(
            app_host("alpha", Path::new("/a"), 8787),
            app_host("alpha", Path::new("/b"), 8787)
        );
        assert!(app_host("alpha", Path::new("/a"), 8787).ends_with(".localhost:8787"));
    }
}

fn manage(apps: &Inventory, command: control::Command) -> Result<Vec<control::Status>, String> {
    let mut records = apps.write().unwrap();
    if let Some(name) = &command.app {
        let app = records
            .get_mut(name)
            .ok_or_else(|| format!("unknown application `{name}`"))?;
        let desired = match command.action {
            control::Action::Status => None,
            control::Action::Start
                if app.desired == DesiredState::Running
                    && !matches!(app.state, AppState::Failed(_)) =>
            {
                None
            }
            control::Action::Stop if app.desired == DesiredState::Stopped => None,
            control::Action::Start | control::Action::Restart => Some(DesiredState::Running),
            control::Action::Stop => Some(DesiredState::Stopped),
        };
        if let Some(desired) = desired {
            app.cancel.store(true, Ordering::Relaxed);
            app.cancel = Arc::new(AtomicBool::new(false));
            app.generation += 1;
            app.desired = desired;
            app.state = match (&app.state, desired) {
                (AppState::Stopped | AppState::Failed(_), DesiredState::Running) => {
                    AppState::Starting
                }
                (AppState::Stopped | AppState::Failed(_), DesiredState::Stopped) => {
                    AppState::Stopped
                }
                _ => AppState::Stopping,
            };
        }
    } else if !matches!(command.action, control::Action::Status) {
        return Err("an application name is required".into());
    }
    Ok(records
        .iter()
        .filter(|(name, _)| command.app.as_ref().is_none_or(|app| app == *name))
        .map(|(name, app)| {
            let (state, error) = match &app.state {
                AppState::Starting => ("starting", None),
                AppState::Running(_, _) => ("running", None),
                AppState::Stopping => ("stopping", None),
                AppState::Stopped => ("stopped", None),
                AppState::Failed(error) => ("failed", Some(error.clone())),
                AppState::Backoff(error) => ("backoff", Some(error.clone())),
            };
            control::Status {
                name: name.clone(),
                desired: if app.desired == DesiredState::Running {
                    "running"
                } else {
                    "stopped"
                }
                .into(),
                state: state.into(),
                error,
            }
        })
        .collect())
}
