//! Bounded per-launch transport for explicitly granted task authoring.
//! Deployment identity is host-bound and cannot be supplied by callers.
use crate::tasks::Tasks;
use serde::Deserialize;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const MAX_REQUEST: usize = 96 * 1024;
const MAX_RESPONSE: usize = 4 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    token: String,
    action: String,
    #[serde(default)]
    body: serde_json::Value,
}
pub struct Service {
    pub address: String,
    pub token: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Service {
    pub fn start(
        tasks: Arc<Tasks>,
        deployment: String,
        _network: Vec<String>,
    ) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let address = listener
            .local_addr()
            .map_err(|e| e.to_string())?
            .to_string();
        let mut random = [0; 32];
        getrandom::fill(&mut random).map_err(|e| e.to_string())?;
        let token: String = random.iter().map(|v| format!("{v:02x}")).collect();
        let auth = token.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => serve(stream, &auth, &deployment, &tasks, &done),
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
impl Drop for Service {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn serve(
    mut stream: TcpStream,
    token: &str,
    deployment: &str,
    tasks: &Arc<Tasks>,
    stop: &AtomicBool,
) {
    if stream.set_nonblocking(true).is_err() {
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut input = Vec::new();
    let mut buffer = [0; 4096];
    while input.len() <= MAX_REQUEST && Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
        match stream.read(&mut buffer) {
            Ok(0) => return,
            Ok(n) => {
                input.extend_from_slice(&buffer[..n]);
                if input.contains(&b'\n') {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(_) => return,
        }
    }
    if stop.load(Ordering::Relaxed) {
        return;
    }
    let result = if input.len() > MAX_REQUEST || !input.ends_with(b"\n") {
        Err("task request too large or incomplete".into())
    } else {
        serde_json::from_slice::<Request>(&input)
            .map_err(|_| "invalid task request".to_string())
            .and_then(|request| {
                if request.token != token {
                    return Err("unauthorized task request".into());
                }
                tasks.command(deployment, &request.action, request.body)
            })
    };
    let response = match result {
        Ok(value) => serde_json::json!({"value":value}),
        Err(error) => serde_json::json!({"error":error}),
    };
    let Ok(mut output) = serde_json::to_vec(&response) else {
        return;
    };
    if output.len() > MAX_RESPONSE {
        output = br#"{"error":"response too large; narrow the query"}"#.to_vec();
    }
    output.push(b'\n');
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut offset = 0;
    while offset < output.len() && Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
        match stream.write(&output[offset..]) {
            Ok(0) => break,
            Ok(n) => offset += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    fn request(service: &Service, value: serde_json::Value) -> serde_json::Value {
        let mut socket = TcpStream::connect(&service.address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(4)))
            .unwrap();
        writeln!(socket, "{value}").unwrap();
        let mut response = String::new();
        std::io::BufReader::new(socket)
            .read_line(&mut response)
            .unwrap();
        serde_json::from_str(&response).unwrap()
    }
    #[test]
    fn transport_binds_scope_and_requires_per_launch_authorization() {
        let root = tempfile::tempdir().unwrap();
        let engine = Tasks::open(root.path(), "/not-used".into()).unwrap();
        engine.set_grant("one".into(), vec![]);
        engine.set_grant("two".into(), vec![]);
        let one = Service::start(engine.clone(), "one".into(), vec![]).unwrap();
        let two = Service::start(engine, "two".into(), vec![]).unwrap();
        assert!(
            request(&one, serde_json::json!({"token":"wrong","action":"list"}))["error"]
                .is_string()
        );
        let created = request(
            &one,
            serde_json::json!({"token":one.token,"action":"draft","body":{"name":"private","source":"export default async () => {};","schedule":{"kind":"manual"}}}),
        );
        assert!(created["value"]["job_id"].is_string(), "{created}");
        let foreign = request(
            &two,
            serde_json::json!({"token":two.token,"action":"revisions","body":{"job_id":created["value"]["job_id"]}}),
        );
        assert!(foreign["error"].is_string());
        assert_eq!(
            request(&two, serde_json::json!({"token":two.token,"action":"list"}))["value"],
            serde_json::json!([])
        );
        assert!(
            request(
                &one,
                serde_json::json!({"token":one.token,"deployment":"two","action":"list"})
            )["error"]
                .is_string()
        );
        let denied = request(
            &one,
            serde_json::json!({"token":one.token,"action":"draft","body":{"name":"network","source":"export default async () => {};","schedule":{"kind":"manual"},"network":["example.com:443"]}}),
        );
        assert!(denied["error"].is_string());
    }
    #[test]
    fn partial_request_cannot_delay_service_shutdown() {
        let root = tempfile::tempdir().unwrap();
        let engine = Tasks::open(root.path(), "/not-used".into()).unwrap();
        engine.set_grant("one".into(), vec![]);
        let service = Service::start(engine, "one".into(), vec![]).unwrap();
        let mut socket = TcpStream::connect(&service.address).unwrap();
        socket.write_all(b"{").unwrap();
        thread::sleep(Duration::from_millis(25));
        let start = Instant::now();
        drop(service);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
