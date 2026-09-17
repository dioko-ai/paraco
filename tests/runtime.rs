//! Real CLI/Deno acceptance tests. Unix signals and process groups are intentional.
#![cfg(unix)]

use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const LIMIT: Duration = Duration::from_secs(15);

struct App {
    child: Child,
    dir: TempDir,
    port: u16,
}

impl App {
    fn start(source: &str, port: u16, manifest: &str, missing_deno: bool) -> Self {
        if !missing_deno {
            assert!(
                Command::new("deno").arg("--version").output().is_ok(),
                "runtime integration tests require Deno on PATH"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.ts"), source).unwrap();
        fs::write(dir.path().join("paraco.json"), manifest).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_paraco"));
        command
            .args([
                "run",
                dir.path().to_str().unwrap(),
                "--port",
                &port.to_string(),
            ])
            .stdout(File::create(dir.path().join("stdout")).unwrap())
            .stderr(File::create(dir.path().join("stderr")).unwrap())
            .stdin(Stdio::null())
            .process_group(0);
        if missing_deno {
            command.env("PATH", dir.path());
        }
        let child = command.spawn().unwrap();
        Self { child, dir, port }
    }

    fn run(source: &str) -> Self {
        Self::start(source, free_port(), MANIFEST, false)
    }

    fn logs(&self) -> String {
        format!(
            "{}\n{}",
            self.stdout(),
            fs::read_to_string(self.dir.path().join("stderr")).unwrap()
        )
    }

    fn stdout(&self) -> String {
        fs::read_to_string(self.dir.path().join("stdout")).unwrap()
    }

    fn ready(&mut self) {
        let deadline = Instant::now() + LIMIT;
        loop {
            if self.stdout().contains("listening on http://") {
                return;
            }
            assert!(self.child.try_wait().unwrap().is_none(), "{}", self.logs());
            assert!(
                Instant::now() < deadline,
                "readiness timed out: {}",
                self.logs()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait(&mut self) -> ExitStatus {
        let deadline = Instant::now() + LIMIT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "exit timed out: {}", self.logs());
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn request(&self, path: &str) -> String {
        let mut stream = TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", self.port).parse().unwrap(),
            Duration::from_secs(2),
        )
        .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    fn interrupt(&self) {
        assert_eq!(
            unsafe { libc::kill(self.child.id() as i32, libc::SIGINT) },
            0
        );
    }

    fn assert_released(&self, pid: i32) {
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "Deno still exists");
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        TcpListener::bind(("127.0.0.1", self.port)).expect("app port still occupied");
    }

    fn pid(&self) -> i32 {
        self.request("/pid")
            .split("\r\n\r\n")
            .nth(1)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Kill the whole isolated test group, including Deno, even after a panic.
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}

const MANIFEST: &str = r#"{"name":"test","entrypoint":"main.ts","capabilities":[]}"#;
const HEALTHY: &str = r#"
console.log("app stdout");
console.error("app stderr");
export default { fetch(request) {
  return new Response(new URL(request.url).pathname === "/pid" ? String(Deno.pid) : "Hello from Paraco");
}};
"#;

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn serves_http_forwards_logs_and_reaps_on_ctrl_c() {
    let mut app = App::run(HEALTHY);
    app.ready();
    let response = app.request("/");
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.ends_with("Hello from Paraco"));
    assert!(app.logs().contains("app stdout"));
    assert!(app.logs().contains("app stderr"));
    let pid = app.pid();
    app.interrupt();
    assert!(app.wait().success(), "{}", app.logs());
    app.assert_released(pid);
}

#[test]
fn missing_deno_is_actionable() {
    let mut app = App::start(HEALTHY, free_port(), MANIFEST, true);
    assert!(!app.wait().success());
    assert!(app.logs().contains("Deno was not found on PATH"));
}

#[test]
fn invalid_manifest_fails_before_launch() {
    let mut app = App::start(HEALTHY, free_port(), "{}", true);
    assert!(!app.wait().success());
    assert!(app.logs().contains("invalid manifest"));
    assert!(!app.logs().contains("Deno was not found"));
}

#[test]
fn invalid_export_and_import_fail_without_readiness() {
    for source in ["export default {};", "import './missing.ts';"] {
        let mut app = App::run(source);
        assert!(!app.wait().success());
        assert!(app.logs().contains("application exited during startup"));
        assert!(!app.stdout().contains("listening on http://"));
    }
}

#[test]
fn occupied_port_fails_without_readiness() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let mut app = App::start(
        HEALTHY,
        listener.local_addr().unwrap().port(),
        MANIFEST,
        false,
    );
    assert!(!app.wait().success());
    assert!(app.logs().contains("application exited during startup"));
    assert!(!app.stdout().contains("listening on http://"));
}

#[test]
fn request_errors_do_not_break_subsequent_requests() {
    let mut app = App::run(
        r#"export default { fetch(request) {
      switch (new URL(request.url).pathname) {
        case "/throw": throw new Error("test failure");
        case "/invalid": return "not a response";
        default: return new Response("healthy");
      }
    }};"#,
    );
    app.ready();
    for path in ["/throw", "/invalid"] {
        let response = app.request(path);
        assert!(response.starts_with("HTTP/1.1 500"));
        assert!(response.ends_with("Internal Server Error"));
        assert!(app.request("/").ends_with("healthy"));
    }
    app.interrupt();
    assert!(app.wait().success());
}

#[test]
fn unexpected_exit_is_reported_and_reaped() {
    let mut app = App::run(HEALTHY);
    app.ready();
    let pid = app.pid();
    assert_eq!(unsafe { libc::kill(pid, libc::SIGKILL) }, 0);
    assert!(!app.wait().success());
    assert!(app.logs().contains("application exited unexpectedly"));
    app.assert_released(pid);
}

#[test]
fn stubborn_app_is_forced_to_exit_after_grace_period() {
    let mut app = App::run(&format!("setInterval(() => {{}}, 1000);\n{HEALTHY}"));
    app.ready();
    let pid = app.pid();
    let start = Instant::now();
    app.interrupt();
    assert!(app.wait().success());
    assert!(
        start.elapsed() >= Duration::from_secs(5),
        "forced shutdown path was not exercised"
    );
    assert!(start.elapsed() < Duration::from_secs(10));
    app.assert_released(pid);
}

#[test]
fn startup_timeout_reaps_an_app_that_never_exports() {
    let mut app = App::run(
        "console.log(`PID:${Deno.pid}`); setInterval(() => {}, 1000); await new Promise(() => {});",
    );
    assert!(!app.wait().success());
    assert!(
        app.logs()
            .contains("application did not start within 10 seconds"),
        "{}",
        app.logs()
    );
    let pid = app
        .stdout()
        .lines()
        .find_map(|line| line.strip_prefix("PID:"))
        .unwrap()
        .parse()
        .unwrap();
    app.assert_released(pid);
}

#[test]
fn repository_hello_example_serves_expected_response() {
    let mut app = App::start(
        include_str!("../examples/hello/main.ts"),
        free_port(),
        include_str!("../examples/hello/paraco.json"),
        false,
    );
    app.ready();
    let response = app.request("/");
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.ends_with("Hello from Paraco"));
    app.interrupt();
    assert!(app.wait().success());
}
