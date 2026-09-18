//! Multi-app acceptance tests use actual Deno processes and the public gateway.
#![cfg(unix)]
use std::{
    fs::{self, File},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

const APP: &str = r#"
export default {async fetch(request, context) {
  const url = new URL(request.url);
  switch (url.pathname) {
    case "/": return new Response(`<link rel="stylesheet" href="style.css"><a href="${context.basePath}next">Next</a>`);
    case "/style.css": return new Response("body { color: green; }", {headers: {"content-type": "text/css"}});
    case "/redirect": return new Response(null, {status: 302, headers: {location: `${context.basePath}next?from=redirect`}});
    case "/relative": return new Response(null, {status: 302, headers: {location: "next"}});
    case "/next": return new Response("arrived");
    case "/inspect": return Response.json({path: url.pathname, query: url.search, base: context.basePath, body: await request.text(), method: request.method, host: url.host, pid: Deno.pid, header: request.headers.get("x-test"), forwarded: request.headers.get("x-forwarded-host")});
    case "/hang": return await new Promise(() => {});
    case "/throw": throw new Error("request failure");
    default: return new Response("app missing", {status: 404});
  }
}};
"#;

struct Server {
    child: Child,
    dir: TempDir,
    port: u16,
}
impl Server {
    fn fixture(apps: &[(&str, &str)]) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        let entries: Vec<_> = apps
            .iter()
            .map(|(name, source)| {
                let app = dir.path().join(name);
                fs::create_dir(&app).unwrap();
                fs::write(app.join("main.ts"), source).unwrap();
                fs::write(
                    app.join("paraco.json"),
                    serde_json::json!({"name": name, "entrypoint": "main.ts", "capabilities": []})
                        .to_string(),
                )
                .unwrap();
                serde_json::json!({"path": name})
            })
            .collect();
        fs::write(
            dir.path().join("server.json"),
            serde_json::json!({"apps": entries}).to_string(),
        )
        .unwrap();
        dir
    }
    fn start(dir: TempDir, config: &Path, port: u16) -> Self {
        let logs = dir.path().join("logs");
        Self::start_with_logs(dir, config, port, &logs)
    }
    fn start_with_logs(dir: TempDir, config: &Path, port: u16, logs: &Path) -> Self {
        assert!(
            Command::new("deno")
                .arg("--version")
                .output()
                .unwrap()
                .status
                .success(),
            "Deno is required"
        );
        let child = Command::new(env!("CARGO_BIN_EXE_paraco"))
            .arg("--log-dir")
            .arg(logs)
            .arg("serve")
            .arg("--config")
            .arg(config)
            .arg("--port")
            .arg(port.to_string())
            .current_dir("/")
            // These must never redirect local upstream traffic through a proxy.
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("ALL_PROXY", "http://127.0.0.1:1")
            .stdin(Stdio::null())
            .stdout(File::create(dir.path().join("stdout")).unwrap())
            .stderr(File::create(dir.path().join("stderr")).unwrap())
            .process_group(0)
            .spawn()
            .unwrap();
        Self { child, dir, port }
    }
    fn run(apps: &[(&str, &str)]) -> Self {
        let dir = Self::fixture(apps);
        let path = dir.path().join("server.json");
        Self::start(dir, &path, free_port())
    }
    fn logs(&self) -> String {
        format!(
            "{}\n{}",
            fs::read_to_string(self.dir.path().join("stdout")).unwrap(),
            fs::read_to_string(self.dir.path().join("stderr")).unwrap()
        )
    }
    fn ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !self.logs().contains("dashboard listening") {
            assert!(self.child.try_wait().unwrap().is_none(), "{}", self.logs());
            assert!(Instant::now() < deadline, "{}", self.logs());
            thread::sleep(Duration::from_millis(20));
        }
    }
    fn request(&self, method: &str, path: &str, body: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(stream, "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1:{}\r\nContent-Length: {}\r\nX-Test: preserved\r\nX-Forwarded-Host: forged\r\nConnection: close\r\n\r\n{body}", self.port, body.len()).unwrap();
        let mut result = String::new();
        stream.read_to_string(&mut result).unwrap();
        result
    }
    fn wait_for(&self, path: &str, expected: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let response = self.request("GET", path, "");
            if response.contains(expected) {
                return response;
            }
            assert!(
                Instant::now() < deadline,
                "Expected {expected}: {response}\n{}",
                self.logs()
            );
            thread::sleep(Duration::from_millis(25));
        }
    }
    fn inspect(&self, name: &str) -> serde_json::Value {
        let response = self.wait_for(&format!("/apps/{name}/inspect"), "\"pid\"");
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
    }
    fn command(&self, action: &str, app: Option<&str>) -> std::process::Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_paraco"));
        command.arg(action).arg("--port").arg(self.port.to_string());
        if let Some(app) = app {
            command.arg(app);
        }
        command.output().unwrap()
    }
    fn control(&self, action: &str, app: Option<&str>) -> serde_json::Value {
        let output = self.command(action, app);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn state(&self, name: &str, expected: &str) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let apps = self.control("status", Some(name));
            if apps[0]["state"] == expected {
                return apps[0].clone();
            }
            assert!(
                Instant::now() < deadline,
                "Expected {expected}: {apps}\n{}",
                self.logs()
            );
            thread::sleep(Duration::from_millis(25));
        }
    }
    #[cfg(target_os = "linux")]
    fn capability_ports(&self) -> Vec<u16> {
        let inodes: Vec<_> = fs::read_dir(format!("/proc/{}/fd", self.child.id()))
            .unwrap()
            .filter_map(|entry| fs::read_link(entry.ok()?.path()).ok())
            .filter_map(|path| {
                path.to_str()?
                    .strip_prefix("socket:[")?
                    .strip_suffix(']')
                    .map(str::to_owned)
            })
            .collect();
        fs::read_to_string(format!("/proc/{}/net/tcp", self.child.id()))
            .unwrap()
            .lines()
            .skip(1)
            .filter_map(|line| {
                let fields: Vec<_> = line.split_whitespace().collect();
                if fields[3] != "0A" || !inodes.iter().any(|inode| inode == fields[9]) {
                    return None;
                }
                let port = u16::from_str_radix(fields[1].split_once(':')?.1, 16).ok()?;
                (port != self.port && port != self.management().0).then_some(port)
            })
            .collect()
    }
    fn management(&self) -> (u16, String) {
        let logs = self.logs();
        let url = logs
            .lines()
            .find_map(|line| line.strip_prefix("paraco: management dashboard "))
            .unwrap();
        let (address, token) = url.split_once("/#").unwrap();
        (
            address.rsplit_once(':').unwrap().1.parse().unwrap(),
            token.into(),
        )
    }
    fn browser_request(&self, method: &str, path: &str, headers: &str, body: &str) -> String {
        let port = self.management().0;
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(stream, "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}", body.len()).unwrap();
        let mut result = String::new();
        stream.read_to_string(&mut result).unwrap();
        result
    }
    fn socket_path(&self) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "paraco-control-{}/{}.sock",
            unsafe { libc::geteuid() },
            self.port
        ))
    }
    fn wait_exit(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "{}", self.logs());
            thread::sleep(Duration::from_millis(25));
        }
    }
    fn stop(&mut self, signal: i32) {
        assert_eq!(unsafe { libc::kill(self.child.id() as i32, signal) }, 0);
        assert!(self.wait_exit().success(), "{}", self.logs());
        TcpListener::bind(("127.0.0.1", self.port)).expect("gateway port not released");
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}
fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
fn assert_reaped(pid: i32) {
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "Deno {pid} still exists");
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}

#[test]
fn routes_two_apps_assets_bodies_queries_and_redirects() {
    let mut server = Server::run(&[("one", APP), ("two", APP)]);
    server.ready();
    let one = server.inspect("one");
    let two = server.inspect("two");
    assert_ne!(one["pid"], two["pid"]);
    assert_eq!(one["base"], "/apps/one/");
    assert_eq!(two["base"], "/apps/two/");
    let root = server.wait_for("/", "2 of 2 apps running");
    assert!(root.contains("href=\"/apps/one/\""));
    assert!(root.contains("href=\"/apps/two/\""));
    assert!(root.contains("no-store"));
    assert!(
        server
            .request("GET", "/apps/one/", "")
            .contains("href=\"style.css\"")
    );
    let css = server.request("GET", "/apps/one/style.css", "");
    assert!(css.contains("text/css") && css.ends_with("body { color: green; }"));
    let inspect = server.request("POST", "/apps/one/inspect?q=a%2Fb&x=2", "hello body");
    let value: serde_json::Value =
        serde_json::from_str(inspect.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(value["path"], "/inspect");
    assert_eq!(value["query"], "?q=a%2Fb&x=2");
    assert_eq!(value["body"], "hello body");
    assert_eq!(value["method"], "POST");
    assert_eq!(value["host"], format!("127.0.0.1:{}", server.port));
    assert_eq!(value["header"], "preserved");
    assert!(value["forwarded"].is_null());
    let redirect = server.request("GET", "/apps/one/redirect", "");
    assert!(redirect.contains("302"));
    assert!(redirect.contains("location: /apps/one/next?from=redirect"));
    assert!(
        server
            .request("GET", "/apps/one/next?from=redirect", "")
            .ends_with("arrived")
    );
    assert!(
        server
            .request("GET", "/apps/one/relative", "")
            .contains("location: next")
    );
    let slash = server.request("POST", "/apps/one?q=yes", "body");
    assert!(slash.contains("308") && slash.contains("location: /apps/one/?q=yes"));
    for path in [
        "/missing",
        "/apps/missing/",
        "/apps/one-more/",
        "/apps/one/missing",
    ] {
        assert!(server.request("GET", path, "").contains("404"));
    }
    assert!(server.request("POST", "/", "").contains("405"));
    assert!(server.request("HEAD", "/", "").ends_with("\r\n\r\n"));
    assert!(
        server
            .request("HEAD", "/apps/one/style.css", "")
            .ends_with("\r\n\r\n")
    );
    server.stop(libc::SIGINT);
    assert_reaped(one["pid"].as_i64().unwrap() as i32);
    assert_reaped(two["pid"].as_i64().unwrap() as i32);
}

#[test]
fn failures_and_hanging_requests_leave_other_apps_and_dashboard_available() {
    let mut server = Server::run(&[
        ("healthy", APP),
        ("crash", APP),
        ("broken", "export default {};"),
    ]);
    server.ready();
    let healthy = server.inspect("healthy");
    let crash = server.inspect("crash");
    server.wait_for("/", "application exited during startup");
    assert!(server.request("GET", "/apps/broken/", "").contains("503"));
    assert!(
        server
            .request("GET", "/apps/healthy/throw", "")
            .contains("500")
    );
    let crash_pid = crash["pid"].as_i64().unwrap() as i32;
    unsafe {
        libc::kill(crash_pid, libc::SIGKILL);
    }
    server.wait_for("/", "application exited unexpectedly");
    assert!(server.request("GET", "/apps/crash/", "").contains("503"));
    let mut hanging = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    write!(
        hanging,
        "GET /apps/healthy/hang HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
        server.port
    )
    .unwrap();
    assert!(
        server
            .request("GET", "/apps/healthy/next", "")
            .ends_with("arrived")
    );
    assert!(server.request("GET", "/", "").contains("Your local apps"));
    server.stop(libc::SIGTERM);
    assert_reaped(healthy["pid"].as_i64().unwrap() as i32);
    assert_reaped(crash_pid);
}

#[test]
fn rejects_duplicate_names_invalid_configuration_and_occupied_gateway() {
    for config in [
        r#"{"apps":[{"path":"one"},{"path":"one"}]}"#,
        r#"{"apps":[],"extra":true}"#,
    ] {
        let dir = Server::fixture(&[("one", APP)]);
        let path = dir.path().join("server.json");
        fs::write(&path, config).unwrap();
        let mut server = Server::start(dir, &path, free_port());
        assert!(!server.wait_exit().success());
        assert!(!server.logs().contains("dashboard listening"));
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let dir = Server::fixture(&[("one", APP)]);
    let path = dir.path().join("server.json");
    let mut server = Server::start(dir, &path, listener.local_addr().unwrap().port());
    assert!(!server.wait_exit().success());
    assert!(server.logs().contains("cannot bind gateway"));
}

#[test]
fn empty_dashboard_and_repository_ai_example() {
    let mut empty = Server::run(&[]);
    empty.ready();
    assert!(empty.request("GET", "/", "").contains("No apps configured"));
    empty.stop(libc::SIGINT);
    let dir = tempfile::tempdir().unwrap();
    let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/server.json");
    let mut server = Server::start(dir, &config, free_port());
    server.ready();
    server.wait_for("/apps/hello/", "Hello from Paraco");
    server.wait_for("/apps/ai-example/", "Fake AI response");
    server.wait_for("/", "2 of 2 apps running");
    server.stop(libc::SIGINT);
}

#[test]
fn shutdown_during_startup_reaps_every_child() {
    let source = "console.log(`CHILD:${Deno.pid}`); setInterval(() => {}, 1000); await new Promise(() => {});";
    let mut server = Server::run(&[("slow-one", source), ("slow-two", source)]);
    server.ready();
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.logs().matches("CHILD:").count() != 2 {
        assert!(Instant::now() < deadline, "{}", server.logs());
        thread::sleep(Duration::from_millis(25));
    }
    let pids: Vec<i32> = server
        .logs()
        .lines()
        .filter_map(|line| line.strip_prefix("CHILD:"))
        .map(|pid| pid.parse().unwrap())
        .collect();
    server.stop(libc::SIGINT);
    for pid in pids {
        assert_reaped(pid);
    }
}

#[test]
fn gateway_rejects_foreign_hosts_upgrades_and_oversized_bodies() {
    let mut server = Server::run(&[("one", APP)]);
    server.ready();
    server.inspect("one");
    for (host, extra, status) in [
        ("foreign.example".to_string(), "", "400"),
        (
            format!("127.0.0.1:{}", server.port),
            "Upgrade: websocket\r\n",
            "501",
        ),
    ] {
        let mut stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(
            stream,
            "GET /apps/one/ HTTP/1.0\r\nHost: {host}\r\n{extra}Connection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(
            response.lines().next().unwrap().contains(status),
            "{response}"
        );
    }
    let oversized = "x".repeat(1024 * 1024 + 1);
    assert!(
        server
            .request("POST", "/apps/one/inspect", &oversized)
            .contains("413")
    );
    assert!(
        server
            .request("GET", "/apps/one/next", "")
            .ends_with("arrived")
    );
    server.stop(libc::SIGINT);
}

#[test]
fn lifecycle_commands_isolate_apps_and_reap_replaced_processes() {
    let mut server = Server::run(&[("one", APP), ("two", APP)]);
    server.ready();
    let first = server.inspect("one")["pid"].as_i64().unwrap() as i32;
    let other = server.inspect("two")["pid"].clone();
    assert_eq!(server.control("status", None).as_array().unwrap().len(), 2);
    for _ in 0..3 {
        server.control("start", Some("one"));
    }
    assert_eq!(server.inspect("one")["pid"], first);
    for _ in 0..3 {
        server.control("stop", Some("one"));
    }
    assert_eq!(server.state("one", "stopped")["desired"], "stopped");
    assert_reaped(first);
    assert!(server.request("GET", "/apps/one/", "").contains("503"));
    assert!(server.request("GET", "/", "").contains("stopped"));
    assert_eq!(server.inspect("two")["pid"], other);
    thread::sleep(Duration::from_millis(150));
    assert_eq!(server.control("status", Some("one"))[0]["state"], "stopped");
    server.control("start", Some("one"));
    server.state("one", "running");
    let second = server.inspect("one")["pid"].as_i64().unwrap() as i32;
    assert_ne!(first, second);
    assert_eq!(server.inspect("one")["base"], "/apps/one/");
    server.control("restart", Some("one"));
    server.state("one", "running");
    assert_reaped(second);
    let third = server.inspect("one")["pid"].as_i64().unwrap() as i32;
    assert_ne!(second, third);
    assert_eq!(server.inspect("two")["pid"], other);
    let missing = server.command("start", Some("missing"));
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("unknown application"));
    server.stop(libc::SIGINT);
    assert_reaped(third);
    assert!(!server.socket_path().exists());
    assert!(!server.command("status", None).status.success());
}

#[test]
fn failed_apps_can_be_fixed_and_started_without_restarting_server() {
    let mut server = Server::run(&[("broken", "export default {};"), ("healthy", APP)]);
    server.ready();
    assert_eq!(server.state("broken", "failed")["desired"], "running");
    let healthy = server.inspect("healthy")["pid"].clone();
    fs::write(server.dir.path().join("broken/main.ts"), APP).unwrap();
    server.control("start", Some("broken"));
    server.state("broken", "running");
    let pid = server.inspect("broken")["pid"].as_i64().unwrap() as i32;
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
    let failed = server.state("broken", "failed");
    assert_eq!(failed["desired"], "running");
    assert!(failed["error"].as_str().unwrap().contains("unexpectedly"));
    assert_reaped(pid);
    server.control("restart", Some("broken"));
    server.state("broken", "running");
    assert_ne!(server.inspect("broken")["pid"], pid);
    assert_eq!(server.inspect("healthy")["pid"], healthy);
    server.stop(libc::SIGTERM);
}

#[test]
fn stop_cancels_startup_and_latest_command_wins_without_orphans() {
    let source = "console.log(`CHILD:${Deno.pid}`); setInterval(() => {}, 1000); await new Promise(() => {});";
    let mut server = Server::run(&[("slow", source), ("healthy", APP)]);
    server.ready();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !server.logs().contains("CHILD:") {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(25));
    }
    let start = Instant::now();
    server.control("stop", Some("slow"));
    server.state("slow", "stopped");
    assert!(start.elapsed() < Duration::from_secs(5));
    for action in ["start", "restart", "restart", "stop", "start", "stop"] {
        server.control(action, Some("slow"));
    }
    server.state("slow", "stopped");
    for pid in server
        .logs()
        .lines()
        .filter_map(|line| line.strip_prefix("CHILD:"))
    {
        assert_reaped(pid.parse().unwrap());
    }
    server.inspect("healthy");
    fs::write(server.dir.path().join("slow/main.ts"), APP).unwrap();
    server.control("start", Some("slow"));
    server.state("slow", "running");
    server.inspect("slow");
    server.stop(libc::SIGINT);
}

#[test]
fn control_is_private_bounded_and_not_exposed_over_http() {
    use std::os::unix::{fs::PermissionsExt, net::UnixStream};
    let mut server = Server::run(&[("one", APP)]);
    server.ready();
    server.state("one", "running");
    let path = server.socket_path();
    assert_eq!(
        fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    for request in [
        "not json\n".to_owned(),
        "x".repeat(4097),
        "{\"action\":\"stop\"}\n".to_owned(),
        "{\"action\":\"stop\",\"app\":\"one\",\"extra\":true}\n".to_owned(),
    ] {
        let mut stream = UnixStream::connect(&path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        assert!(reply.contains("error"), "{reply}");
    }
    // A client sending occasional bytes must not extend the absolute deadline.
    let mut trickling = UnixStream::connect(&path).unwrap();
    trickling
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut writer = trickling.try_clone().unwrap();
    let sending = thread::spawn(move || {
        for _ in 0..12 {
            if writer.write_all(b" ").is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
    });
    let start = Instant::now();
    let mut reply = String::new();
    trickling.read_to_string(&mut reply).unwrap();
    assert!(reply.contains("error"));
    assert!(start.elapsed() < Duration::from_secs(2));
    sending.join().unwrap();
    let _stalled = UnixStream::connect(&path).unwrap();
    assert_eq!(server.control("status", Some("one"))[0]["state"], "running");
    assert!(
        server
            .request("POST", "/control", r#"{"action":"stop","app":"one"}"#)
            .contains("404")
    );
    assert!(server.request("POST", "/", "").contains("405"));
    server.inspect("one");
    server.stop(libc::SIGINT);
}

#[test]
fn ai_capability_is_recreated_and_released_on_lifecycle_changes() {
    let dir = Server::fixture(&[("ai", APP)]);
    let app = dir.path().join("ai");
    fs::write(
        app.join("paraco.json"),
        r#"{"name":"ai","entrypoint":"main.ts","capabilities":["ai"]}"#,
    )
    .unwrap();
    fs::write(
        app.join("main.ts"),
        r#"
export default { async fetch(_request, context) {
 return Response.json({pid: Deno.pid, result: await context.ai.complete({prompt: "hello"})});
}};
"#,
    )
    .unwrap();
    fs::write(dir.path().join("ai.json"), r#"{"credentials":{"fake":"fake"},"routes":[{"selection":{"provider":"fake","model":"test"},"credential":"fake"}],"apps":{"ai":{"credential_grants":["fake"]}},"default":{"provider":"fake","model":"test"}}"#).unwrap();
    let config = dir.path().join("server.json");
    fs::write(&config, r#"{"apps":[{"path":"ai","aiConfig":"ai.json"}]}"#).unwrap();
    let mut server = Server::start(dir, &config, free_port());
    server.ready();
    let first = server.wait_for("/apps/ai/", "Fake AI response");
    let first: serde_json::Value =
        serde_json::from_str(first.split_once("\r\n\r\n").unwrap().1).unwrap();
    server.control("restart", Some("ai"));
    server.state("ai", "running");
    assert_reaped(first["pid"].as_i64().unwrap() as i32);
    server.wait_for("/apps/ai/", "Fake AI response");
    #[cfg(target_os = "linux")]
    let ports = server.capability_ports();
    server.control("stop", Some("ai"));
    server.state("ai", "stopped");
    #[cfg(target_os = "linux")]
    {
        assert_eq!(ports.len(), 1, "expected one host-owned AI listener");
        assert!(server.capability_ports().is_empty());
        for port in ports {
            TcpListener::bind(("127.0.0.1", port)).expect("AI listener not released");
        }
    }
    server.control("start", Some("ai"));
    server.wait_for("/apps/ai/", "Fake AI response");
    server.stop(libc::SIGTERM);
}

#[test]
fn slow_stop_keeps_control_and_other_apps_responsive() {
    let source = format!(
        "{APP}\nDeno.addSignalListener('SIGTERM', () => {{}}); setInterval(() => {{}}, 1000);"
    );
    let mut server = Server::run(&[("stubborn", &source), ("healthy", APP)]);
    server.ready();
    let pid = server.inspect("stubborn")["pid"].as_i64().unwrap() as i32;
    let start = Instant::now();
    server.control("stop", Some("stubborn"));
    let status = server.control("status", Some("stubborn"));
    assert_eq!(status[0]["state"], "stopping");
    assert_eq!(status[0]["desired"], "stopped");
    server.inspect("healthy");
    assert!(server.request("GET", "/", "").contains("stopping"));
    assert!(start.elapsed() < Duration::from_secs(3));
    server.state("stubborn", "stopped");
    assert_reaped(pid);
    assert!(start.elapsed() >= Duration::from_secs(5));
    server.stop(libc::SIGINT);
}

#[test]
fn occupied_control_socket_is_preserved_and_launches_no_apps() {
    use std::os::unix::net::UnixListener;
    let mut live = Server::run(&[]);
    live.ready();
    let port = free_port();
    let path = live
        .socket_path()
        .parent()
        .unwrap()
        .join(format!("{port}.sock"));
    let listener = UnixListener::bind(&path).unwrap();
    let dir = Server::fixture(&[("one", "console.log('UNEXPECTED_START'); export default {};")]);
    let config = dir.path().join("server.json");
    let mut conflicting = Server::start(dir, &config, port);
    assert!(!conflicting.wait_exit().success());
    assert!(conflicting.logs().contains("cannot bind control socket"));
    assert!(!conflicting.logs().contains("UNEXPECTED_START"));
    assert!(path.exists());
    drop(listener);
    fs::remove_file(path).unwrap();
    live.stop(libc::SIGINT);
}

#[test]
fn control_rejects_insecure_or_symlinked_directories() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join(format!("paraco-control-{}", unsafe { libc::geteuid() }));
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    for symlinked in [false, true] {
        if symlinked {
            fs::remove_dir(&path).unwrap();
            symlink(dir.path(), &path).unwrap();
        }
        let output = Command::new(env!("CARGO_BIN_EXE_paraco"))
            .arg("status")
            .env("TMPDIR", dir.path())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("control directory"));
    }
}

#[test]
fn browser_management_controls_apps_and_releases_listener() {
    let mut server = Server::run(&[("hello", APP), ("other", APP)]);
    server.ready();
    let old_pid = server.inspect("hello")["pid"].as_u64().unwrap() as i32;
    let other_pid = server.inspect("other")["pid"].clone();
    let (port, token) = server.management();
    assert_ne!(port, server.port);
    let headers = format!(
        "Authorization: Bearer {token}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\nSec-Fetch-Site: same-origin\r\n"
    );
    let page = server.browser_request("GET", "/", "", "");
    assert!(page.starts_with("HTTP/1.0 200"), "{page}");
    assert!(page.contains("dashboard.js"));
    assert!(page.contains("frame-ancestors 'none'"));
    assert!(page.contains("referrer-policy: no-referrer"));
    assert!(!page.contains(&token));
    assert!(!server.request("GET", "/", "").contains(&token));
    let script = server.browser_request("GET", "/dashboard.js", "", "");
    assert!(script.contains("text/javascript"));
    let status = server.browser_request("GET", "/api/apps", &headers, "");
    assert!(status.contains("\"desired\":\"running\""), "{status}");
    for (action, state) in [
        ("stop", "stopped"),
        ("start", "running"),
        ("restart", "running"),
    ] {
        let before = if action == "restart" {
            Some(server.inspect("hello")["pid"].clone())
        } else {
            None
        };
        let body = format!(r#"{{"action":"{action}","app":"hello"}}"#);
        let response = server.browser_request("POST", "/api/apps", &headers, &body);
        assert!(response.starts_with("HTTP/1.0 200"), "{response}");
        server.state("hello", state);
        if let Some(before) = before {
            assert_ne!(server.inspect("hello")["pid"], before);
            assert_reaped(before.as_u64().unwrap() as i32);
        }
        assert_eq!(server.inspect("other")["pid"], other_pid);
    }
    assert_reaped(old_pid);
    server.stop(libc::SIGTERM);
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    TcpListener::bind(("127.0.0.1", port)).unwrap();
}

#[test]
fn browser_management_rejects_unauthorized_cross_origin_and_invalid_commands() {
    let mut server = Server::run(&[("hello", APP)]);
    server.ready();
    let pid = server.inspect("hello")["pid"].clone();
    let (port, token) = server.management();
    let origin = format!("Origin: http://127.0.0.1:{port}\r\n");
    let auth = format!("Authorization: Bearer {token}\r\n");
    let valid = format!("{auth}{origin}Content-Type: application/json\r\n");
    let command = r#"{"action":"stop","app":"hello"}"#;
    for (headers, code) in [
        (String::new(), 401),
        (format!("{origin}Authorization: Bearer wrong\r\n"), 401),
        (auth.clone(), 403),
        (
            format!("{auth}Origin: http://127.0.0.1:{}\r\n", server.port),
            403,
        ),
        (format!("{auth}Origin: null\r\n"), 403),
        (format!("{valid}Sec-Fetch-Site: same-site\r\n"), 403),
        (format!("{auth}{origin}Content-Type: text/plain\r\n"), 415),
    ] {
        let response = server.browser_request("POST", "/api/apps", &headers, command);
        assert!(
            response.starts_with(&format!("HTTP/1.0 {code}")),
            "{response}"
        );
        assert!(!response.contains("access-control-allow-origin"));
    }
    assert!(
        server
            .browser_request("GET", "/api/apps", "", "")
            .starts_with("HTTP/1.0 401")
    );
    assert!(
        server
            .browser_request("OPTIONS", "/api/apps", &valid, "")
            .starts_with("HTTP/1.0 405")
    );
    for body in [
        "invalid",
        r#"{"action":"stop"}"#,
        r#"{"action":"stop","app":"missing"}"#,
        r#"{"action":"stop","app":"hello","extra":true}"#,
    ] {
        assert!(
            server
                .browser_request("POST", "/api/apps", &valid, body)
                .starts_with("HTTP/1.0 400")
        );
    }
    assert!(
        server
            .browser_request("POST", "/api/apps", &valid, &"x".repeat(4097))
            .starts_with("HTTP/1.0 413")
    );
    assert!(
        server
            .request("POST", "/api/apps", command)
            .starts_with("HTTP/1.0 404")
    );
    assert_eq!(server.inspect("hello")["pid"], pid);
    server.stop(libc::SIGTERM);
}

#[test]
fn persistent_logs_identify_apps_and_launches_and_survive_server_exit() {
    let source = format!("console.log('startup-out'); console.error('startup-error');\n{APP}");
    let mut server = Server::run(&[
        ("hello", &source),
        ("other", APP),
        ("broken", "throw new Error('startup-failure-detail');"),
    ]);
    server.ready();
    let first = server.inspect("hello")["pid"].as_u64().unwrap();
    server.state("broken", "failed");
    server.request("GET", "/apps/hello/throw", "");
    server.control("restart", Some("hello"));
    server.state("hello", "running");
    let second = server.inspect("hello")["pid"].as_u64().unwrap();
    server.stop(libc::SIGTERM);
    let output = Command::new(env!("CARGO_BIN_EXE_paraco"))
        .args(["logs", "hello", "--tail", "100", "--log-dir"])
        .arg(server.dir.path().join("logs"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let records: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        records
            .iter()
            .all(|r| r["app"] == "hello" && r["mode"] == "serve" && r["port"] == server.port)
    );
    assert!(
        records
            .iter()
            .any(|r| r["stream"] == "stdout" && r["message"] == "startup-out")
    );
    assert!(
        records
            .iter()
            .any(|r| r["stream"] == "stderr" && r["message"] == "startup-error")
    );
    assert!(
        records
            .iter()
            .any(|r| r["message"].as_str().unwrap().contains("request failure"))
    );
    let run_id = |pid| records.iter().find(|r| r["pid"] == pid).unwrap()["run_id"].clone();
    assert_ne!(run_id(first), run_id(second));
    assert!(
        records
            .iter()
            .any(|r| r["event"] == "stopped" && r["pid"] == second)
    );
    let raw = fs::read_to_string(server.dir.path().join("logs/current.jsonl")).unwrap();
    assert!(raw.contains("startup-failure-detail"));
    assert!(!raw.contains("PARACO_READY:"));
    assert!(!raw.contains(&server.management().1));
}

#[test]
fn independent_servers_share_one_persistent_log_store() {
    let logs = tempfile::tempdir().unwrap();
    let source = format!("for(let i=0;i<100;i++) console.log(`record-${{i}}`);\n{APP}");
    let start = || {
        let dir = Server::fixture(&[("hello", &source)]);
        let config = dir.path().join("server.json");
        Server::start_with_logs(dir, &config, free_port(), &logs.path().join("logs"))
    };
    let mut first = start();
    let mut second = start();
    first.ready();
    second.ready();
    first.inspect("hello");
    second.inspect("hello");
    first.stop(libc::SIGTERM);
    second.stop(libc::SIGTERM);
    let output = fs::read_to_string(logs.path().join("logs/current.jsonl")).unwrap();
    let records: Vec<serde_json::Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for port in [first.port, second.port] {
        let lines: Vec<_> = records
            .iter()
            .filter(|r| r["port"] == port && r["stream"] == "stdout")
            .collect();
        assert_eq!(lines.len(), 100);
        assert_eq!(lines.first().unwrap()["message"], "record-0");
        assert_eq!(lines.last().unwrap()["message"], "record-99");
    }
}

#[test]
fn dashboard_logs_are_authenticated_filtered_and_rotation_safe() {
    let mut server = Server::run(&[("one", APP), ("two", APP)]);
    server.ready();
    server.state("one", "running");
    let token = server.management().1;
    let auth = format!("Authorization: Bearer {token}\r\n");
    assert!(
        server
            .browser_request("GET", "/api/logs?app=one", "", "")
            .contains("401 Unauthorized")
    );
    let store = log_store::Store::open(&server.dir.path().join("logs")).unwrap();
    let launch = store.app("one", "serve", server.port).unwrap();
    launch.event("output", "<script>hostile</script>");
    let records = store.tail(Some("one"), Some(server.port), 1).unwrap();
    let id = &records[0].run_id;
    let path = format!("/api/logs?app=one&run_id={id}&limit=2");
    let response = server.browser_request("GET", &path, &auth, "");
    assert!(response.contains("hostile"));
    assert!(
        !server
            .browser_request("GET", &format!("/api/logs?app=two&run_id={id}"), &auth, "")
            .contains("hostile")
    );
    assert!(
        server
            .browser_request("GET", "/api/logs?app=one&limit=501", &auth, "")
            .contains("400 Bad Request")
    );
    for _ in 0..150 {
        launch.event("output", &"x".repeat(16000));
    }
    launch.event("output", "after rotation");
    assert!(server.dir.path().join("logs/archive-1.jsonl").exists());
    assert!(
        server
            .browser_request("GET", &path, &auth, "")
            .contains("after rotation")
    );
}

#[allow(dead_code)]
#[path = "../src/logs.rs"]
mod log_store;

#[test]
fn automatic_recovery_is_bounded_cancellable_and_isolated() {
    let crash = "setTimeout(() => Deno.exit(9), 300); export default {fetch() {return new Response('up');}};";
    let dir = Server::fixture(&[("crash", crash), ("healthy", APP)]);
    let config = dir.path().join("server.json");
    fs::write(&config, r#"{"apps":[{"path":"crash","restart":{"onFailure":true,"maxRetries":2,"backoffMs":500,"maxBackoffMs":1000}},{"path":"healthy"}]}"#).unwrap();
    let mut server = Server::start(dir, &config, free_port());
    server.ready();
    server.state("crash", "backoff");
    server.wait_for("/apps/healthy/next", "arrived");
    // Correct the app before its scheduled retry, then confirm recovery.
    fs::write(server.dir.path().join("crash/main.ts"), APP).unwrap();
    server.state("crash", "running");
    server.wait_for("/apps/crash/next", "arrived");
    // A manual restart resets the budget; repeated crashes exhaust it.
    fs::write(server.dir.path().join("crash/main.ts"), crash).unwrap();
    server.control("restart", Some("crash"));
    let failed = server.state("crash", "failed");
    assert!(
        failed["error"]
            .as_str()
            .unwrap()
            .contains("retry limit reached (2/2)")
    );
    let store = log_store::Store::open(&server.dir.path().join("logs")).unwrap();
    let launches = || {
        store
            .tail(Some("crash"), None, 1000)
            .unwrap()
            .iter()
            .filter(|r| r.event == "starting")
            .count()
    };
    let starts: Vec<_> = store
        .tail(Some("crash"), None, 1000)
        .unwrap()
        .into_iter()
        .filter(|r| r.event == "starting")
        .map(|r| r.timestamp_unix_ms)
        .collect();
    let attempts = &starts[starts.len() - 3..];
    assert!(attempts[1] - attempts[0] >= 500);
    assert!(attempts[2] - attempts[1] >= 1000);
    let count = launches();
    thread::sleep(Duration::from_millis(1200));
    assert_eq!(launches(), count);
    server.control("start", Some("crash"));
    server.state("crash", "backoff");
    server.control("stop", Some("crash"));
    server.state("crash", "stopped");
    let count = launches();
    thread::sleep(Duration::from_millis(1200));
    assert_eq!(launches(), count);
    server.wait_for("/apps/healthy/next", "arrived");
}

#[test]
fn startup_failures_also_exhaust_recovery_budget() {
    let dir = Server::fixture(&[("broken", "throw Error('startup failure')")]);
    let config = dir.path().join("server.json");
    fs::write(&config, r#"{"apps":[{"path":"broken","restart":{"onFailure":true,"maxRetries":1,"backoffMs":25,"maxBackoffMs":25}}]}"#).unwrap();
    let mut server = Server::start(dir, &config, free_port());
    server.ready();
    let failed = server.state("broken", "failed");
    assert!(
        failed["error"]
            .as_str()
            .unwrap()
            .contains("retry limit reached (1/1)")
    );
}
