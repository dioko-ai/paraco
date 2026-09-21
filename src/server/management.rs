//! Browser management has its own origin and a per-server bearer credential.
use super::*;

#[derive(Clone)]
pub(super) struct Management {
    apps: Inventory,
    durable: Arc<Mutex<state::Store>>,
    origin: String,
    authorization: String,
    gateway_port: u16,
    store: logs::Store,
    // SQLite tail reads run on the blocking pool. Keep their admission separate
    // from request proxying so dashboard refreshes cannot consume it without
    // bound, while also avoiding an unbounded spawn_blocking backlog.
    log_reads: Arc<tokio::sync::Semaphore>,
}

pub(super) async fn bind(
    apps: Inventory,
    durable: Arc<Mutex<state::Store>>,
    gateway_port: u16,
    store: logs::Store,
) -> Result<(tokio::net::TcpListener, Router, String), String> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| format!("cannot bind management dashboard: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|_| "cannot generate management token")?;
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let origin = format!("http://127.0.0.1:{port}");
    let url = format!("{origin}/#{token}");
    let state = Management {
        apps,
        durable,
        origin,
        authorization: format!("Bearer {token}"),
        gateway_port,
        store,
        log_reads: Arc::new(tokio::sync::Semaphore::new(4)),
    };
    Ok((
        listener,
        Router::new().fallback(dispatch).with_state(state),
        url,
    ))
}

async fn dispatch(State(state): State<Management>, request: Request) -> Response {
    let mut response = handle(&state, request).await;
    let headers = response.headers_mut();
    for (name, value) in [
        ("cache-control", "no-store"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-resource-policy", "same-origin"),
        (
            "content-security-policy",
            "default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
        ),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

async fn handle(state: &Management, request: Request) -> Response {
    let headers = request.headers();
    if headers.get(header::HOST).and_then(|h| h.to_str().ok())
        != state.origin.strip_prefix("http://")
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    // Reject cross-origin browser requests, even with a copied credential. No CORS.
    if headers
        .get(header::ORIGIN)
        .is_some_and(|h| h != state.origin.as_str())
        || headers
            .get("sec-fetch-site")
            .is_some_and(|h| h != "same-origin" && h != "none")
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let path = request.uri().path();
    if request.method() == Method::GET {
        match path {
            "/" => return Html(include_str!("../../runtime/management.html")).into_response(),
            "/dashboard.js" => {
                return (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../../runtime/management.js"),
                )
                    .into_response();
            }
            _ => {}
        }
    }
    if path != "/api/apps" && path != "/api/logs" {
        return StatusCode::NOT_FOUND.into_response();
    }
    if headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        != Some(state.authorization.as_str())
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if path == "/api/logs" {
        if request.method() != Method::GET {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        let mut app = None;
        let mut run_id = None;
        let mut limit = 100usize;
        for (key, value) in
            url::form_urlencoded::parse(request.uri().query().unwrap_or("").as_bytes())
        {
            match key.as_ref() {
                "app" if app.is_none() => app = Some(value.into_owned()),
                "run_id"
                    if run_id.is_none()
                        && value.len() == 32
                        && value.bytes().all(|b| b.is_ascii_hexdigit()) =>
                {
                    run_id = Some(value.into_owned())
                }
                "limit" => match value.parse::<usize>() {
                    Ok(n) if n <= 500 => limit = n,
                    _ => return (StatusCode::BAD_REQUEST, "limit must be 0–500").into_response(),
                },
                _ => return StatusCode::BAD_REQUEST.into_response(),
            }
        }
        let Some(app) = app else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        if !state.apps.read().unwrap().contains_key(&app) {
            return StatusCode::NOT_FOUND.into_response();
        }
        let store_losses = state.store.losses();
        let store = state.store.clone();
        let port = state.gateway_port;
        // Do not queue a potentially slow file/SQLite read. The owned permit
        // remains alive until the blocking job has completed (or its join has
        // been cancelled), so there are at most four admitted jobs.
        let Ok(_read_permit) = state.log_reads.clone().try_acquire_owned() else {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Log reader is busy; retry refresh.",
            )
                .into_response();
        };
        return match tokio::task::spawn_blocking(move || {
            // The permit deliberately belongs to this closure rather than the
            // request future. A disconnected browser drops its JoinHandle,
            // but cannot admit another filesystem job until this one ends.
            let _read_permit = _read_permit;
            store.tail_launch(Some(&app), Some(port), run_id.as_deref(), limit)
        })
        .await
        {
            Ok(Ok(records)) => (
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"records": records, "store_losses": store_losses, "console_losses": runner::console_losses()}).to_string(),
            )
                .into_response(),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to read logs; retry refresh.",
            )
                .into_response(),
        };
    }
    let command = if request.method() == Method::GET {
        control::Command {
            action: control::Action::Status,
            app: None,
        }
    } else if request.method() == Method::POST {
        if headers.get(header::ORIGIN).and_then(|h| h.to_str().ok()) != Some(&state.origin) {
            return StatusCode::FORBIDDEN.into_response();
        }
        if headers
            .get(header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            != Some("application/json")
        {
            return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
        }
        let body =
            match tokio::time::timeout(Duration::from_secs(2), to_bytes(request.into_body(), 4096))
                .await
            {
                Ok(Ok(body)) => body,
                Ok(Err(_)) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
                Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
            };
        match serde_json::from_slice::<control::Command>(&body) {
            Ok(command) => command,
            Err(_) => {
                return (StatusCode::BAD_REQUEST, "Invalid lifecycle command").into_response();
            }
        }
    } else {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET, POST")],
        )
            .into_response();
    };
    match manage(&state.apps, &state.durable, command) {
        Ok(apps) => {
            let roots = state.apps.read().unwrap();
            let apps: Vec<_> = apps
                .into_iter()
                .map(|app| {
                    let mut value = serde_json::to_value(&app).unwrap();
                    if let Some(record) = roots.get(&app.name) {
                        value["url"] = serde_json::Value::String(format!(
                            "http://{}",
                            app_host(&record.deployment_id, state.gateway_port)
                        ));
                    }
                    value
                })
                .collect();
            (
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"apps": apps}).to_string(),
            )
                .into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, error).into_response(),
    }
}
