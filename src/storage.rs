//! Per-launch scoped storage transport. The caller cannot select a deployment.
use crate::state::Store;
use serde::Deserialize;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    token: String,
    namespace: String,
    key: String,
    #[serde(default)]
    value: Option<serde_json::Value>,
    operation: Operation,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Operation {
    Get,
    Set,
    Delete,
}

pub struct Service {
    pub address: String,
    pub token: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Service {
    pub fn start(store: Arc<Mutex<Store>>, deployment: String) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let address = listener
            .local_addr()
            .map_err(|e| e.to_string())?
            .to_string();
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let auth = token.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let _ = stream.set_nonblocking(true);
                let deadline = Instant::now() + Duration::from_secs(2);
                let mut input = Vec::new();
                let mut buffer = [0; 4096];
                while Instant::now() < deadline
                    && input.len() <= 70000
                    && !done.load(Ordering::Relaxed)
                {
                    match stream.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(n) => {
                            input.extend_from_slice(&buffer[..n]);
                            if input.contains(&b'\n') {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(_) => break,
                    }
                }
                let result = if input.len() > 70000 {
                    Err("storage request too large".into())
                } else {
                    serde_json::from_slice::<Request>(&input)
                        .map_err(|_| "invalid storage request".to_string())
                        .and_then(|r| {
                            if r.token != auth {
                                return Err("unauthorized storage request".into());
                            }
                            store.lock().map_err(|_| "storage unavailable")?.storage(
                                &deployment,
                                &r.namespace,
                                &r.key,
                                if matches!(r.operation, Operation::Set) {
                                    Some(r.value.unwrap_or(serde_json::Value::Null))
                                } else {
                                    None
                                },
                                matches!(r.operation, Operation::Delete),
                            )
                        })
                };
                let reply = match result {
                    Ok(value) => serde_json::json!({"value":value}),
                    Err(error) => serde_json::json!({"error":error}),
                };
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                let _ = writeln!(stream, "{reply}");
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
impl Drop for Service {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
