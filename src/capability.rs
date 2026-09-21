//! Private, one-request-per-connection framed JSON transport for host-selected AI backends.
use crate::manifest::App;
use paraco::ai::{
    AppPolicy, Config, Error as AiError, FakeProxy, OpenAiProxy, Request, Response, Route,
    Selection,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{
    Arc, Mutex, OnceLock, Weak,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_FRAME: usize = 65536;
const MAX_HTTP: usize = 65536;
const REQUEST_HEADER_TIMEOUT: Duration = Duration::from_secs(1);
// A host configuration represents one provider service. Launches get their
// own unforgeable loopback tokens and listeners, while routing, credential
// handling, and provider admission remain shared for every app using it.
static PROVIDER_SERVICES: OnceLock<Mutex<BTreeMap<String, Weak<Provider>>>> = OnceLock::new();
static PROVIDER_RUNTIME: OnceLock<Result<tokio::runtime::Runtime, ()>> = OnceLock::new();

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostConfig {
    credentials: BTreeMap<String, String>,
    #[serde(default)]
    credential_files: BTreeMap<String, std::path::PathBuf>,
    #[serde(default)]
    provider_endpoints: BTreeMap<String, String>,
    routes: Vec<Route>,
    apps: BTreeMap<String, Grants>,
    default: Option<Selection>,
    #[serde(default)]
    provider_default_models: BTreeMap<String, String>,
    #[cfg(test)]
    #[serde(default)]
    provider_test_root: Option<std::path::PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Grants {
    credential_grants: BTreeSet<String>,
    default: Option<Selection>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    token: String,
    request: Request,
}

enum Provider {
    Fake(Box<FakeProxy>),
    OpenAi(Box<OpenAiProxy>),
}

impl Provider {
    fn complete(&self, caller: &str, request: &Request) -> Result<Response, AiError> {
        match self {
            Self::Fake(proxy) => proxy.complete(caller, request),
            Self::OpenAi(proxy) => provider_runtime()?.block_on(proxy.complete(caller, request)),
        }
    }

    fn stream<F>(&self, caller: &str, request: &Request, sink: F) -> Result<(), AiError>
    where
        F: FnMut(&str) -> Result<(), AiError>,
    {
        match self {
            Self::Fake(proxy) => {
                let response = proxy.complete(caller, request)?;
                let mut sink = sink;
                sink(&response.text)
            }
            Self::OpenAi(proxy) => provider_runtime()?
                .block_on(proxy.stream(caller, request, sink))
                .map(|_| ()),
        }
    }
}

fn provider_runtime() -> Result<&'static tokio::runtime::Runtime, AiError> {
    PROVIDER_RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|_| ())
        })
        .as_ref()
        .map_err(|_| AiError::ProviderFailed)
}

pub struct Capability {
    pub address: String,
    pub token: String,
    /// Loopback-only OpenAI-compatibility endpoint. This credential is minted
    /// for one launch and is distinct from provider and management secrets.
    pub http_address: String,
    pub http_token: String,
    pub timeout: Duration,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Capability {
    /// `deployment_id` is allocated by the host durable state.  Configuration
    /// grants are deliberately keyed by it, never by an app-controlled name.
    pub fn start(
        app: &App,
        deployment_id: &str,
        config_path: Option<&Path>,
    ) -> Result<Self, String> {
        let (host, service_key): (HostConfig, String) = match config_path {
            Some(path) => {
                let path = path
                    .canonicalize()
                    .map_err(|_| "cannot read AI configuration")?;
                if path.starts_with(&app.root) {
                    return Err("AI configuration must be outside the application directory".into());
                }
                let bytes = std::fs::read(&path).map_err(|_| "cannot read AI configuration")?;
                let digest = Sha256::digest(&bytes);
                (
                    serde_json::from_slice(&bytes).map_err(|_| "invalid AI configuration")?,
                    format!("{}:{digest:x}", path.display()),
                )
            }
            None => (HostConfig::default(), "no-configuration".into()),
        };
        let mut config = Config {
            credentials: host.credentials.clone(),
            credential_files: host.credential_files.clone(),
            provider_endpoints: host.provider_endpoints.clone(),
            routes: host.routes.clone(),
            default: host.default.clone(),
            provider_default_models: host.provider_default_models.clone(),
            ..Config::default()
        };
        for (name, grants) in &host.apps {
            config.apps.insert(
                name.clone(),
                AppPolicy {
                    // Whether an app receives a capability is determined by
                    // its manifest before this service is exposed. Keeping
                    // configured entries active here lets one immutable host
                    // service route requests for all granted deployments.
                    requests_ai: true,
                    credential_grants: grants.credential_grants.clone(),
                    default: grants.default.clone(),
                },
            );
        }
        config
            .apps
            .entry(deployment_id.to_owned())
            .or_default()
            .requests_ai = app.requests_ai;
        for path in config.credential_files.values_mut() {
            let canonical = path
                .canonicalize()
                .map_err(|_| "cannot read AI credential")?;
            if canonical.starts_with(&app.root) {
                return Err("AI credential must be outside the application directory".into());
            }
            // Retain the canonical host-owned path: a later symlink swap cannot
            // redirect a provider read into an application directory.
            *path = canonical;
        }
        let timeout = config.limits.timeout;
        let services = PROVIDER_SERVICES.get_or_init(|| Mutex::new(BTreeMap::new()));
        let mut services = services
            .lock()
            .map_err(|_| "AI provider service is unavailable")?;
        // Services exist only while at least one running capability uses them.
        // A config edit gets a new content key, and expired revisions are
        // pruned instead of accumulating host-owned routing snapshots.
        services.retain(|_, provider| provider.strong_count() != 0);
        let proxy = match services.get(&service_key).and_then(Weak::upgrade) {
            Some(proxy) => proxy,
            None => {
                let proxy = Arc::new(if config.provider_endpoints.is_empty() {
                    Provider::Fake(Box::new(FakeProxy::new(config).map_err(|e| e.to_string())?))
                } else {
                    Provider::OpenAi(Box::new(
                        new_openai_provider(config, &host).map_err(|e| e.to_string())?,
                    ))
                });
                services.insert(service_key, Arc::downgrade(&proxy));
                proxy
            }
        };
        drop(services);
        let mut random = [0u8; 32];
        getrandom::fill(&mut random).map_err(|_| "cannot generate capability token")?;
        let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let listener =
            TcpListener::bind(("127.0.0.1", 0)).map_err(|_| "cannot bind AI capability")?;
        let address = listener
            .local_addr()
            .map_err(|_| "cannot inspect AI listener")?
            .to_string();
        listener
            .set_nonblocking(true)
            .map_err(|_| "cannot configure AI listener")?;
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker_token = token.clone();
        let caller = deployment_id.to_owned();
        let worker_proxy = proxy.clone();
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = serve(&mut stream, &worker_token, &caller, &worker_proxy, timeout);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        });
        let http_listener =
            TcpListener::bind(("127.0.0.1", 0)).map_err(|_| "cannot bind AI HTTP listener")?;
        let http_address = http_listener
            .local_addr()
            .map_err(|_| "cannot inspect AI HTTP listener")?
            .to_string();
        http_listener
            .set_nonblocking(true)
            .map_err(|_| "cannot configure AI HTTP listener")?;
        let http_token = token.clone();
        let http_stop = stop.clone();
        let http_caller = deployment_id.to_owned();
        let http_proxy = proxy;
        let http_worker = thread::spawn(move || {
            while !http_stop.load(Ordering::Relaxed) {
                match http_listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = serve_http(
                            &mut stream,
                            &http_token,
                            &http_caller,
                            &http_proxy,
                            timeout,
                        );
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            address,
            token: token.clone(),
            http_address,
            http_token: token.clone(),
            timeout,
            stop,
            worker: Some(thread::spawn(move || {
                let _ = worker.join();
                let _ = http_worker.join();
            })),
        })
    }
}

fn new_openai_provider(config: Config, _host: &HostConfig) -> Result<OpenAiProxy, AiError> {
    #[cfg(test)]
    if let Some(root) = &_host.provider_test_root {
        let certificate = std::fs::read(root).map_err(|_| AiError::InvalidProvider)?;
        let certificate =
            reqwest::Certificate::from_pem(&certificate).map_err(|_| AiError::InvalidProvider)?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .add_root_certificate(certificate)
            .build()
            .map_err(|_| AiError::InvalidProvider)?;
        return OpenAiProxy::with_client(config, client);
    }
    OpenAiProxy::new(config)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HttpRequest {
    model: Option<String>,
    messages: Vec<HttpMessage>,
    stream: Option<bool>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HttpMessage {
    role: String,
    content: String,
}

fn serve_http(
    stream: &mut TcpStream,
    token: &str,
    caller: &str,
    proxy: &Provider,
    timeout: Duration,
) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    // Treat the HTTP head and body as one bounded request, rather than
    // resetting a socket-level timeout for every byte a slow client sends.
    let deadline = Instant::now() + REQUEST_HEADER_TIMEOUT;
    let mut raw = Vec::new();
    let mut byte = [0u8; 1];
    while raw.len() < MAX_HTTP {
        read_exact_before(stream, &mut byte, deadline)?;
        raw.push(byte[0]);
        if raw.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head = std::str::from_utf8(&raw).unwrap_or("");
    let mut lines = head.split("\r\n");
    let request = lines.next().unwrap_or("");
    let mut length = 0usize;
    let mut authorized = false;
    let mut origin = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = value.trim().parse().unwrap_or(MAX_HTTP + 1),
            "authorization" => {
                if let Some(value) = value.trim().strip_prefix("Bearer ") {
                    authorized = token_matches(value, token);
                }
            }
            "origin" => origin = true,
            _ => {}
        }
    }
    let mut body = vec![0; length.min(MAX_HTTP + 1)];
    if length <= MAX_HTTP {
        read_exact_before(stream, &mut body, deadline)?;
    }
    let parsed = serde_json::from_slice::<HttpRequest>(&body);
    let (status, value) = if !request.starts_with("POST /v1/chat/completions ") || origin {
        (
            400,
            serde_json::json!({"error":{"message":"unsupported HTTP request","type":"invalid_request_error"}}),
        )
    } else if !authorized {
        (
            401,
            serde_json::json!({"error":{"message":"unauthorized","type":"authentication_error"}}),
        )
    } else if length > MAX_HTTP {
        (
            413,
            serde_json::json!({"error":{"message":"request too large","type":"invalid_request_error"}}),
        )
    } else if matches!(parsed.as_ref(), Ok(v) if v.stream == Some(true) && v.messages.len() == 1 && v.messages[0].role == "user")
    {
        stream.set_write_timeout(Some(timeout))?;
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n")?;
        let v = parsed.as_ref().expect("matched parsed request");
        let request = Request {
            provider: None,
            model: v.model.clone(),
            prompt: v.messages[0].content.clone(),
        };
        let result = proxy.stream(caller, &request, |text| {
            let json = serde_json::json!({"choices":[{"delta":{"content": text}}]});
            let body = serde_json::to_string(&json).map_err(|_| AiError::ProviderFailed)?;
            stream
                .write_all(format!("data: {body}\n\n").as_bytes())
                .map_err(|_| AiError::ProviderFailed)
        });
        if result.is_ok() {
            let _ = stream.write_all(b"data: [DONE]\n\n");
        } else {
            // Headers may already be committed, so make failure explicit in
            // the stream rather than letting EOF resemble normal completion.
            let _ = stream.write_all(b"event: error\ndata: {\"error\":{\"message\":\"AI provider stream failed\",\"type\":\"provider_error\"}}\n\n");
        }
        return Ok(());
    } else {
        match parsed {
            Ok(v) if v.messages.len() != 1 || v.messages[0].role != "user" => (
                400,
                serde_json::json!({"error":{"message":"unsupported chat completion fields","type":"invalid_request_error"}}),
            ),
            Ok(v) => match proxy.complete(
                caller,
                &Request {
                    provider: None,
                    model: v.model,
                    prompt: v.messages[0].content.clone(),
                },
            ) {
                Ok(r) => (
                    200,
                    serde_json::json!({"id":"paraco","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":r.text},"finish_reason":"stop"}],"model":r.selection.model}),
                ),
                Err(e) => (
                    403,
                    serde_json::json!({"error":{"message":e.to_string(),"type":"permission_error"}}),
                ),
            },
            Err(_) => (
                400,
                serde_json::json!({"error":{"message":"invalid JSON","type":"invalid_request_error"}}),
            ),
        }
    };
    let out = serde_json::to_vec(&value)?;
    stream.set_write_timeout(Some(timeout))?;
    stream.write_all(format!("HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", if status == 200 { "OK" } else { "Error" }, out.len()).as_bytes())?;
    stream.write_all(&out)
}

impl Drop for Capability {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn read_exact_before(
    stream: &mut TcpStream,
    mut bytes: &mut [u8],
    deadline: Instant,
) -> std::io::Result<()> {
    while !bytes.is_empty() {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or(std::io::ErrorKind::TimedOut)?;
        stream.set_read_timeout(Some(remaining))?;
        let count = stream.read(bytes)?;
        if count == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        bytes = &mut bytes[count..];
    }
    Ok(())
}

fn serve(
    stream: &mut TcpStream,
    token: &str,
    caller: &str,
    proxy: &Provider,
    timeout: Duration,
) -> std::io::Result<()> {
    // Accepted sockets can inherit the listener's nonblocking mode on macOS.
    // Framed reads and writes below use blocking I/O with bounded timeouts.
    stream.set_nonblocking(false)?;
    let deadline = Instant::now() + REQUEST_HEADER_TIMEOUT;
    let mut header = [0; 4];
    read_exact_before(stream, &mut header, deadline)?;
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_FRAME {
        return Ok(());
    }
    let mut body = vec![0; len];
    read_exact_before(stream, &mut body, deadline)?;
    let response = match serde_json::from_slice::<Envelope>(&body) {
        Err(_) => serde_json::json!({"error": "invalid AI request"}),
        Ok(envelope) if !token_matches(&envelope.token, token) => {
            serde_json::json!({"error": "unauthorized AI caller"})
        }
        Ok(envelope) => match proxy.complete(caller, &envelope.request) {
            Ok(response) => serde_json::json!({"result": response}),
            Err(error) => serde_json::json!({"error": error.to_string()}),
        },
    };
    let bytes = serde_json::to_vec(&response)?;
    stream.set_write_timeout(Some(timeout))?;
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)
}

fn token_matches(candidate: &str, expected: &str) -> bool {
    candidate.len() == expected.len()
        && candidate
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonblocking_accepted_socket_waits_for_request_frame() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        // Explicitly reproduce macOS inheritance on every test platform.
        server.set_nonblocking(true).unwrap();
        let worker = thread::spawn(move || {
            serve(
                &mut server,
                "expected",
                "test",
                &Provider::Fake(Box::new(FakeProxy::new(Config::default()).unwrap())),
                Duration::from_secs(3),
            )
        });
        thread::sleep(Duration::from_millis(50));
        let body = br#"{"token":"wrong","request":{"prompt":"hello"}}"#;
        client
            .write_all(&(body.len() as u32).to_be_bytes())
            .unwrap();
        thread::sleep(Duration::from_millis(50));
        client.write_all(body).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut header = [0; 4];
        client.read_exact(&mut header).unwrap();
        let mut response = vec![0; u32::from_be_bytes(header) as usize];
        client.read_exact(&mut response).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&response).unwrap()["error"],
            "unauthorized AI caller"
        );
        worker.join().unwrap().unwrap();
    }

    #[test]
    fn trickled_http_request_cannot_extend_the_absolute_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let app = App {
            name: "test".into(),
            root: dir.path().to_path_buf(),
            entrypoint: dir.path().join("main.ts"),
            requests_ai: true,
        };
        let capability = Capability::start(&app, "deployment-test", None).unwrap();
        let mut slow = TcpStream::connect(&capability.http_address).unwrap();
        // Keep supplying bytes more often than the old per-read timeout. The
        // next HTTP client must still be served once the absolute deadline
        // expires, while this connection is still open.
        for byte in b"POST" {
            slow.write_all(&[*byte]).unwrap();
            thread::sleep(Duration::from_millis(300));
        }
        let started = Instant::now();
        let response = http_exchange(
            &capability.http_address,
            &capability.http_token,
            serde_json::json!({"messages":[{"role":"user","content":"private"}]}),
        );
        assert!(
            started.elapsed() < Duration::from_millis(600),
            "trickled HTTP header held the listener for {:?}",
            started.elapsed()
        );
        assert!(response.starts_with("HTTP/1.1 403 Error"));
    }

    fn exchange(address: &str, value: serde_json::Value) -> serde_json::Value {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let bytes = serde_json::to_vec(&value).unwrap();
        stream
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .unwrap();
        stream.write_all(&bytes).unwrap();
        let mut header = [0; 4];
        stream.read_exact(&mut header).unwrap();
        let mut body = vec![0; u32::from_be_bytes(header) as usize];
        stream.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn http_exchange(address: &str, token: &str, body: serde_json::Value) -> String {
        let body = serde_json::to_vec(&body).unwrap();
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .write_all(
                format!(
                    "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .unwrap();
        stream.write_all(&body).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn granted_fake_provider_works_over_private_and_streaming_http_transports() {
        let app_root = tempfile::tempdir().unwrap();
        let host_root = tempfile::tempdir().unwrap();
        let config = host_root.path().join("ai.json");
        std::fs::write(
            &config,
            r#"{"credentials":{"fixture":"fake"},"routes":[{"selection":{"provider":"fake","model":"small"},"credential":"fixture"}],"apps":{"deployment-fixture":{"credential_grants":["fixture"]}},"default":{"provider":"fake","model":"small"}}"#,
        )
        .unwrap();
        let app = App {
            name: "fixture".into(),
            root: app_root.path().canonicalize().unwrap(),
            entrypoint: app_root.path().join("main.ts"),
            requests_ai: true,
        };
        let capability = Capability::start(&app, "deployment-fixture", Some(&config)).unwrap();
        let completed = exchange(
            &capability.address,
            serde_json::json!({"token": capability.token, "request": {"prompt":"private"}}),
        );
        assert_eq!(completed["result"]["text"], "Fake AI response");
        let streamed = http_exchange(
            &capability.http_address,
            &capability.http_token,
            serde_json::json!({"messages":[{"role":"user","content":"private"}],"stream":true}),
        );
        assert!(streamed.starts_with("HTTP/1.1 200 OK"));
        assert!(streamed.contains("Fake AI response"));
        assert!(streamed.contains("data: [DONE]"));
    }

    #[test]
    fn shared_configuration_grants_each_deployment_and_content_changes_refresh_service() {
        let app_root = tempfile::tempdir().unwrap();
        let host_root = tempfile::tempdir().unwrap();
        let config = host_root.path().join("ai.json");
        let write_config = |model: &str| {
            std::fs::write(
                &config,
                format!(
                    r#"{{"credentials":{{"fixture":"fake"}},"routes":[{{"selection":{{"provider":"fake","model":"{model}"}},"credential":"fixture"}}],"apps":{{"deployment-one":{{"credential_grants":["fixture"]}},"deployment-two":{{"credential_grants":["fixture"]}}}},"default":{{"provider":"fake","model":"{model}"}}}}"#
                ),
            )
            .unwrap();
        };
        write_config("one");
        let app = App {
            name: "fixture".into(),
            root: app_root.path().canonicalize().unwrap(),
            entrypoint: app_root.path().join("main.ts"),
            requests_ai: true,
        };
        let one = Capability::start(&app, "deployment-one", Some(&config)).unwrap();
        let two = Capability::start(&app, "deployment-two", Some(&config)).unwrap();
        for capability in [&one, &two] {
            assert_eq!(
                exchange(
                    &capability.address,
                    serde_json::json!({"token": capability.token, "request": {"prompt":"private"}}),
                )["result"]["selection"]["model"],
                "one"
            );
        }
        drop(one);
        drop(two);
        write_config("two");
        let refreshed = Capability::start(&app, "deployment-one", Some(&config)).unwrap();
        assert_eq!(
            exchange(
                &refreshed.address,
                serde_json::json!({"token": refreshed.token, "request": {"prompt":"private"}}),
            )["result"]["selection"]["model"],
            "two"
        );
    }

    #[cfg(unix)]
    #[test]
    fn trusted_tls_provider_fixture_exercises_reqwest_completion_and_streaming() {
        use std::process::{Command, Stdio};

        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        let app_root = tempfile::tempdir().unwrap();
        let host_root = tempfile::tempdir().unwrap();
        let certificate = host_root.path().join("cert.pem");
        let key = host_root.path().join("key.pem");
        assert!(
            Command::new("openssl")
                .args([
                    "req",
                    "-x509",
                    "-newkey",
                    "rsa:2048",
                    "-nodes",
                    "-keyout",
                    key.to_str().unwrap(),
                    "-out",
                    certificate.to_str().unwrap(),
                    "-subj",
                    "/CN=127.0.0.1",
                    "-addext",
                    "subjectAltName=IP:127.0.0.1",
                    "-addext",
                    "basicConstraints=critical,CA:FALSE",
                    "-days",
                    "1",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let script = r#"import json, ssl, socket, sys
s=socket.socket(); s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1); s.bind(('127.0.0.1',int(sys.argv[1]))); s.listen(2)
c=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); c.load_cert_chain(sys.argv[2],sys.argv[3]); print('ready',flush=True)
for _ in range(2):
 x=c.wrap_socket(s.accept()[0],server_side=True); raw=b''
 while b'\r\n\r\n' not in raw: raw+=x.recv(4096)
 head,body=raw.split(b'\r\n\r\n',1); n=int([z for z in head.split(b'\r\n') if z.lower().startswith(b'content-length:')][0].split(b':',1)[1])
 while len(body)<n: body+=x.recv(4096)
 if json.loads(body).get('stream'):
  out=b'data: {\"choices\":[{\"delta\":{\"content\":\"fixture\"}}]}\n\ndata: [DONE]\n\n'; x.sendall(b'HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: '+str(len(out)).encode()+b'\r\n\r\n'+out)
 else:
  out=b'{\"choices\":[{\"message\":{\"content\":\"fixture\"}}]}'; x.sendall(b'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: '+str(len(out)).encode()+b'\r\n\r\n'+out)
 x.close()
"#;
        let mut provider = ChildGuard(
            Command::new("python3")
                .args([
                    "-c",
                    script,
                    &port.to_string(),
                    certificate.to_str().unwrap(),
                    key.to_str().unwrap(),
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let mut ready = [0; 6];
        provider
            .0
            .stdout
            .as_mut()
            .unwrap()
            .read_exact(&mut ready)
            .unwrap();
        assert_eq!(&ready, b"ready\n");
        let secret = host_root.path().join("secret");
        std::fs::write(&secret, "fixture-secret").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        let config = host_root.path().join("ai.json");
        std::fs::write(&config, format!(r#"{{"credentials":{{"fixture":"fixture"}},"credential_files":{{"fixture":"{}"}},"provider_endpoints":{{"fixture":"https://127.0.0.1:{port}/v1/chat/completions"}},"provider_test_root":"{}","routes":[{{"selection":{{"provider":"fixture","model":"test"}},"credential":"fixture"}}],"apps":{{"deployment-tls":{{"credential_grants":["fixture"]}}}},"default":{{"provider":"fixture","model":"test"}}}}"#, secret.display(), certificate.display())).unwrap();
        let app = App {
            name: "fixture".into(),
            root: app_root.path().canonicalize().unwrap(),
            entrypoint: app_root.path().join("main.ts"),
            requests_ai: true,
        };
        let capability = Capability::start(&app, "deployment-tls", Some(&config)).unwrap();
        let completed = exchange(
            &capability.address,
            serde_json::json!({"token": capability.token, "request":{"prompt":"private"}}),
        );
        assert_eq!(completed["result"]["text"], "fixture", "{completed}");
        let streamed = http_exchange(
            &capability.http_address,
            &capability.http_token,
            serde_json::json!({"messages":[{"role":"user","content":"private"}],"stream":true}),
        );
        assert!(streamed.contains("fixture") && streamed.contains("data: [DONE]"));
        drop(capability);
        assert!(provider.0.wait().unwrap().success());
    }

    #[test]
    fn authenticates_rejects_forged_identity_and_releases_listener() {
        let dir = tempfile::tempdir().unwrap();
        let app = App {
            name: "test".into(),
            root: dir.path().to_path_buf(),
            entrypoint: dir.path().join("main.ts"),
            requests_ai: true,
        };
        let capability = Capability::start(&app, "deployment-test", None).unwrap();
        let request = serde_json::json!({"prompt": "private"});
        assert_eq!(
            exchange(
                &capability.address,
                serde_json::json!({"token": "wrong", "request": request})
            )["error"],
            "unauthorized AI caller"
        );
        assert_eq!(
            exchange(
                &capability.address,
                serde_json::json!({"token": capability.token, "request": {"prompt": "private", "caller": "other"}})
            )["error"],
            "invalid AI request"
        );
        assert_eq!(
            exchange(
                &capability.address,
                serde_json::json!({"token": capability.token, "request": request})
            )["error"],
            "no permitted application or runtime AI default is configured"
        );
        let address = capability.address.clone();
        let old_token = capability.token.clone();
        drop(capability);
        let listener = TcpListener::bind(&address).unwrap();
        drop(listener);
        let next = Capability::start(&app, "deployment-test", None).unwrap();
        assert_ne!(next.token, old_token);
        assert_eq!(
            exchange(
                &next.address,
                serde_json::json!({"token": old_token, "request": request})
            )["error"],
            "unauthorized AI caller"
        );
        // Oversized and stalled clients cannot permanently wedge the worker.
        let mut oversized = TcpStream::connect(&next.address).unwrap();
        oversized.write_all(&65537u32.to_be_bytes()).unwrap();
        oversized
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut byte = [0];
        assert_eq!(oversized.read(&mut byte).unwrap(), 0);
        let _stalled = TcpStream::connect(&next.address).unwrap();
        assert_eq!(
            exchange(
                &next.address,
                serde_json::json!({"token": "wrong", "request": request})
            )["error"],
            "unauthorized AI caller"
        );
        let start = Instant::now();
        drop(next);
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn config_must_be_host_owned_and_invalid_content_is_redacted() {
        let root = tempfile::tempdir().unwrap();
        let app = App {
            name: "test".into(),
            root: root.path().canonicalize().unwrap(),
            entrypoint: root.path().join("main.ts"),
            requests_ai: true,
        };
        let inside = root.path().join("ai.json");
        std::fs::write(&inside, "{}").unwrap();
        assert!(
            Capability::start(&app, "deployment-test", Some(&inside))
                .err()
                .unwrap()
                .contains("outside")
        );
        let host = tempfile::tempdir().unwrap();
        let path = host.path().join("ai.json");
        std::fs::write(&path, r#"{"secret-sentinel": "private"}"#).unwrap();
        assert_eq!(
            Capability::start(&app, "deployment-test", Some(&path))
                .err()
                .unwrap(),
            "invalid AI configuration"
        );
    }
}
