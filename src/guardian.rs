//! A separate process owns and reaps workerd when the main runtime disappears.
//! Its stdin is a bootstrap line followed by a lifetime pipe (EOF means stop).
//! No JavaScript cooperation or persisted process identifiers are required.
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command},
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

/// The host writes one JSON bootstrap record, then retains stdin as its lease.
pub const BOOTSTRAP_DELIMITER: u8 = b'\n';

#[allow(dead_code)] // Used by the Unix guardian launch path; absent on other targets.
pub fn inherit_lease(command: &mut Command, lease: Option<&File>) -> Result<(), String> {
    #[cfg(unix)]
    if let Some(lease) = lease {
        use std::os::{fd::AsRawFd, unix::process::CommandExt};
        let fd = lease.as_raw_fd();
        command.arg(fd.to_string());
        // Only the forked guardian inherits the lease. The parent's descriptor
        // stays CLOEXEC, so other concurrently launched programs cannot inherit it.
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        return Ok(());
    }
    let _ = lease;
    command.arg("-");
    Ok(())
}

pub fn run() -> i32 {
    match supervise() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("paraco: {error}");
            1
        }
    }
}

// Both host and guardian own cleanup: either survivor can remove launch files.
struct LaunchDirectory(std::path::PathBuf);
impl Drop for LaunchDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct AppChild(Child);
impl Drop for AppChild {
    fn drop(&mut self) {
        if let Ok(Some(_)) = self.0.try_wait() {
            return;
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(self.0.id() as i32, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        let _ = self.0.kill();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match self.0.try_wait() {
                Ok(Some(_)) => return,
                Err(_) => break,
                Ok(None) => thread::sleep(Duration::from_millis(25)),
            }
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn supervise() -> Result<i32, String> {
    let mut args = std::env::args_os().skip(2);
    let lease = args.next().ok_or("missing guardian lease")?;
    #[cfg(not(unix))]
    let _ = lease;
    #[cfg(unix)]
    let _lease = if lease != "-" {
        use std::os::fd::FromRawFd;
        let fd: i32 = lease
            .to_str()
            .ok_or("invalid lease")?
            .parse()
            .map_err(|_| "invalid lease")?;
        if fd < 3 {
            return Err("invalid lease descriptor".into());
        }
        // Keep ownership until workerd is reaped, but never pass it into workerd.
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            return Err("cannot protect guardian lease".into());
        }
        Some(unsafe { File::from_raw_fd(fd) })
    } else {
        None
    };
    let stop = crate::runner::install_interrupt_handler()?;
    let mut input = BufReader::new(std::io::stdin());
    let mut bootstrap = Vec::new();
    // Bound private bootstrap framing as well as detecting death during startup.
    let count = input
        .by_ref()
        .take(65537)
        .read_until(BOOTSTRAP_DELIMITER, &mut bootstrap)
        .map_err(|e| format!("cannot read host bootstrap: {e}"))?;
    if count > 65536 || bootstrap.last() != Some(&BOOTSTRAP_DELIMITER) {
        return Err("incomplete or oversized host bootstrap".into());
    }
    let owner_gone = stop.clone();
    thread::spawn(move || {
        let mut byte = [0];
        loop {
            match input.read(&mut byte) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                _ => break,
            }
        }
        owner_gone.store(true, Ordering::Relaxed);
    });
    if stop.load(Ordering::Relaxed) {
        return Ok(0);
    }
    let dir = std::path::PathBuf::from(args.next().ok_or("missing worker directory")?);
    let _directory = LaunchDirectory(dir.clone());
    let port: u16 = args
        .next()
        .ok_or("missing port")?
        .to_string_lossy()
        .parse()
        .map_err(|_| "invalid port")?;
    let nonce = args
        .next()
        .ok_or("missing nonce")?
        .to_string_lossy()
        .to_string();
    let settings: serde_json::Value =
        serde_json::from_slice(&bootstrap).map_err(|e| e.to_string())?;
    let bridge = crate::bridge::Bridge::start(settings.clone(), &dir)?;
    let public = serde_json::json!({"basePath":settings["basePath"], "identity":settings["identity"], "backendToken":settings["backendToken"], "nonce":nonce, "hostToken":bridge.token,
        "containmentEvidence":settings["containmentEvidence"], "system":settings["system"].as_bool().unwrap_or(false), "ai":!settings["ai"].is_null(), "storage":!settings["storage"].is_null(), "tasks":!settings["tasks"].is_null()});
    std::fs::write(
        dir.join("adapter.js"),
        include_str!("../runtime/workerd_host.js"),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("platform-client.js"),
        include_str!("../runtime/platform-client.js"),
    )
    .map_err(|e| e.to_string())?;
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let config = crate::worker::config(&dir, port, Some(&bridge.address), &public)?;
    let runtime = std::env::var_os("PARACO_GUARDIAN_WORKERD").ok_or("missing workerd runtime")?;
    let mut command =
        crate::worker::launch_command(std::path::Path::new(&runtime), &dir, &config, &listener)?;
    let mut child = AppChild(
        command
            .spawn()
            .map_err(|e| format!("cannot launch workerd: {e}"))?,
    );
    drop(listener);
    writeln!(
        std::io::stdout(),
        "PARACO_READY:{nonce}:PID:{}",
        child.0.id()
    )
    .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(0);
        }
        if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
            return Err(format!("workerd exited: {status}"));
        }
        if let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", port)) {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
            let _ = write!(
                stream,
                "GET / HTTP/1.1\r\nHost: localhost\r\nx-paraco-ready: {nonce}\r\nConnection: close\r\n\r\n"
            );
            let mut bytes = [0; 128];
            if stream
                .read(&mut bytes)
                .is_ok_and(|n| String::from_utf8_lossy(&bytes[..n]).contains("200 OK"))
            {
                break;
            }
        }
        if Instant::now() > deadline {
            return Err("workerd readiness deadline exceeded".into());
        }
        thread::sleep(Duration::from_millis(25));
    }
    writeln!(std::io::stdout(), "PARACO_READY:{nonce}:{port}").map_err(|e| e.to_string())?;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(0);
        }
        if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
            return Ok(status.code().unwrap_or(1));
        }
        thread::sleep(Duration::from_millis(25));
    }
}
