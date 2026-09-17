use crate::{manifest, runner};
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    apps: Vec<AppConfig>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AppConfig {
    path: PathBuf,
    ai_config: Option<PathBuf>,
}

struct PreparedApp {
    app: manifest::App,
    ai_config: Option<PathBuf>,
}

#[derive(Clone)]
enum AppState {
    Starting,
    Running(u16),
    Failed(String),
}

type Inventory = Arc<RwLock<BTreeMap<String, AppState>>>;

#[derive(Clone)]
struct Gateway {
    apps: Inventory,
    client: reqwest::Client,
    port: u16,
}

/// Supervisors own their app resources; all stop concurrently before joining.
struct Supervisors {
    stop: Arc<AtomicBool>,
    threads: Vec<thread::JoinHandle<()>>,
}
impl Drop for Supervisors {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
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
            Ok(PreparedApp {
                app,
                ai_config: entry.ai_config.map(|p| root.join(p)),
            })
        })
        .collect()
}

pub fn serve(config: &Path, port: u16) -> Result<(), String> {
    if port == 0 {
        return Err("port must be from 1 through 65535".into());
    }
    let apps = load(config)?;
    let stop = runner::install_interrupt_handler()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(host(apps, port, stop))
}

async fn host(apps: Vec<PreparedApp>, port: u16, stop: Arc<AtomicBool>) -> Result<(), String> {
    // Bind the gateway before launching apps: an occupied public port starts none.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("cannot bind gateway: {e}"))?;
    let inventory: Inventory = Arc::new(RwLock::new(
        apps.iter()
            .map(|a| (a.app.name.clone(), AppState::Starting))
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
    };
    for prepared in apps {
        let states = inventory.clone();
        let app_stop = stop.clone();
        supervisors.threads.push(thread::spawn(move || {
            let name = prepared.app.name.clone();
            let base = format!("/apps/{name}/");
            let result = runner::RunningApp::start(
                prepared.app,
                0,
                prepared.ai_config.as_deref(),
                &base,
                &app_stop,
            );
            let mut running = match result {
                Ok(running) => running,
                Err(error) => {
                    eprintln!("paraco: {name}: {error}");
                    states
                        .write()
                        .unwrap()
                        .insert(name, AppState::Failed(error));
                    return;
                }
            };
            states
                .write()
                .unwrap()
                .insert(name.clone(), AppState::Running(running.port));
            while !app_stop.load(Ordering::Relaxed) {
                if let Err(error) = running.check() {
                    eprintln!("paraco: {name}: {error}");
                    states
                        .write()
                        .unwrap()
                        .insert(name, AppState::Failed(error));
                    return;
                }
                thread::sleep(Duration::from_millis(25));
            }
        }));
    }
    let gateway = Gateway {
        apps: inventory,
        client,
        port,
    };
    let app = Router::new().fallback(dispatch).with_state(gateway);
    let mut serving = tokio::spawn(async move { axum::serve(listener, app).await });
    println!("paraco: dashboard listening on http://127.0.0.1:{port}");
    let result = tokio::select! {
        result = &mut serving => result.map_err(|e| e.to_string())?.map_err(|e| e.to_string()),
        _ = async { while !stop.load(Ordering::Relaxed) { tokio::time::sleep(Duration::from_millis(25)).await; } } => Ok(()),
    };
    // Stop accepting requests and cancel in-flight proxy calls on shutdown.
    serving.abort();
    // Dropping the owners reaps every child, including apps still starting.
    drop(supervisors);
    result
}

async fn dispatch(State(gateway): State<Gateway>, request: Request) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if host != format!("127.0.0.1:{}", gateway.port)
        && host != format!("localhost:{}", gateway.port)
    {
        return (StatusCode::BAD_REQUEST, "Use the local gateway address").into_response();
    }
    let path = request.uri().path();
    if path == "/" {
        if request.method() != Method::GET && request.method() != Method::HEAD {
            return (
                StatusCode::METHOD_NOT_ALLOWED,
                [(header::ALLOW, "GET, HEAD")],
                "Read-only dashboard",
            )
                .into_response();
        }
        let mut response = dashboard(&gateway.apps).into_response();
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
    let state = gateway.apps.read().unwrap().get(name).cloned();
    let Some(state) = state else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if suffix.is_none() {
        let query = request
            .uri()
            .query()
            .map(|q| format!("?{q}"))
            .unwrap_or_default();
        return (
            StatusCode::PERMANENT_REDIRECT,
            [(header::LOCATION, format!("/apps/{name}/{query}"))],
        )
            .into_response();
    }
    let port = match state {
        AppState::Running(port) => port,
        AppState::Starting => {
            return (StatusCode::SERVICE_UNAVAILABLE, "Application is starting").into_response();
        }
        AppState::Failed(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Application failed; see the dashboard",
            )
                .into_response();
        }
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
    let target = format!("http://127.0.0.1:{port}/{}{query}", suffix.unwrap());
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
    let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
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

fn dashboard(apps: &Inventory) -> Html<String> {
    let apps = apps.read().unwrap();
    let rows: String = apps.iter().map(|(name, state)| {
        let (label, detail) = match state {
            AppState::Starting => ("starting", "Starting application".to_string()),
            AppState::Running(_) => ("running", format!("<a href=\"/apps/{name}/\">Open app <span aria-hidden=\"true\">↗</span></a>")),
            AppState::Failed(error) => ("failed", escape(error)),
        };
        format!("<li class=\"app\"><div><h2>{name}</h2><p class=\"path\">/apps/{name}/</p></div><span class=\"status {label}\">{label}</span><div class=\"detail\">{detail}</div></li>")
    }).collect();
    let running = apps
        .values()
        .filter(|s| matches!(s, AppState::Running(_)))
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
