use crate::{artifact, capability::Capability, guardian, logs, manifest, runtime, state};
use std::io::Write;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
// The guardian gives its Deno child five seconds after receiving our signal.
// Leave it time to reap that child before escalating the guardian itself.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(7);
const CONSOLE_QUEUE_RECORDS: usize = 256;
#[derive(Clone, Copy, serde::Serialize)]
pub struct ConsoleLossCounters {
    pub queue_drops: u64,
    pub write_failures: u64,
}
static CONSOLE_QUEUE_DROPS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CONSOLE_WRITE_FAILURES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub fn console_losses() -> ConsoleLossCounters {
    ConsoleLossCounters {
        queue_drops: CONSOLE_QUEUE_DROPS.load(Ordering::Relaxed),
        write_failures: CONSOLE_WRITE_FAILURES.load(Ordering::Relaxed),
    }
}

pub fn run(
    app_path: &Path,
    port: u16,
    ai_config: Option<&Path>,
    log_dir: &Path,
) -> Result<(), String> {
    if port == 0 {
        return Err("port must be from 1 through 65535".into());
    }
    let app = manifest::load(app_path).map_err(|error| error.to_string())?;
    let store = logs::Store::open(log_dir)?;
    store.outside(&app.root)?;
    announce(true, format!("paraco: logs at {}", store.path().display()));
    let log = store.app(&app.name, "run", port)?;
    let stop = install_interrupt_handler()?;
    let name = app.name.clone();
    let (_state, deployment_id) = standalone_state(&app.root, log_dir)?;
    let mut running = RunningApp::start(
        app,
        &deployment_id,
        Arc::new(Mutex::new(_state)),
        port,
        ai_config,
        "/",
        None,
        &stop,
        log,
        None,
    )?;
    announce(
        false,
        format!(
            "paraco: {name} listening on http://127.0.0.1:{}",
            running.port
        ),
    );
    while !stop.load(Ordering::Relaxed) {
        running.check()?;
        thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}

pub fn run_prepared(
    artifact_path: &Path,
    port: u16,
    ai_config: Option<&Path>,
    log_dir: &Path,
) -> Result<(), String> {
    if port == 0 {
        return Err("port must be from 1 through 65535".into());
    }
    let artifact = artifact::open(artifact_path)?;
    let store = logs::Store::open(log_dir)?;
    store.outside(&artifact.app.root)?;
    let log = store.app(&artifact.app.name, "run", port)?;
    let stop = install_interrupt_handler()?;
    let name = artifact.app.name.clone();
    let (_state, deployment_id) = standalone_state(&artifact.app.root, log_dir)?;
    let mut running = RunningApp::start_prepared(
        artifact,
        &deployment_id,
        Arc::new(Mutex::new(_state)),
        port,
        ai_config,
        "/",
        None,
        &stop,
        log,
    )?;
    announce(
        false,
        format!(
            "paraco: {name} listening on http://127.0.0.1:{}",
            running.port
        ),
    );
    while !stop.load(Ordering::Relaxed) {
        running.check()?;
        thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}

/// Allocate or discover the host-owned identity used by standalone `run`.
/// Keeping this store open for the process also prevents a concurrent server
/// from claiming the same lifecycle state directory.
fn standalone_state(root: &Path, log_dir: &Path) -> Result<(state::Store, String), String> {
    use sha2::{Digest, Sha256};
    let source = root
        .canonicalize()
        .map_err(|e| format!("cannot canonicalize application source: {e}"))?;
    let digest = Sha256::digest(source.as_os_str().as_encoded_bytes());
    let state_root = log_dir
        .with_file_name("standalone-state")
        .join(format!("{:x}", digest));
    let mut store = state::Store::open(&state_root)?;
    let deployment = store.install(&source)?;
    Ok((store, deployment.id))
}

/// Print the stable ID before configuring grants. This is intentionally a
/// host command, rather than a manifest field controlled by the application.
pub fn standalone_identity(app_path: &Path, log_dir: &Path) -> Result<String, String> {
    let app = manifest::load(app_path).map_err(|error| error.to_string())?;
    let (_store, id) = standalone_state(&app.root, log_dir)?;
    Ok(id)
}

struct PreparedRuntime {
    runtime: std::path::PathBuf,
    cache: std::path::PathBuf,
    lock: std::path::PathBuf,
    config: Option<std::path::PathBuf>,
}

/// Owns every resource of one app. Dropping it always stops and reaps Deno.
pub struct RunningApp {
    child: Child,
    // Kept open for a guardian-managed process. EOF is the guardian's proof
    // that this host disappeared and it must reap Deno.
    _guardian_lifetime: Option<ChildStdin>,
    pub port: u16,
    output: Option<thread::JoinHandle<()>>,
    errors: Option<thread::JoinHandle<()>>,
    _capability: Option<Capability>,
    _storage: Option<crate::storage::Service>,
    _temporary: tempfile::TempDir,
    log: logs::AppLog,
}

impl RunningApp {
    #[allow(clippy::too_many_arguments)] // launch resources are explicit at host boundary.
    pub fn start(
        app: manifest::App,
        deployment_id: &str,
        durable: Arc<Mutex<state::Store>>,
        port: u16,
        ai_config: Option<&Path>,
        base_path: &str,
        backend_token: Option<&str>,
        stop: &AtomicBool,
        log: logs::AppLog,
        guardian_lease: Option<&std::fs::File>,
    ) -> Result<Self, String> {
        log.event("starting", "Starting application");
        let result = Self::launch(
            app,
            deployment_id,
            durable,
            port,
            ai_config,
            base_path,
            backend_token,
            stop,
            log.clone(),
            None,
            guardian_lease,
        );
        if let Err(error) = &result {
            log.event("failed", error);
        }
        result
    }

    #[allow(clippy::too_many_arguments)] // prepared and source launches share this boundary.
    pub fn start_prepared(
        prepared: artifact::PreparedApp,
        deployment_id: &str,
        durable: Arc<Mutex<state::Store>>,
        port: u16,
        ai_config: Option<&Path>,
        base_path: &str,
        backend_token: Option<&str>,
        stop: &AtomicBool,
        log: logs::AppLog,
    ) -> Result<Self, String> {
        let artifact::PreparedApp {
            app,
            runtime,
            cache,
            lock,
            config,
        } = prepared;
        log.event("starting", "Starting prepared application");
        let result = Self::launch(
            app,
            deployment_id,
            durable,
            port,
            ai_config,
            base_path,
            backend_token,
            stop,
            log.clone(),
            Some(PreparedRuntime {
                runtime,
                cache,
                lock,
                config,
            }),
            None,
        );
        if let Err(error) = &result {
            log.event("failed", error);
        }
        result
    }

    #[allow(clippy::too_many_arguments)] // each resource has a distinct lifetime.
    fn launch(
        app: manifest::App,
        deployment_id: &str,
        durable: Arc<Mutex<state::Store>>,
        port: u16,
        ai_config: Option<&Path>,
        base_path: &str,
        backend_token: Option<&str>,
        stop: &AtomicBool,
        log: logs::AppLog,
        prepared: Option<PreparedRuntime>,
        guardian_lease: Option<&std::fs::File>,
    ) -> Result<Self, String> {
        let capability = if app.requests_ai {
            Some(Capability::start(&app, deployment_id, ai_config)?)
        } else {
            if ai_config.is_some() {
                return Err("AI configuration requires an app requesting ai".into());
            }
            None
        };
        let storage = if app.requests_storage {
            Some(crate::storage::Service::start(
                durable,
                deployment_id.to_string(),
            )?)
        } else {
            None
        };
        let temporary = tempfile::tempdir()
            .map_err(|error| format!("cannot create runtime temporary directory: {error}"))?;
        // Deno writes runtime caches even with --cached-only. Keep immutable
        // prepared dependencies pristine and discard per-launch cache writes.
        let launch_cache = temporary.path().join("deno-cache");
        if let Some(prepared) = &prepared {
            artifact::copy_tree(&prepared.cache, &launch_cache)?;
        }
        let nonce = readiness_nonce();
        let entrypoint_url = url::Url::from_file_path(&app.entrypoint)
            .map_err(|_| "cannot create entrypoint file URL")?;
        let bootstrap = serde_json::json!({
            "basePath": base_path,
            "storage": storage.as_ref().map(|s| serde_json::json!({"address":s.address,"token":s.token})),
            "backendToken": backend_token,
            "ai": capability.as_ref().map(|c| serde_json::json!({"address": c.address, "token": c.token, "timeoutMs": c.timeout.as_millis(), "http": {"address": c.http_address, "token": c.http_token}}))
        });
        let mut input =
            serde_json::to_vec(&bootstrap).map_err(|_| "cannot encode host bootstrap")?;
        // Guardian consumes one framed bootstrap line and then retains stdin as
        // the owner lifetime pipe. Keep this delimiter in the shared host path.
        input.push(guardian::BOOTSTRAP_DELIMITER);
        let deno = match &prepared {
            Some(p) => p.runtime.clone(),
            None => runtime::command_for_host()?,
        };
        // The guardian owns the adapter and Deno after the host dies. Pass the
        // already selected absolute runtime/cache so activation cannot swap it.
        let executable = std::env::current_exe()
            .map_err(|e| format!("cannot locate Paraco guardian executable: {e}"))?
            .canonicalize()
            .map_err(|e| format!("cannot resolve immutable Paraco guardian executable: {e}"))?;
        let mut command = Command::new(executable);
        command.arg("__paraco_guardian");
        guardian::inherit_lease(&mut command, guardian_lease)?;
        command.arg("run");
        if let Some(prepared) = &prepared {
            command
                .arg("--cached-only")
                .arg("--frozen")
                .arg("--lock")
                .arg(&prepared.lock);
            if let Some(config) = &prepared.config {
                command.arg("--config").arg(config);
            } else {
                command.arg("--no-config");
            }
        }
        command
            .arg("--quiet")
            .arg("--no-prompt")
            .arg(format!("--allow-read={}", app.root.display()))
            .arg(format!(
                "--allow-net=127.0.0.1:{port}{}{}",
                capability
                    .as_ref()
                    .map(|c| format!(",{},{}", c.address, c.http_address))
                    .unwrap_or_default(),
                storage
                    .as_ref()
                    .map(|s| format!(",{}", s.address))
                    .unwrap_or_default()
            ))
            .arg(entrypoint_url.as_str())
            .arg(port.to_string())
            .arg(&nonce)
            .current_dir(&app.root)
            .env_clear()
            .env("PARACO_GUARDIAN_DENO_DIR", &launch_cache)
            .env("PARACO_GUARDIAN_DENO", deno)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
        // The guardian owns its temporary adapter directory. Preserve an
        // operator-selected temporary root while keeping all other ambient
        // environment out of the app process.
        if let Some(tmpdir) = std::env::var_os("TMPDIR") {
            command.env("TMPDIR", tmpdir);
        }
        let child = command.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "Deno was not found on PATH; install Deno and try again".to_string()
            } else {
                format!("cannot launch Deno: {error}")
            }
        })?;
        log.pid(child.id());
        let mut running = Self {
            child,
            _guardian_lifetime: None,
            port,
            output: None,
            errors: None,
            _capability: capability,
            _storage: storage,
            _temporary: temporary,
            log: log.clone(),
        };
        let mut stdin = running
            .child
            .stdin
            .take()
            .ok_or("cannot initialize guardian")?;
        stdin
            .write_all(&input)
            .map_err(|_| "cannot initialize Deno host")?;
        running._guardian_lifetime = Some(stdin);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        running.output = Some(forward_stdout(
            running.child.stdout.take().unwrap(),
            format!("PARACO_READY:{nonce}:"),
            ready_tx,
            log.clone(),
        ));
        running.errors = Some(forward_stderr(
            running.child.stderr.take().unwrap(),
            log.clone(),
        ));
        running.port = wait_until_ready(&mut running.child, &ready_rx, stop)?;
        log.event("running", "Application listener ready");
        Ok(running)
    }

    pub fn check(&mut self) -> Result<(), String> {
        if let Some(status) = self
            .child
            .try_wait()
            .map_err(|e| format!("cannot check Deno process: {e}"))?
        {
            let error = format!("application exited unexpectedly with status {status}");
            self.log.event("failed", &error);
            Err(error)
        } else {
            Ok(())
        }
    }
}

impl Drop for RunningApp {
    fn drop(&mut self) {
        terminate_and_reap(&mut self.child);
        if let Some(output) = self.output.take() {
            let _ = output.join();
        }
        if let Some(errors) = self.errors.take() {
            let _ = errors.join();
        }
        self.log
            .event("stopped", "Application process reaped and output drained");
        self.log.flush(Duration::from_millis(100));
    }
}

fn forward_stdout(
    stdout: impl std::io::Read + Send + 'static,
    marker: String,
    ready: mpsc::SyncSender<u16>,
    log: logs::AppLog,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Err(error) = logs::lines(stdout, |bytes, truncated| {
            let line = String::from_utf8_lossy(bytes);
            if !truncated && line.starts_with(&marker) {
                if let Ok(port) = line[marker.len()..].parse::<u16>()
                    && port != 0
                {
                    let _ = ready.try_send(port);
                }
            } else {
                log.write("stdout", "output", &line, truncated);
                console(false, line.into_owned());
            }
        }) {
            log.event("read_error", &format!("Cannot read stdout: {error}"));
        }
    })
}

fn forward_stderr(
    stderr: impl std::io::Read + Send + 'static,
    log: logs::AppLog,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Err(error) = logs::lines(stderr, |bytes, truncated| {
            let line = String::from_utf8_lossy(bytes);
            log.write("stderr", "output", &line, truncated);
            console(true, line.into_owned());
        }) {
            log.event("read_error", &format!("Cannot read stderr: {error}"));
        }
    })
}

/// A pair of process-wide bounded console workers. Pipe readers use `try_send`,
/// so an undrained parent stdout/stderr can only lose display copies, never block
/// readiness, JSONL dispatch, or child-pipe draining.
pub fn announce(stderr: bool, line: String) {
    static SINKS: OnceLock<(mpsc::SyncSender<String>, mpsc::SyncSender<String>)> = OnceLock::new();
    let sinks = SINKS.get_or_init(|| {
        let (out_tx, out_rx) = mpsc::sync_channel(CONSOLE_QUEUE_RECORDS);
        let (err_tx, err_rx) = mpsc::sync_channel(CONSOLE_QUEUE_RECORDS);
        thread::spawn(move || {
            for line in out_rx {
                if writeln!(std::io::stdout(), "{line}").is_err() {
                    CONSOLE_WRITE_FAILURES.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        thread::spawn(move || {
            for line in err_rx {
                if writeln!(std::io::stderr(), "{line}").is_err() {
                    CONSOLE_WRITE_FAILURES.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        (out_tx, err_tx)
    });
    if (if stderr {
        sinks.1.try_send(line)
    } else {
        sinks.0.try_send(line)
    })
    .is_err()
    {
        CONSOLE_QUEUE_DROPS.fetch_add(1, Ordering::Relaxed);
    }
}

fn console(stderr: bool, line: String) {
    announce(stderr, line);
}

pub fn install_interrupt_handler() -> Result<Arc<AtomicBool>, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let signal_stop = stop.clone();
    ctrlc::set_handler(move || {
        signal_stop.store(true, Ordering::Relaxed);
    })
    .map_err(|error| format!("cannot install signal handler: {error}"))?;
    Ok(stop)
}

fn wait_until_ready(
    child: &mut Child,
    ready: &mpsc::Receiver<u16>,
    stop: &AtomicBool,
) -> Result<u16, String> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err("interrupted while application was starting".into());
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot check Deno process: {error}"))?
        {
            return Err(format!(
                "application exited during startup with status {status}"
            ));
        }
        if let Ok(port) = ready.try_recv() {
            return Ok(port);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "application did not start within {} seconds",
                STARTUP_TIMEOUT.as_secs()
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn terminate_and_reap(child: &mut Child) {
    if let Ok(Some(_)) = child.try_wait() {
        return;
    }

    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }

    let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => thread::sleep(Duration::from_millis(25)),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn readiness_nonce() -> String {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{time}", std::process::id())
}
