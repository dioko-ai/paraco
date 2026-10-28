//! Main dashboard and same-origin local browser controls.
use super::*;

#[derive(Clone)]
pub(super) struct Management {
    apps: Inventory,
    durable: Arc<Mutex<state::Store>>,
    origin: String,
    gateway_port: u16,
    store: logs::Store,
    // SQLite tail reads run on the blocking pool. Keep their admission separate
    // from request proxying so dashboard refreshes cannot consume it without
    // bound, while also avoiding an unbounded spawn_blocking backlog.
    log_reads: Arc<tokio::sync::Semaphore>,
    plugins: PathBuf,
    stop: Arc<AtomicBool>,
    plugin_jobs: Arc<tokio::sync::Mutex<()>>,
    workers: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
}

pub(super) fn new(
    apps: Inventory,
    durable: Arc<Mutex<state::Store>>,
    gateway_port: u16,
    store: logs::Store,
    plugins: PathBuf,
    stop: Arc<AtomicBool>,
    workers: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
) -> Management {
    Management {
        plugins,
        stop,
        plugin_jobs: Arc::new(tokio::sync::Mutex::new(())),
        workers,
        apps,
        durable,
        origin: String::new(),
        gateway_port,
        store,
        log_reads: Arc::new(tokio::sync::Semaphore::new(4)),
    }
}

pub(super) async fn dispatch(mut state: Management, request: Request) -> Response {
    // Gateway dispatch has already restricted Host to the loopback dashboard
    // aliases. Match Origin against the alias used for this request.
    state.origin = format!(
        "http://{}",
        request
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
    );
    let head = request.method() == Method::HEAD;
    let mut response = handle(&state, request).await;
    if head {
        *response.body_mut() = Body::empty();
    }
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
    // Local app origins remain separate; browser controls use same-origin requests.
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
    if request.method() == Method::GET || request.method() == Method::HEAD {
        match path {
            "/" => return Html(include_str!("../../runtime/dashboard.html")).into_response(),
            "/ai-settings.js" => {
                return (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../../runtime/ai-settings.js"),
                )
                    .into_response();
            }
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
    if path == "/" || path == "/dashboard.js" || path == "/ai-settings.js" {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET, HEAD")],
        )
            .into_response();
    }
    if path == "/api/ai" {
        return ai_settings(state, request).await;
    }
    if path != "/api/apps" && path != "/api/logs" && path != "/api/plugins" {
        return StatusCode::NOT_FOUND.into_response();
    }
    if path == "/api/plugins" {
        return plugins(state, request).await;
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
                        value["scheduling_independent"] = serde_json::Value::Bool(
                            state
                                .durable
                                .lock()
                                .unwrap()
                                .task_grants
                                .contains_key(&record.deployment_id),
                        );
                        value["uninstallable"] = serde_json::Value::Bool(
                            record.source.parent() == Some(state.plugins.as_path()),
                        );
                        value["url"] = serde_json::Value::String(format!(
                            "http://{}",
                            app_host(&app.name, state.gateway_port)
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginCommand {
    action: String,
    name: String,
    version: Option<String>,
    #[serde(default)]
    setup: PluginSettings,
}

#[derive(Clone, Default, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PluginSettings {
    #[serde(default)]
    pub approved: bool,
    pub ai_config: Option<PathBuf>,
    #[serde(default)]
    pub task_authoring: bool,
    #[serde(default)]
    pub task_network: Vec<String>,
}

pub(super) fn plugin_settings(path: &Path) -> Result<PluginSettings, String> {
    match std::fs::read(path.join(".paraco-host-settings.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(PluginSettings::default()),
        Err(e) => Err(e.to_string()),
    }
}

fn install_reason(entry: &serde_json::Value) -> &'static str {
    let caps = entry["capabilities"].as_array();
    if caps.is_some_and(|caps| caps.iter().any(|cap| cap == "ai" || cap == "tasks")) {
        "This plugin requires capability approval. Choose Set up and install to review AI settings and task network grants for this configured host."
    } else {
        ""
    }
}

async fn plugins(state: &Management, request: Request) -> Response {
    if request.method() == Method::GET {
        return match crate::marketplace::dashboard_catalog() {
            Ok(mut value) => {
                let apps = state.apps.read().unwrap();
                if let Some(entries) = value["apps"].as_array_mut() {
                    for entry in entries {
                        let reason = install_reason(entry);
                        let installed = entry["name"]
                            .as_str()
                            .is_some_and(|name| apps.contains_key(name));
                        entry["installed"] = installed.into();
                        entry["installable"] = reason.is_empty().into();
                        entry["install_reason"] = reason.into();
                        entry["setup_available"] = (!reason.is_empty()).into();
                    }
                }
                (
                    [(header::CONTENT_TYPE, "application/json")],
                    value.to_string(),
                )
                    .into_response()
            }
            Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
        };
    }
    if request.method() != Method::POST {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    if request
        .headers()
        .get(header::ORIGIN)
        .and_then(|h| h.to_str().ok())
        != Some(&state.origin)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if request
        .headers()
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
            _ => return StatusCode::BAD_REQUEST.into_response(),
        };
    let command: PluginCommand = match serde_json::from_slice(&body) {
        Ok(command) => command,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let Ok(_job) = state.plugin_jobs.try_lock() else {
        return StatusCode::CONFLICT.into_response();
    };
    match plugin_change(state, command).await {
        Ok(()) => ([(header::CONTENT_TYPE, "application/json")], "{}").into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error).into_response(),
    }
}

async fn plugin_change(state: &Management, command: PluginCommand) -> Result<(), String> {
    paraco::slug::validate(&command.name).map_err(|e| e.to_string())?;
    let path = state.plugins.join(&command.name);
    match command.action.as_str() {
        "install" | "setup" => {
            {
                let apps = state.apps.read().unwrap();
                if apps.contains_key(&command.name) || apps.len() >= 50 {
                    return Err("Already installed or 50 app limit reached".into());
                }
            }
            if std::fs::symlink_metadata(&path).is_ok() {
                return Err("installation destination already exists".into());
            }
            let version = command.version.ok_or("Select a catalog version")?;
            let catalog = crate::marketplace::dashboard_catalog()?;
            let entry = catalog["apps"]
                .as_array()
                .and_then(|entries| {
                    entries
                        .iter()
                        .find(|entry| entry["name"] == command.name && entry["version"] == version)
                })
                .ok_or("app/version is not approved in this catalog")?;
            let reason = install_reason(entry);
            if !reason.is_empty() && (command.action != "setup" || !command.setup.approved) {
                return Err(reason.into());
            }
            let setup = command.setup;
            if (setup.task_authoring || setup.ai_config.is_some() || !setup.task_network.is_empty())
                && !setup.approved
            {
                return Err("Review and approve capabilities before installation".into());
            }
            if !setup.task_authoring && !setup.task_network.is_empty() {
                return Err("Network grants require task authoring approval".into());
            }
            if let Some(path) = &setup.ai_config
                && (!path.is_absolute() || !path.is_file())
            {
                return Err("Choose an existing absolute host AI configuration path".into());
            }
            if setup.task_network.len() > 32
                || setup.task_network.iter().any(|host| {
                    host.is_empty() || host.contains(['/', '*', ',', ' ']) || host.starts_with('-')
                })
            {
                return Err("Use explicit network hosts, without wildcards or URLs".into());
            }
            let staging = tempfile::Builder::new()
                .prefix(".plugin-prepare-")
                .tempdir_in(&state.plugins)
                .map_err(|e| e.to_string())?;
            let staged = staging.path().join("source");
            crate::marketplace::dashboard_install(&command.name, &version, &staged).await?;
            // Roll back every fallible preparation step, including lease acquisition.
            let prepared = (|| {
                let mut app = manifest::load(&staged).map_err(|e| e.to_string())?;
                if app.requests_tasks && !setup.task_authoring {
                    return Err("Approve task authoring to install this app".into());
                }
                std::fs::write(
                    staged.join(".paraco-host-settings.json"),
                    serde_json::to_vec(&setup).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                app.entrypoint = path.join(
                    app.entrypoint
                        .strip_prefix(&app.root)
                        .map_err(|e| e.to_string())?,
                );
                app.root = path.clone();
                state.store.outside(&app.root)?;
                let mut durable = state.durable.lock().unwrap();
                let lease = Arc::new(durable.guardian_lease()?);
                if setup.task_authoring && durable.tasks.is_none() {
                    let tasks = crate::tasks::Tasks::open(
                        state.plugins.parent().unwrap(),
                        crate::runtime::command_for_host()?,
                    )?;
                    tasks.set_lease(durable.guardian_lease()?);
                    durable.tasks = Some(tasks);
                }
                let deployment = durable.install_plugin(&app.root, &command.name)?;
                if let Err(error) = durable.check_capabilities(&deployment.id, &app, false) {
                    durable
                        .retire_plugin(&app.root, &command.name)
                        .map_err(|rollback| {
                            format!("{error}; deployment rollback failed: {rollback}")
                        })?;
                    return Err(error);
                }
                if let Err(error) = crate::marketplace::publish_new(&staged, &path) {
                    durable
                        .retire_plugin(&path, &command.name)
                        .map_err(|rollback| {
                            format!("{error}; deployment rollback failed: {rollback}")
                        })?;
                    return Err(error);
                }
                if setup.task_authoring {
                    let tasks = durable.tasks.as_ref().unwrap();
                    tasks.set_grant(deployment.id.clone(), setup.task_network.clone());
                    tasks.start();
                    durable
                        .task_grants
                        .insert(deployment.id.clone(), setup.task_network.clone());
                }
                Ok((app, deployment, lease))
            })();
            let (app, deployment, lease) = prepared?;
            state.apps.write().unwrap().insert(
                command.name,
                AppRecord {
                    requests_ai: app.requests_ai,
                    explicit_ai: false,
                    deployment_id: deployment.id,
                    source: app.root.clone(),
                    root: app.root.clone(),
                    desired: DesiredState::Running,
                    state: AppState::Starting,
                    generation: 0,
                    cancel: Arc::new(AtomicBool::new(false)),
                    admission: Arc::new(Semaphore::new(8)),
                },
            );
            let prepared = PreparedApp {
                dashboard_plugin: true,
                app,
                task_authoring: setup.task_authoring,
                task_network: setup.task_network,
                restart: RestartPolicy::default(),
                ai_config: setup.ai_config,
            };
            let worker_state = state.clone();
            state.workers.lock().unwrap().push(thread::spawn(move || {
                supervise(
                    prepared,
                    worker_state.durable,
                    worker_state.apps,
                    worker_state.stop,
                    worker_state.store,
                    worker_state.gateway_port,
                    Some(lease),
                );
            }));
        }
        "uninstall" => {
            let mut apps = state.apps.write().unwrap();
            let app = apps.get(&command.name).ok_or("Unknown plugin")?;
            if app.source != path || !matches!(app.state, AppState::Stopped) {
                return Err("Stop the downloaded plugin before uninstalling it".into());
            }
            let catalog = crate::marketplace::dashboard_catalog()?;
            let catalogued = catalog["apps"]
                .as_array()
                .is_some_and(|entries| entries.iter().any(|entry| entry["name"] == command.name));
            // Move the complete tree aside before deleting it, so a filesystem
            // failure cannot leave a partially removed live installation.
            let staging = tempfile::Builder::new()
                .prefix(".uninstall-")
                .tempdir_in(&state.plugins)
                .map_err(|e| e.to_string())?;
            let staged = staging.path().join("plugin");
            if catalogued {
                state
                    .durable
                    .lock()
                    .unwrap()
                    .retire_plugin(&app.source, &command.name)?;
            }
            if let Err(error) = std::fs::rename(&path, &staged) {
                if catalogued {
                    let mut durable = state.durable.lock().unwrap();
                    if durable.install_plugin(&path, &command.name).is_ok() {
                        let _ = durable.stop(&path);
                    }
                }
                return Err(error.to_string());
            }
            if !catalogued {
                let result = state.durable.lock().unwrap().reconcile(
                    apps.iter()
                        .filter(|(name, _)| *name != &command.name)
                        .map(|(_, app)| app.source.clone()),
                );
                if let Err(error) = result {
                    if let Err(restore) = std::fs::rename(&staged, &path) {
                        let retained = staging.keep();
                        return Err(format!(
                            "{error}; failed to restore plugin source: {restore}; source retained at {}",
                            retained.display()
                        ));
                    }
                    return Err(error);
                }
            }
            let deployment = app.deployment_id.clone();
            apps.remove(&command.name);
            let mut durable = state.durable.lock().unwrap();
            if let Some(tasks) = &durable.tasks {
                tasks.revoke_grant(&deployment);
            }
            durable.task_grants.remove(&deployment);
            drop(durable);
            if let Err(error) = staging.close() {
                eprintln!("uninstalled plugin source cleanup failed: {error}");
            }
        }
        _ => return Err("Unknown plugin action".into()),
    }
    Ok(())
}

async fn ai_settings(state: &Management, request: Request) -> Response {
    let Some(root) = paraco::ai_settings::root() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if request.method() == Method::POST {
        if request
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            != Some(&state.origin)
        {
            return StatusCode::FORBIDDEN.into_response();
        }
        if request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            != Some("application/json")
        {
            return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
        }
        let body = match tokio::time::timeout(
            Duration::from_secs(2),
            to_bytes(request.into_body(), 16384),
        )
        .await
        {
            Ok(Ok(body)) => body,
            _ => return StatusCode::BAD_REQUEST.into_response(),
        };
        let update: paraco::ai_settings::Update = match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(_) => return (StatusCode::BAD_REQUEST, "Invalid AI settings").into_response(),
        };
        let deployment = if let Some(name) = &update.app {
            let apps = state.apps.read().unwrap();
            let Some(app) = apps.get(name) else {
                return StatusCode::NOT_FOUND.into_response();
            };
            if !app.requests_ai {
                return (StatusCode::BAD_REQUEST, "This app does not request AI").into_response();
            }
            if app.explicit_ai {
                return (StatusCode::CONFLICT, "This app uses an explicit AI configuration file; remove aiConfig to use dashboard settings").into_response();
            }
            Some(app.deployment_id.clone())
        } else {
            None
        };
        if let Err(error) = paraco::ai_settings::update(root, deployment.as_deref(), update) {
            return (StatusCode::BAD_REQUEST, error).into_response();
        }
    } else if request.method() != Method::GET {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    match paraco::ai_settings::read(root) {
        Ok(settings) => {
            let apps: Vec<_> = state.apps.read().unwrap().iter().map(|(name, app)| {
                serde_json::json!({"name": name, "requests_ai":app.requests_ai, "explicit_ai":app.explicit_ai,
                    "override":settings.apps.get(&app.deployment_id)})
            }).collect();
            // No credential values are ever returned. Paths are host dashboard metadata only.
            (
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"default":settings.default,"apps":apps}).to_string(),
            )
                .into_response()
        }
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guided_settings_survive_restart_and_missing_settings_grant_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!plugin_settings(dir.path()).unwrap().approved);
        let setup = PluginSettings {
            approved: true,
            ai_config: None,
            task_authoring: true,
            task_network: vec!["example.com:443".into()],
        };
        std::fs::write(
            dir.path().join(".paraco-host-settings.json"),
            serde_json::to_vec(&setup).unwrap(),
        )
        .unwrap();
        let loaded = plugin_settings(dir.path()).unwrap();
        assert!(loaded.approved && loaded.task_authoring);
        assert_eq!(loaded.task_network, ["example.com:443"]);
        std::fs::write(dir.path().join(".paraco-host-settings.json"), "broken").unwrap();
        assert!(plugin_settings(dir.path()).is_err());
    }
    #[tokio::test]
    async fn catalog_is_available_without_token_and_mutations_require_origin() {
        let dir = tempfile::tempdir().unwrap();
        let state = Management {
            apps: Arc::new(RwLock::new(BTreeMap::new())),
            durable: Arc::new(Mutex::new(
                state::Store::open(&dir.path().join("state")).unwrap(),
            )),
            origin: "http://127.0.0.1:1234".into(),
            gateway_port: 3000,
            store: logs::Store::open(&dir.path().join("logs")).unwrap(),
            log_reads: Arc::new(Semaphore::new(4)),
            plugins: dir.path().join("plugins"),
            stop: Arc::new(AtomicBool::new(false)),
            plugin_jobs: Arc::new(tokio::sync::Mutex::new(())),
            workers: Arc::new(Mutex::new(vec![])),
        };
        let request = |method, origin: Option<&str>| {
            let mut builder = Request::builder()
                .method(method)
                .uri("/api/plugins")
                .header(header::HOST, "127.0.0.1:1234");
            if let Some(origin) = origin {
                builder = builder.header(header::ORIGIN, origin);
            }
            builder.body(Body::empty()).unwrap()
        };
        assert_eq!(
            handle(&state, request(Method::GET, None)).await.status(),
            StatusCode::OK
        );
        let response = handle(&state, request(Method::GET, None)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let catalog: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(!catalog["apps"].as_array().unwrap().is_empty());
        let catalog_apps = catalog["apps"].as_array().unwrap();
        let task_app = catalog_apps
            .iter()
            .find(|app| app["name"] == "cron-time-machine")
            .unwrap();
        assert_eq!(task_app["installed"], false);
        assert_eq!(task_app["installable"], false);
        assert_eq!(task_app["setup_available"], true);
        assert!(
            task_app["install_reason"]
                .as_str()
                .unwrap()
                .contains("configured host")
        );
        let unsupported = plugin_change(
            &state,
            PluginCommand {
                action: "install".into(),
                name: "cron-time-machine".into(),
                version: Some("0.1.0".into()),
                setup: PluginSettings::default(),
            },
        )
        .await
        .unwrap_err();
        assert!(unsupported.contains("configured host"));
        assert!(!state.plugins.join("cron-time-machine").exists());
        assert_eq!(
            handle(&state, request(Method::POST, None)).await.status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            handle(&state, request(Method::GET, Some("http://evil.localhost")))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        let result = plugin_change(
            &state,
            PluginCommand {
                action: "install".into(),
                name: "../escape".into(),
                version: Some("0.1.0".into()),
                setup: PluginSettings::default(),
            },
        )
        .await;
        assert!(result.is_err());
        let path = state.plugins.join("sample");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("main.js"), "export default {};").unwrap();
        let deployment = state.durable.lock().unwrap().install(&path).unwrap();
        state.apps.write().unwrap().insert(
            "sample".into(),
            AppRecord {
                requests_ai: false,
                explicit_ai: false,
                deployment_id: deployment.id,
                source: path.clone(),
                root: path.clone(),
                desired: DesiredState::Running,
                state: AppState::Starting,
                generation: 0,
                cancel: Arc::new(AtomicBool::new(false)),
                admission: Arc::new(Semaphore::new(8)),
            },
        );
        let uninstall = || PluginCommand {
            action: "uninstall".into(),
            name: "sample".into(),
            version: None,
            setup: PluginSettings::default(),
        };
        assert!(plugin_change(&state, uninstall()).await.is_err());
        assert!(path.exists());
        state.apps.write().unwrap().get_mut("sample").unwrap().state = AppState::Stopped;
        plugin_change(&state, uninstall()).await.unwrap();
        assert!(!path.exists());
        assert!(state.apps.read().unwrap().is_empty());
    }
}
