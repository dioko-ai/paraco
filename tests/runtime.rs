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
    _log_dir: TempDir,
}

impl App {
    fn start(source: &str, port: u16, manifest: &str, missing_deno: bool) -> Self {
        Self::start_config(source, port, manifest, missing_deno, None)
    }

    fn start_config(
        source: &str,
        port: u16,
        manifest: &str,
        missing_deno: bool,
        config: Option<&std::path::Path>,
    ) -> Self {
        if !missing_deno {
            assert!(
                Command::new("deno").arg("--version").output().is_ok(),
                "runtime integration tests require Deno on PATH"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.ts"), source).unwrap();
        fs::write(dir.path().join("paraco.json"), manifest).unwrap();
        let log_dir = tempfile::tempdir().unwrap();
        if let Some(config) = config {
            let contents = fs::read_to_string(config).unwrap();
            if contents.contains("REPLACE_WITH_PARACO_IDENTITY_OUTPUT") {
                let identity = Command::new(env!("CARGO_BIN_EXE_paraco"))
                    .arg("--log-dir")
                    .arg(log_dir.path().join("logs"))
                    .arg("identity")
                    .arg(dir.path())
                    .output()
                    .unwrap();
                assert!(identity.status.success());
                fs::write(
                    config,
                    contents.replace(
                        "REPLACE_WITH_PARACO_IDENTITY_OUTPUT",
                        std::str::from_utf8(&identity.stdout).unwrap().trim(),
                    ),
                )
                .unwrap();
            }
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_paraco"));
        command.arg("--log-dir").arg(log_dir.path().join("logs"));
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
        if let Some(config) = config {
            command.arg("--ai-config").arg(config);
        }
        let child = command.spawn().unwrap();
        Self {
            child,
            dir,
            port,
            _log_dir: log_dir,
        }
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
fn forced_runtime_death_reaps_even_an_app_with_a_blocked_event_loop() {
    let mut app = App::run(&format!(
        "{HEALTHY}\nsetTimeout(() => {{ console.log('BLOCKED'); while (true) {{}} }}, 1000);"
    ));
    app.ready();
    let pid = app.pid();
    let deadline = Instant::now() + LIMIT;
    while !app.stdout().contains("BLOCKED") {
        assert!(Instant::now() < deadline, "{}", app.logs());
        thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(
        unsafe { libc::kill(app.child.id() as i32, libc::SIGKILL) },
        0
    );
    assert!(!app.wait().success());
    let deadline = Instant::now() + Duration::from_secs(8);
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(Instant::now() < deadline, "Deno survived runtime death");
        thread::sleep(Duration::from_millis(25));
    }
    app.assert_released(pid);
}

#[test]
fn forced_runtime_death_during_import_reaps_the_starting_app() {
    let mut app = App::run("console.log(`PID:${Deno.pid}`); while (true) {}");
    let deadline = Instant::now() + LIMIT;
    let pid: i32 = loop {
        if let Some(pid) = app
            .stdout()
            .lines()
            .find_map(|line| line.strip_prefix("PID:"))
        {
            break pid.parse().unwrap();
        }
        assert!(Instant::now() < deadline, "{}", app.logs());
        thread::sleep(Duration::from_millis(25));
    };
    assert_eq!(
        unsafe { libc::kill(app.child.id() as i32, libc::SIGKILL) },
        0
    );
    app.wait();
    let deadline = Instant::now() + Duration::from_secs(8);
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(
            Instant::now() < deadline,
            "starting Deno survived runtime death"
        );
        thread::sleep(Duration::from_millis(25));
    }
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

#[test]
fn prepare_accepts_resolved_contained_parent_imports_and_ordinary_source_text() {
    let deno = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join("deno"))
        .find(|candidate| candidate.is_file())
        .expect("runtime integration tests require Deno on PATH")
        .canonicalize()
        .unwrap();
    let source = tempfile::tempdir().unwrap();
    let nested = source.path().join("src");
    fs::create_dir(&nested).unwrap();
    fs::write(
        source.path().join("paraco.json"),
        r#"{"name":"prepared-graph","entrypoint":"src/main.ts","capabilities":[]}"#,
    )
    .unwrap();
    fs::write(
        source.path().join("util.ts"),
        "export const message = 'ok';",
    )
    .unwrap();
    fs::write(
        nested.join("main.ts"),
        "// Documentation example: ../assets\nimport { message } from '../util.ts';\nexport default { fetch(){ return new Response(message); } };",
    )
    .unwrap();
    let destination = tempfile::tempdir().unwrap();
    let artifact = destination.path().join("artifact");
    let status = Command::new(env!("CARGO_BIN_EXE_paraco"))
        .args(["prepare", source.path().to_str().unwrap(), "--output"])
        .arg(&artifact)
        .arg("--deno")
        .arg(deno)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(artifact.join("paraco-artifact.json").is_file());

    let port = free_port();
    let stdout = destination.path().join("stdout");
    let stderr = destination.path().join("stderr");
    let child = Command::new(env!("CARGO_BIN_EXE_paraco"))
        .args(["run-prepared", artifact.to_str().unwrap(), "--port"])
        .arg(port.to_string())
        .stdout(File::create(&stdout).unwrap())
        .stderr(File::create(&stderr).unwrap())
        .stdin(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let mut app = App {
        child,
        dir: destination,
        port,
        _log_dir: tempfile::tempdir().unwrap(),
    };
    app.ready();
    assert!(app.request("/").ends_with("ok"));
    app.interrupt();
    assert!(app.wait().success());
}

const AI_MANIFEST: &str = r#"{"name":"test","entrypoint":"main.ts","capabilities":["ai"]}"#;
const AI_APP: &str = r#"
export default { async fetch(request, context) {
  if (new URL(request.url).pathname === "/pid") return new Response(String(Deno.pid));
  try {
    const result = await context.ai.complete({prompt: "private prompt", provider: "fake", model: "small"});
    return Response.json(result);
  } catch (error) { return new Response(error.message, {status: 403}); }
}};
"#;

#[test]
fn ai_capability_works_end_to_end_with_explicit_host_grant() {
    let config = tempfile::tempdir().unwrap();
    let path = config.path().join("ai.json");
    fs::write(
        &path,
        r#"{
      "credentials":{"test-key":"fake"},
      "routes":[{"selection":{"provider":"fake","model":"small"},"credential":"test-key"}],
      "apps":{"REPLACE_WITH_PARACO_IDENTITY_OUTPUT":{"credential_grants":["test-key"]}}
    }"#,
    )
    .unwrap();
    let mut app = App::start_config(AI_APP, free_port(), AI_MANIFEST, false, Some(&path));
    app.ready();
    let response = app.request("/");
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("Fake AI response"));
    assert!(response.contains("fake") && response.contains("small"));
    for private in ["private prompt", "test-key"] {
        assert!(!response.contains(private));
        assert!(!app.logs().contains(private));
    }
    let pid = app.pid();
    app.interrupt();
    assert!(app.wait().success());
    app.assert_released(pid);

    // A grant for another identity never authorizes this launched app.
    let contents = fs::read_to_string(&path)
        .unwrap()
        .replace("\"test\":", "\"other\":");
    fs::write(&path, contents).unwrap();
    let mut denied = App::start_config(AI_APP, free_port(), AI_MANIFEST, false, Some(&path));
    denied.ready();
    let response = denied.request("/");
    assert!(response.starts_with("HTTP/1.1 403"));
    assert!(response.contains("no credential grant"));
    denied.interrupt();
    assert!(denied.wait().success());
}

#[test]
fn ai_request_without_host_configuration_does_not_grant_access() {
    let mut app = App::start(AI_APP, free_port(), AI_MANIFEST, false);
    app.ready();
    let response = app.request("/");
    assert!(response.starts_with("HTTP/1.1 403"));
    assert!(response.contains("no configured AI route"));
    app.interrupt();
    assert!(app.wait().success());
}

#[test]
fn app_without_ai_has_no_capability() {
    let mut app = App::run(
        r#"export default {fetch(_request, context) {return new Response(String(context.ai === undefined));}};"#,
    );
    app.ready();
    assert!(app.request("/").ends_with("true"));
    app.interrupt();
    assert!(app.wait().success());
}

#[test]
fn repository_ai_example_uses_runtime_default() {
    let config_dir = tempfile::tempdir().unwrap();
    let config = config_dir.path().join("ai-config.json");
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/ai-config.json"),
        &config,
    )
    .unwrap();
    let mut app = App::start_config(
        include_str!("../examples/ai/main.ts"),
        free_port(),
        include_str!("../examples/ai/paraco.json"),
        false,
        Some(&config),
    );
    app.ready();
    let response = app.request("/");
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("Fake AI response"));
    app.interrupt();
    assert!(app.wait().success());
}

#[test]
fn persistent_single_app_logs_bound_large_lines_and_preserve_invalid_utf8() {
    let mut app = App::run(&format!(
        "Deno.stdout.writeSync(new Uint8Array([255, 10])); console.log('x'.repeat(200000));\n{HEALTHY}"
    ));
    app.ready();
    assert!(app.request("/").contains("200 OK"));
    app.interrupt();
    assert!(app.wait().success());
    let output = fs::read_to_string(app._log_dir.path().join("logs/current.jsonl")).unwrap();
    let records: Vec<serde_json::Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(records.iter().any(|r| r["message"] == "�"));
    let large = records.iter().find(|r| r["truncated"] == true).unwrap();
    assert_eq!(large["message"].as_str().unwrap().len(), 16 * 1024);
    assert!(
        records
            .iter()
            .any(|r| r["event"] == "running" && r["mode"] == "run")
    );
    assert!(records.iter().any(|r| r["event"] == "stopped"));
    assert!(!output.contains("PARACO_READY:"));
}
