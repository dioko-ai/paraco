use crate::{capability::Capability, logs, manifest};
use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const CONSOLE_QUEUE_RECORDS: usize = 256;
const HOST_SOURCE: &str = include_str!("../runtime/deno_host.ts");
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
    let mut running = RunningApp::start(app, port, ai_config, "/", None, &stop, log)?;
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

/// Owns every resource of one app. Dropping it always stops and reaps Deno.
pub struct RunningApp {
    child: Child,
    pub port: u16,
    output: Option<thread::JoinHandle<()>>,
    errors: Option<thread::JoinHandle<()>>,
    _capability: Option<Capability>,
    _temporary: tempfile::TempDir,
    log: logs::AppLog,
}

impl RunningApp {
    pub fn start(
        app: manifest::App,
        port: u16,
        ai_config: Option<&Path>,
        base_path: &str,
        backend_token: Option<&str>,
        stop: &AtomicBool,
        log: logs::AppLog,
    ) -> Result<Self, String> {
        log.event("starting", "Starting application");
        let result = Self::launch(
            app,
            port,
            ai_config,
            base_path,
            backend_token,
            stop,
            log.clone(),
        );
        if let Err(error) = &result {
            log.event("failed", error);
        }
        result
    }

    fn launch(
        app: manifest::App,
        port: u16,
        ai_config: Option<&Path>,
        base_path: &str,
        backend_token: Option<&str>,
        stop: &AtomicBool,
        log: logs::AppLog,
    ) -> Result<Self, String> {
        let capability = if app.requests_ai {
            Some(Capability::start(&app, ai_config)?)
        } else {
            if ai_config.is_some() {
                return Err("AI configuration requires an app requesting ai".into());
            }
            None
        };
        let temporary = tempfile::tempdir()
            .map_err(|error| format!("cannot create runtime temporary directory: {error}"))?;
        let host_path = temporary.path().join("paraco-deno-host.ts");
        std::fs::write(&host_path, HOST_SOURCE)
            .map_err(|error| format!("cannot write Deno host adapter: {error}"))?;
        let nonce = readiness_nonce();
        let entrypoint_url = url::Url::from_file_path(&app.entrypoint)
            .map_err(|_| "cannot create entrypoint file URL")?;
        let bootstrap = serde_json::json!({
            "basePath": base_path,
            "backendToken": backend_token,
            "ai": capability.as_ref().map(|c| serde_json::json!({"address": c.address, "token": c.token}))
        });
        let input = serde_json::to_vec(&bootstrap).map_err(|_| "cannot encode host bootstrap")?;
        let mut command = Command::new("deno");
        command
            .arg("run")
            .arg("--quiet")
            .arg("--no-prompt")
            .arg(format!("--allow-read={}", app.root.display()))
            .arg(format!(
                "--allow-net=127.0.0.1:{port}{}",
                capability
                    .as_ref()
                    .map(|c| format!(",{}", c.address))
                    .unwrap_or_default()
            ))
            .arg(&host_path)
            .arg(entrypoint_url.as_str())
            .arg(port.to_string())
            .arg(&nonce)
            .current_dir(&app.root)
            .env_clear()
            .env("DENO_DIR", temporary.path().join("deno-cache"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
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
            port,
            output: None,
            errors: None,
            _capability: capability,
            _temporary: temporary,
            log: log.clone(),
        };
        running
            .child
            .stdin
            .take()
            .unwrap()
            .write_all(&input)
            .map_err(|_| "cannot initialize Deno host")?;
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
