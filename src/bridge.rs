//! Host-only translation of scoped HTTP bindings to existing capability services.
use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
pub struct Bridge {
    pub address: String,
    pub token: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Bridge {
    pub fn start(settings: serde_json::Value, directory: &std::path::Path) -> Result<Self, String> {
        let socket = directory.join("host.sock");
        let listener =
            std::os::unix::net::UnixListener::bind(&socket).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let address = format!("unix:{}", socket.display());
        let mut random = [0; 32];
        getrandom::fill(&mut random).map_err(|e| e.to_string())?;
        let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let auth = token.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let _ = respond(&mut stream, &settings, &auth);
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
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.worker.take() {
            let _ = t.join();
        }
    }
}
fn respond(
    stream: &mut std::os::unix::net::UnixStream,
    settings: &serde_json::Value,
    auth: &str,
) -> std::io::Result<()> {
    let result = handle(stream, settings, auth);
    let (status, body) = match result {
        Ok(v) => (200, v),
        Err(e) => (403, serde_json::json!({"error":e})),
    };
    let body = body.to_string();
    write!(
        stream,
        "HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn handle(
    stream: &mut std::os::unix::net::UnixStream,
    settings: &serde_json::Value,
    auth: &str,
) -> Result<serde_json::Value, String> {
    // macOS can inherit O_NONBLOCK from the listener. Request parsing and
    // response writing use blocking I/O with bounded socket deadlines.
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte).map_err(|e| e.to_string())?;
        header.push(byte[0]);
        if header.len() > 8192 {
            return Err("headers too large".into());
        }
    }
    let header = String::from_utf8(header).map_err(|e| e.to_string())?;
    if !header
        .lines()
        .filter_map(|l| l.split_once(':'))
        .any(|(k, v)| {
            k.eq_ignore_ascii_case("authorization") && v.trim() == format!("Bearer {auth}")
        })
    {
        return Err("unauthorized capability connection".into());
    }
    let mut first = header.lines().next().unwrap_or("").split_whitespace();
    if first.next() != Some("POST") {
        return Err("POST required".into());
    }
    let cap = first.next().unwrap_or("").trim_start_matches('/');
    if !["ai", "storage", "tasks", "network", "system"].contains(&cap) {
        return Err("unknown capability".into());
    }
    let config = &settings[cap];
    let length = header
        .lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .ok_or("content length required")?
        .1
        .trim()
        .parse::<usize>()
        .map_err(|_| "invalid length")?;
    if length
        > if cap == "tasks" {
            crate::tasks::MAX_REQUEST_BYTES
        } else {
            96 * 1024
        }
    {
        return Err("request too large".into());
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body).map_err(|e| e.to_string())?;
    let mut body: serde_json::Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    if cap == "system" {
        if config.as_bool() != Some(true) {
            return Err("system capability not granted".into());
        }
        return Ok(serde_json::json!({"value": crate::telemetry::snapshot()?}));
    }
    if cap == "network" {
        return outbound(&body, config);
    }
    let address = config["address"].as_str().ok_or("capability not granted")?;
    body.as_object_mut()
        .ok_or("object required")?
        .insert("token".into(), config["token"].clone());
    let mut connection = TcpStream::connect(address).map_err(|e| e.to_string())?;
    let timeout = Duration::from_millis(config["timeoutMs"].as_u64().unwrap_or(30000));
    connection
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    connection
        .set_write_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    let mut response = Vec::new();
    if cap == "ai" {
        if bytes.len() > 65536 {
            return Err("AI request too large".into());
        }
        connection
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .map_err(|e| e.to_string())?;
        connection.write_all(&bytes).map_err(|e| e.to_string())?;
        let mut size = [0; 4];
        connection
            .read_exact(&mut size)
            .map_err(|e| e.to_string())?;
        let size = u32::from_be_bytes(size) as usize;
        if size > 65536 {
            return Err("AI response too large".into());
        }
        response.resize(size, 0);
        connection
            .read_exact(&mut response)
            .map_err(|e| e.to_string())?;
    } else {
        connection.write_all(&bytes).map_err(|e| e.to_string())?;
        connection.write_all(b"\n").map_err(|e| e.to_string())?;
        connection
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut response)
            .map_err(|e| e.to_string())?;
        if response.len() > 16 * 1024 * 1024 {
            return Err("response too large".into());
        }
    }
    serde_json::from_slice(&response).map_err(|e| e.to_string())
}

fn outbound(
    body: &serde_json::Value,
    grants: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    use base64::Engine;
    let url =
        url::Url::parse(body["url"].as_str().ok_or("URL required")?).map_err(|_| "invalid URL")?;
    if !["http", "https"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("unsupported network URL".into());
    }
    let host = url.host_str().ok_or("missing host")?;
    let authority = format!(
        "{host}:{}",
        url.port_or_known_default().ok_or("missing port")?
    );
    if !grants.as_array().is_some_and(|g| {
        g.iter()
            .any(|v| v.as_str().is_some_and(|v| v == host || v == authority))
    }) {
        return Err("network destination not granted".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body["body"].as_str().ok_or("body required")?)
        .map_err(|_| "invalid body")?;
    let method =
        reqwest::Method::from_bytes(body["method"].as_str().ok_or("method required")?.as_bytes())
            .map_err(|_| "invalid method")?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let client=reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(30)).build().map_err(|e|e.to_string())?;
        let mut request=client.request(method,url).body(bytes);
        for pair in body["headers"].as_array().ok_or("headers required")? {
            let name=pair[0].as_str().ok_or("invalid header")?;
            if ["host","connection","content-length","transfer-encoding","proxy-authorization"].contains(&name.to_ascii_lowercase().as_str()) {continue;}
            request=request.header(name,pair[1].as_str().ok_or("invalid header value")?);
        }
        let mut response=request.send().await.map_err(|_|"network request failed")?;
        let status=response.status().as_u16();
        let headers:Vec<_>=response.headers().iter().filter(|(name,_)| !["connection","transfer-encoding","content-length"].contains(&name.as_str())).map(|(n,v)|(n.as_str().to_owned(),v.to_str().unwrap_or("").to_owned())).collect();
        let mut bytes=Vec::new();
        while let Some(chunk)=response.chunk().await.map_err(|_|"network response failed")? {
            if bytes.len()+chunk.len()>1024*1024 {return Err("network response exceeds 1 MiB".into());}
            bytes.extend_from_slice(&chunk);
        }
        Ok(serde_json::json!({"status":status,"headers":headers,"body":base64::engine::general_purpose::STANDARD.encode(bytes)}))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nonblocking_accepted_socket_waits_for_headers_and_body() {
        let (mut client, mut server) = std::os::unix::net::UnixStream::pair().unwrap();
        // Reproduce inherited macOS listener flags on every test platform.
        server.set_nonblocking(true).unwrap();
        let worker = thread::spawn(move || {
            respond(&mut server, &serde_json::json!({"network": []}), "expected")
        });
        thread::sleep(Duration::from_millis(50));
        let body = br#"{"url":"https://denied.example"}"#;
        write!(client, "POST /network HTTP/1.1\r\nAuthorization: Bearer expected\r\nContent-Length: {}\r\n\r\n", body.len()).unwrap();
        thread::sleep(Duration::from_millis(50));
        client.write_all(body).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 403 Result\r\n"),
            "{response}"
        );
        assert!(
            response.contains("network destination not granted"),
            "{response}"
        );
        worker.join().unwrap().unwrap();
    }
}
