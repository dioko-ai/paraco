//! A separate process owns and reaps Deno when the main runtime disappears.
//! Its stdin is a bootstrap line followed by a lifetime pipe (EOF means stop).
//! No JavaScript cooperation or persisted process identifiers are required.
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
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
        // Keep ownership until Deno is reaped, but never pass it into Deno.
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
    let deno_args: Vec<_> = args.collect();
    // The last three arguments belong to the embedded host adapter; preceding
    // arguments are Deno options selected by the runtime.
    let split = deno_args
        .len()
        .checked_sub(3)
        .ok_or("missing adapter arguments")?;
    let (deno_options, adapter_args) = deno_args.split_at(split);
    let nonce = adapter_args[2].to_str().ok_or("invalid readiness nonce")?;
    let temporary = tempfile::tempdir().map_err(|e| format!("cannot create app directory: {e}"))?;
    let host_path = temporary.path().join("paraco-deno-host.ts");
    std::fs::write(&host_path, include_str!("../runtime/deno_host.ts"))
        .map_err(|e| format!("cannot write Deno host adapter: {e}"))?;
    let deno = std::env::var_os("PARACO_GUARDIAN_DENO")
        .map(std::path::PathBuf::from)
        .ok_or("missing pinned guardian Deno runtime")?;
    if !deno.is_absolute() || !deno.is_file() {
        return Err("invalid pinned guardian Deno runtime".into());
    }
    let cache = std::env::var_os("PARACO_GUARDIAN_DENO_DIR")
        .map(std::path::PathBuf::from)
        .ok_or("missing pinned guardian Deno cache")?;
    let mut child = AppChild(
        Command::new(deno)
            .args(deno_options)
            .arg(&host_path)
            .args(adapter_args)
            .env_remove("TMPDIR")
            .env("DENO_DIR", cache)
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    "Deno was not found on PATH; install Deno and try again".to_owned()
                } else {
                    format!("cannot launch Deno: {error}")
                }
            })?,
    );
    // Deno cannot import the app before receiving its bootstrap, so PID metadata
    // reaches the runtime before application output.
    writeln!(
        std::io::stdout(),
        "PARACO_READY:{nonce}:PID:{}",
        child.0.id()
    )
    .map_err(|e| format!("cannot report application PID: {e}"))?;
    child
        .0
        .stdin
        .take()
        .unwrap()
        .write_all(&bootstrap)
        .map_err(|e| format!("cannot initialize Deno: {e}"))?;
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
