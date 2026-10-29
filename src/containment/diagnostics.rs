//! Best-effort behavioral checks, not a proof of isolation.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{ExecutionSpec, WorkloadKind};
use serde_json::{Value, json};

const NAMES: [&str; 7] = [
    "control",
    "read",
    "write",
    "readonly",
    "network",
    "exec",
    "environment",
];

fn check(id: String, status: &str, detail: &str) -> Value {
    json!({"id": id, "status": status, "detail": detail})
}
fn unavailable(prefix: &str, detail: &str) -> Vec<Value> {
    let end = detail
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i <= 1024)
        .last()
        .unwrap_or(0);
    let detail = if detail.len() > 1024 {
        &detail[..end]
    } else {
        detail
    };
    NAMES
        .iter()
        .map(|name| check(format!("{prefix}-{name}"), "UNVERIFIED", detail))
        .collect()
}
fn denied(err: &io::Error) -> bool {
    matches!(
        err.raw_os_error(),
        Some(libc::EACCES | libc::EPERM | libc::ENOENT)
    )
}
fn access_result(result: io::Result<()>) -> &'static str {
    match result {
        Ok(()) => "FAIL",
        Err(e) if denied(&e) => "PASS",
        Err(_) => "UNVERIFIED",
    }
}

/// Internal entry point. Only operates on the paths and loopback address passed
/// by the parent diagnostic; writing the sentinel opens it without modifying it.
pub fn child() -> i32 {
    let args: Vec<_> = std::env::args_os().skip(2).collect();
    if args.len() != 5 {
        return 2;
    }
    let allowed = Path::new(&args[0]);
    let scratch = Path::new(&args[1]);
    let sentinel = Path::new(&args[2]);
    let addr: SocketAddr = match args[3].to_str().and_then(|s| s.parse().ok()) {
        Some(a) => a,
        None => return 2,
    };
    let secret_present_in_parent = args[4] == "1";
    let allowed_read = fs::read(allowed).is_ok_and(|v| v == b"paraco probe");
    let control = allowed_read && fs::write(scratch.join("written"), b"probe").is_ok();
    let readonly = if !allowed_read {
        "UNVERIFIED"
    } else {
        match OpenOptions::new().write(true).open(allowed) {
            Ok(_) => "FAIL",
            Err(e)
                if matches!(
                    e.raw_os_error(),
                    Some(libc::EACCES | libc::EPERM | libc::EROFS)
                ) =>
            {
                "PASS"
            }
            Err(_) => "UNVERIFIED",
        }
    };
    let read = access_result(File::open(sentinel).map(|_| ()));
    let write = access_result(OpenOptions::new().write(true).open(sentinel).map(|_| ()));
    let network = match TcpStream::connect_timeout(&addr, Duration::from_secs(1)) {
        Ok(_) => "FAIL",
        Err(e)
            if matches!(
                e.raw_os_error(),
                Some(libc::EACCES | libc::EPERM | libc::ENETUNREACH | libc::ECONNREFUSED)
            ) =>
        {
            "PASS"
        }
        Err(_) => "UNVERIFIED",
    };
    let exec = match Command::new("/bin/sh").args(["-c", "exit 0"]).status() {
        Ok(_) => "FAIL",
        Err(e) if denied(&e) => "PASS",
        Err(_) => "UNVERIFIED",
    };
    let environment = if std::env::var_os("PARACO_PROBE_SECRET").is_some() {
        "FAIL"
    } else if secret_present_in_parent {
        "PASS"
    } else {
        "UNVERIFIED"
    };
    println!(
        "{}",
        json!([
            if control { "PASS" } else { "FAIL" },
            read,
            write,
            readonly,
            network,
            exec,
            environment
        ])
    );
    0
}

fn run(
    kind: WorkloadKind,
    prefix: &str,
    executable: &Path,
    root: &Path,
    listener: &TcpListener,
) -> Vec<Value> {
    let dir = root.join(prefix);
    let scratch = dir.join("scratch");
    let allowed = dir.join("allowed");
    let sentinel = dir.join("sentinel");
    if fs::create_dir_all(&scratch).is_err()
        || fs::write(&allowed, b"paraco probe").is_err()
        || fs::write(&sentinel, b"sentinel").is_err()
    {
        return unavailable(prefix, "Could not prepare probe files.");
    }
    let allowed_before = fs::read(&allowed).is_ok_and(|v| v == b"paraco probe");
    let addr = match listener.local_addr() {
        Ok(a) => a,
        Err(_) => return unavailable(prefix, "Could not inspect loopback listener."),
    };
    if TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_err() {
        return unavailable(prefix, "Loopback positive control failed.");
    }
    let readable = [allowed.clone()];
    let writable = [scratch.clone()];
    let spec = ExecutionSpec {
        executable,
        readable: &readable,
        writable: &writable,
        cwd: &scratch,
        kind,
        capability_socket: None,
    };
    let mut cmd = match super::command(&spec) {
        Ok(c) => c,
        Err(e) => {
            return unavailable(
                prefix,
                &format!(
                    "Could not prepare the containment backend: {}",
                    redact(&e, root, executable)
                ),
            );
        }
    };
    let out_path = dir.join("output");
    let out = match File::create(&out_path) {
        Ok(f) => f,
        Err(_) => return unavailable(prefix, "Could not capture probe output."),
    };
    let err_path = dir.join("error");
    let err = match File::create(&err_path) {
        Ok(f) => f,
        Err(e) => return unavailable(prefix, &format!("Could not capture probe error: {e}")),
    };
    cmd.arg("__paraco_containment_probe")
        .arg(&allowed)
        .arg(&scratch)
        .arg(&sentinel)
        .arg(addr.to_string())
        .arg(if std::env::var_os("PARACO_PROBE_SECRET").is_some() {
            "1"
        } else {
            "0"
        })
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err));
    let mut process = match cmd.spawn() {
        Ok(p) => p,
        Err(e) => return unavailable(prefix, &format!("Could not start the contained probe: {e}")),
    };
    let start = Instant::now();
    let status = loop {
        match process.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = process.kill();
                let _ = process.wait();
                return unavailable(prefix, "Probe failed or timed out.");
            }
        }
    };
    if !status.success() {
        let mut error = String::new();
        let _ = File::open(err_path).and_then(|f| f.take(4096).read_to_string(&mut error));
        let diagnostic = startup_failure(process.id(), &status.to_string(), &error);
        return unavailable(prefix, &redact(&diagnostic, root, executable));
    }
    let mut output = String::new();
    if File::open(out_path)
        .and_then(|f| f.take(16 * 1024).read_to_string(&mut output))
        .is_err()
    {
        return unavailable(prefix, "Could not read probe results.");
    }
    let results: Vec<String> = match serde_json::from_str(&output) {
        Ok(r) => r,
        Err(_) => return unavailable(prefix, "Probe results were invalid."),
    };
    if results.len() != NAMES.len()
        || results
            .iter()
            .any(|s| !["PASS", "FAIL", "UNVERIFIED"].contains(&s.as_str()))
    {
        return unavailable(prefix, "Probe results were invalid.");
    }
    let allowed_ok = allowed_before && fs::read(&allowed).is_ok_and(|v| v == b"paraco probe");
    let sentinel_ok = fs::read(&sentinel).is_ok_and(|v| v == b"sentinel");
    let network_ok = TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_ok();
    NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let status = if ((*name == "read" || *name == "write") && !sentinel_ok)
                || (*name == "readonly" && !allowed_ok)
                || (*name == "network" && !network_ok)
            {
                "UNVERIFIED"
            } else {
                &results[i]
            };
            check(
                format!("{prefix}-{name}"),
                status,
                match *name {
                    "control" => "Allowed file read and scratch write.",
                    "read" => "Read outside the allowed paths.",
                    "readonly" => "Open an allowed read-only file for writing.",
                    "write" => "Open for writing outside the allowed paths.",
                    "network" => "Connect to a host loopback listener.",
                    "exec" => "Launch /bin/sh (other executable paths are not tested).",
                    _ => {
                        "Inherited probe environment variable; requires it to be set in the parent."
                    }
                },
            )
        })
        .collect()
}

fn startup_failure(pid: u32, status: &str, stderr: &str) -> String {
    let stderr = stderr.trim();
    let detail = if stderr.is_empty() {
        "No stderr was captured. On macOS, inspect the matching process crash report in ~/Library/Logs/DiagnosticReports and the sandbox denial logs for this PID."
    } else {
        stderr
    };
    format!("Contained probe PID {pid} exited {status} before completing its checks. {detail}")
}

pub fn verify() -> Value {
    let mut checks = Vec::new();
    let unsafe_mode =
        std::env::var_os("PARACO_ALLOW_UNSANDBOXED").as_deref() == Some(std::ffi::OsStr::new("1"));
    checks.push(check(
        "containment-mode".into(),
        if unsafe_mode { "FAIL" } else { "PASS" },
        if unsafe_mode {
            "Unsandboxed development mode is enabled."
        } else {
            "Unsandboxed development mode is not enabled."
        },
    ));
    let setup = tempfile::tempdir().and_then(|dir| {
        let executable = std::env::current_exe()?.canonicalize()?;
        let root = dir.path().canonicalize()?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        Ok((dir, root, executable, listener))
    });
    match setup {
        Ok((_dir, root, executable, listener)) => {
            checks.extend(run(
                WorkloadKind::Runtime,
                "native",
                &executable,
                &root,
                &listener,
            ));
            checks.extend(run(
                WorkloadKind::Compiler,
                "compiler",
                &executable,
                &root,
                &listener,
            ));
        }
        Err(_) => {
            checks.extend(unavailable("native", "Could not prepare the diagnostic."));
            checks.extend(unavailable("compiler", "Could not prepare the diagnostic."));
        }
    }
    report(checks)
}

fn redact(message: &str, root: &Path, executable: &Path) -> String {
    message
        .replace(root.to_string_lossy().as_ref(), "<probe directory>")
        .replace(executable.to_string_lossy().as_ref(), "<executable>")
}

fn report(checks: Vec<Value>) -> Value {
    let status = if checks.iter().any(|c| c["status"] == "FAIL") {
        "FAIL"
    } else if checks.iter().all(|c| c["status"] == "PASS") {
        "PASS"
    } else {
        "UNVERIFIED"
    };
    json!({"schemaVersion": 1, "backend": super::backend_name(), "status": status, "checks": checks, "limitations": ["Behavioral probes are not proof of isolation; they cover only the resources tested.", "Resource quotas and denial-of-service resistance are not tested.", "These canaries do not exercise the workerd listener or capability socket; run the runtime security suite too.", "Blocking /bin/sh does not establish a general ban on subprocesses.", "Environment clearing requires a canary in the controller process."]})
}

fn controller_failure(detail: &str) -> Value {
    let mut checks = vec![check(
        "containment-mode".into(),
        if std::env::var_os("PARACO_ALLOW_UNSANDBOXED").as_deref()
            == Some(std::ffi::OsStr::new("1"))
        {
            "FAIL"
        } else {
            "UNVERIFIED"
        },
        detail,
    )];
    checks.extend(unavailable("native", detail));
    checks.extend(unavailable("compiler", detail));
    report(checks)
}

fn valid_checks(checks: &[Value]) -> bool {
    let mut expected = std::collections::HashSet::from(["containment-mode".to_owned()]);
    for prefix in ["native", "compiler"] {
        expected.extend(NAMES.iter().map(|name| format!("{prefix}-{name}")));
    }
    if checks.len() != expected.len() {
        return false;
    }
    for c in checks {
        let Some(id) = c["id"].as_str() else {
            return false;
        };
        if !expected.remove(id)
            || !c["detail"].is_string()
            || !["PASS", "FAIL", "UNVERIFIED"]
                .iter()
                .any(|s| c["status"] == *s)
        {
            return false;
        }
    }
    expected.is_empty()
}

/// Launch a controller with a known environment canary without modifying this
/// process's environment. The controller invokes `verify` through the CLI.
pub fn verify_with_canary() -> Value {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            return controller_failure(&format!("Could not prepare diagnostic controller: {e}"));
        }
    };
    let executable = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => return controller_failure(&format!("Could not find diagnostic controller: {e}")),
    };
    let path = dir.path().join("output");
    let output = match File::create(&path) {
        Ok(f) => f,
        Err(e) => return controller_failure(&format!("Could not capture controller output: {e}")),
    };
    let mut child = match Command::new(executable)
        .arg("__paraco_containment_check")
        .env("PARACO_PROBE_SECRET", "paraco-probe-canary")
        .stdin(Stdio::null())
        .stdout(Stdio::from(output))
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return controller_failure(&format!("Could not start diagnostic controller: {e}"));
        }
    };
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() < Duration::from_secs(15) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return controller_failure("Diagnostic controller failed or timed out.");
            }
        }
    };
    if !status.success() {
        return controller_failure(&format!("Diagnostic controller exited {status}."));
    }
    let mut content = String::new();
    if File::open(path)
        .and_then(|f| f.take(65536).read_to_string(&mut content))
        .is_err()
    {
        return controller_failure("Could not read diagnostic controller output.");
    }
    match serde_json::from_str::<Value>(&content) {
        Ok(value)
            if value["schemaVersion"] == 1
                && value["backend"].is_string()
                && value["checks"].as_array().is_some_and(|a| valid_checks(a)) =>
        {
            report(value["checks"].as_array().unwrap().clone())
        }
        _ => controller_failure("Diagnostic controller returned invalid output."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_abort_stays_unverified_and_identifies_the_process() {
        let detail = startup_failure(1234, "signal: 6 (SIGABRT)", "");
        assert!(detail.contains("PID 1234"));
        assert!(detail.contains("No stderr was captured"));
        assert!(detail.contains("DiagnosticReports"));
        let checks = unavailable("native", &detail);
        assert!(checks.iter().all(|check| check["status"] == "UNVERIFIED"));
        let with_error = startup_failure(1234, "exit status: 1", " loader error\n");
        assert!(with_error.ends_with("loader error"));
        assert!(!with_error.contains("No stderr"));
    }
    #[test]
    fn unavailable_results_do_not_expose_paths() {
        let results = unavailable("native", "Could not start the contained probe.");
        assert_eq!(results.len(), 7);
        assert!(results.iter().all(|v| v["status"] == "UNVERIFIED"));
        assert!(
            !json!(results)
                .to_string()
                .contains(std::env::temp_dir().to_string_lossy().as_ref())
        );
    }
    #[test]
    fn controller_requires_exact_unique_check_ids() {
        let checks = controller_failure("unavailable")["checks"]
            .as_array()
            .unwrap()
            .clone();
        assert!(valid_checks(&checks));
        let mut duplicate = checks.clone();
        duplicate[1]["id"] = duplicate[0]["id"].clone();
        assert!(!valid_checks(&duplicate));
        let mut unknown = checks;
        unknown[1]["id"] = json!("unknown");
        assert!(!valid_checks(&unknown));
    }
}
