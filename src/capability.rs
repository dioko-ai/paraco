//! Private, one-request-per-connection framed JSON transport for the fake AI core.
use crate::manifest::App;
use paraco::ai::{AppPolicy, Config, FakeProxy, Request, Route, Selection};
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
const IO_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostConfig {
    credentials: BTreeMap<String, String>,
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

pub struct Capability {
    pub address: String,
    pub token: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Capability {
    pub fn start(app: &App, config_path: Option<&Path>) -> Result<Self, String> {
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
        config.apps.entry(app.name.clone()).or_default().requests_ai = app.requests_ai;
        let proxy = FakeProxy::new(config).map_err(|e| e.to_string())?;
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
        let caller = app.name.clone();
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = serve(&mut stream, &worker_token, &caller, &proxy);
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
            token,
            stop,
            worker: Some(worker),
        })
    }
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
    proxy: &FakeProxy,
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
                &FakeProxy::new(Config::default()).unwrap(),
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
        let capability = Capability::start(&app, None).unwrap();
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
        let next = Capability::start(&app, None).unwrap();
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
            Capability::start(&app, Some(&inside))
                .err()
                .unwrap()
                .contains("outside")
        );
        let host = tempfile::tempdir().unwrap();
        let path = host.path().join("ai.json");
        std::fs::write(&path, r#"{"secret-sentinel": "private"}"#).unwrap();
        assert_eq!(
            Capability::start(&app, Some(&path)).err().unwrap(),
            "invalid AI configuration"
        );
    }
}
