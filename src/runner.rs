use crate::{capability::Capability, manifest};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const HOST_SOURCE: &str = include_str!("../runtime/deno_host.ts");

pub fn run(app_path: &Path, port: u16, ai_config: Option<&Path>) -> Result<(), String> {
    if port == 0 {
        return Err("port must be from 1 through 65535".into());
    }
    let app = manifest::load(app_path).map_err(|error| error.to_string())?;
    let stop = install_interrupt_handler()?;
    let name = app.name.clone();
    let mut running = RunningApp::start(app, port, ai_config, "/", &stop)?;
    println!(
        "paraco: {name} listening on http://127.0.0.1:{}",
        running.port
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
}

impl RunningApp {
    pub fn start(
        app: manifest::App,
        port: u16,
        ai_config: Option<&Path>,
        base_path: &str,
        stop: &AtomicBool,
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
        let mut running = Self {
            child,
            port,
            output: None,
            errors: None,
            _capability: capability,
            _temporary: temporary,
        };
        running
            .child
            .stdin
            .take()
            .unwrap()
            .write_all(&input)
            .map_err(|_| "cannot initialize Deno host")?;
        let (ready_tx, ready_rx) = mpsc::channel();
        running.output = Some(forward_stdout(
            running.child.stdout.take().unwrap(),
            format!("PARACO_READY:{nonce}:"),
            ready_tx,
        ));
        running.errors = Some(forward_stderr(running.child.stderr.take().unwrap()));
        running.port = wait_until_ready(&mut running.child, &ready_rx, stop)?;
        Ok(running)
    }

    pub fn check(&mut self) -> Result<(), String> {
        if let Some(status) = self
            .child
            .try_wait()
            .map_err(|e| format!("cannot check Deno process: {e}"))?
        {
            Err(format!(
                "application exited unexpectedly with status {status}"
            ))
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
    }
}

fn forward_stdout(
    stdout: impl std::io::Read + Send + 'static,
    marker: String,
    ready: mpsc::Sender<u16>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) if line.starts_with(&marker) => {
                    if let Ok(port) = line[marker.len()..].parse::<u16>()
                        && port != 0
                    {
                        let _ = ready.send(port);
                    }
                }
                Ok(line) => println!("{line}"),
                Err(error) => {
                    eprintln!("paraco: error reading Deno stdout: {error}");
                    break;
                }
            }
        }
    })
}

fn forward_stderr(stderr: impl std::io::Read + Send + 'static) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            match line {
                Ok(line) => eprintln!("{line}"),
                Err(error) => {
                    eprintln!("paraco: error reading Deno stderr: {error}");
                    break;
                }
            }
        }
    })
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
