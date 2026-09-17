use crate::manifest;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const HOST_SOURCE: &str = include_str!("../runtime/deno_host.ts");

pub fn run(app_path: &Path, port: u16) -> Result<(), String> {
    let app = manifest::load(app_path).map_err(|error| error.to_string())?;
    let app_dir = &app.root;
    let temporary = tempfile::tempdir()
        .map_err(|error| format!("cannot create runtime temporary directory: {error}"))?;
    let host_path = temporary.path().join("paraco-deno-host.ts");
    std::fs::write(&host_path, HOST_SOURCE)
        .map_err(|error| format!("cannot write Deno host adapter: {error}"))?;

    let nonce = readiness_nonce();
    let interrupt = install_interrupt_handler()?;
    let (ready_tx, ready_rx) = mpsc::channel();
    let entrypoint_url = url::Url::from_file_path(&app.entrypoint)
        .map_err(|_| format!("cannot create a file URL for {}", app.entrypoint.display()))?;
    let mut command = Command::new("deno");
    command
        .arg("run")
        .arg("--quiet")
        .arg("--no-prompt")
        .arg(format!("--allow-read={}", app_dir.display()))
        .arg(format!("--allow-net=127.0.0.1:{port}"));
    command
        .arg(&host_path)
        .arg(entrypoint_url.as_str())
        .arg(port.to_string())
        .arg(&nonce)
        .current_dir(app_dir)
        .env_clear()
        .env("DENO_DIR", temporary.path().join("deno-cache"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "Deno was not found on PATH; install Deno and try again".to_string()
        } else {
            format!("cannot launch Deno: {error}")
        }
    })?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "cannot capture Deno stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "cannot capture Deno stderr".to_string())?;
    let output_thread = forward_stdout(stdout, format!("PARACO_READY:{nonce}"), ready_tx);
    let error_thread = forward_stderr(stderr);

    if let Err(error) = wait_until_ready(&mut child, &ready_rx, &interrupt) {
        terminate_and_reap(&mut child);
        let _ = output_thread.join();
        let _ = error_thread.join();
        return Err(error);
    }
    println!("paraco: {} listening on http://127.0.0.1:{port}", app.name);

    let result = wait_for_exit_or_interrupt(&mut child, &interrupt);
    terminate_and_reap(&mut child);
    let _ = output_thread.join();
    let _ = error_thread.join();
    result
}

fn forward_stdout(
    stdout: impl std::io::Read + Send + 'static,
    marker: String,
    ready: mpsc::Sender<()>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) if line == marker => {
                    let _ = ready.send(());
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

fn install_interrupt_handler() -> Result<mpsc::Receiver<()>, String> {
    let (sender, receiver) = mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = sender.send(());
    })
    .map_err(|error| format!("cannot install Ctrl+C handler: {error}"))?;
    Ok(receiver)
}

fn wait_until_ready(
    child: &mut Child,
    ready: &mpsc::Receiver<()>,
    interrupt: &mpsc::Receiver<()>,
) -> Result<(), String> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        if ready.try_recv().is_ok() {
            return Ok(());
        }
        if interrupt.try_recv().is_ok() {
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
        if Instant::now() >= deadline {
            return Err(format!(
                "application did not start within {} seconds",
                STARTUP_TIMEOUT.as_secs()
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_exit_or_interrupt(
    child: &mut Child,
    interrupt: &mpsc::Receiver<()>,
) -> Result<(), String> {
    loop {
        if interrupt.try_recv().is_ok() {
            return Ok(());
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot check Deno process: {error}"))?
        {
            return Err(format!(
                "application exited unexpectedly with status {status}"
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
