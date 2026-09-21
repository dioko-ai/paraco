//! Private, one-request-per-connection framed JSON transport for host-selected AI backends.
use crate::manifest::App;
use paraco::ai::{
    AppPolicy, Config, Error as AiError, FakeProxy, OpenAiProxy, Request, Response, Route,
    Selection,
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_FRAME: usize = 65536;
const MAX_HTTP: usize = 65536;
const IO_TIMEOUT: Duration = Duration::from_secs(1);

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
    Fake(FakeProxy),
    OpenAi(OpenAiProxy),
}

impl Provider {
    fn complete(&self, caller: &str, request: &Request) -> Result<Response, AiError> {
        match self {
            Self::Fake(proxy) => proxy.complete(caller, request),
            Self::OpenAi(proxy) => tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .map_err(|_| AiError::ProviderFailed)?
                .block_on(proxy.complete(caller, request)),
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
            Self::OpenAi(proxy) => tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .map_err(|_| AiError::ProviderFailed)?
                .block_on(proxy.stream(caller, request, sink))
                .map(|_| ()),
        }
    }
}

pub struct Capability {
    pub address: String,
    pub token: String,
    /// Loopback-only OpenAI-compatibility endpoint. This credential is minted
    /// for one launch and is distinct from provider and management secrets.
    pub http_address: String,
    pub http_token: String,
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
        let host: HostConfig = match config_path {
            Some(path) => {
                let path = path
                    .canonicalize()
                    .map_err(|_| "cannot read AI configuration")?;
                if path.starts_with(&app.root) {
                    return Err("AI configuration must be outside the application directory".into());
                }
                let bytes = std::fs::read(path).map_err(|_| "cannot read AI configuration")?;
                serde_json::from_slice(&bytes).map_err(|_| "invalid AI configuration")?
            }
            None => HostConfig::default(),
        };
        let mut config = Config {
            credentials: host.credentials,
            credential_files: host.credential_files,
            provider_endpoints: host.provider_endpoints,
            routes: host.routes,
            default: host.default,
            provider_default_models: host.provider_default_models,
            ..Config::default()
        };
        for (name, grants) in host.apps {
            config.apps.insert(
                name,
                AppPolicy {
                    requests_ai: false,
                    credential_grants: grants.credential_grants,
                    default: grants.default,
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
        let proxy = Arc::new(if config.provider_endpoints.is_empty() {
            Provider::Fake(FakeProxy::new(config).map_err(|e| e.to_string())?)
        } else {
            Provider::OpenAi(OpenAiProxy::new(config).map_err(|e| e.to_string())?)
        });
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
                        let _ = serve(&mut stream, &worker_token, &caller, &worker_proxy);
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
                        let _ = serve_http(&mut stream, &http_token, &http_caller, &http_proxy);
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
            stop,
            worker: Some(thread::spawn(move || {
                let _ = worker.join();
                let _ = http_worker.join();
            })),
        })
    }
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
) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    let mut raw = Vec::new();
    let mut byte = [0u8; 1];
    while raw.len() < MAX_HTTP {
        stream.read_exact(&mut byte)?;
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
        stream.read_exact(&mut body)?;
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
        stream.set_write_timeout(Some(IO_TIMEOUT))?;
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
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
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
) -> std::io::Result<()> {
    // Accepted sockets can inherit the listener's nonblocking mode on macOS.
    // Framed reads and writes below use blocking I/O with bounded timeouts.
    stream.set_nonblocking(false)?;
    let deadline = Instant::now() + IO_TIMEOUT;
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
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
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
                &Provider::Fake(FakeProxy::new(Config::default()).unwrap()),
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
