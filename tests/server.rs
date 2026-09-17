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
