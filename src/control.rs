//! Private local CLI transport. Management never shares an HTTP origin with apps.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Status,
    Start,
    Stop,
    Restart,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub action: Action,
    pub app: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct Status {
    pub name: String,
    pub desired: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct Reply {
    apps: Vec<Status>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub fn execute(port: u16, action: Action, app: Option<String>) -> Result<(), String> {
    let reply = transport::request(port, Command { action, app })?;
    if let Some(error) = reply.error {
        return Err(error);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&reply.apps).map_err(|e| e.to_string())?
    );
    Ok(())
}

pub use transport::Server;

#[cfg(unix)]
mod transport {
    use super::*;
    use std::{
        fs,
        io::{Read, Write},
        os::unix::{
            fs::{DirBuilderExt, MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };

    fn endpoint(port: u16) -> Result<PathBuf, String> {
        if port == 0 {
            return Err("port must be from 1 through 65535".into());
        }
        let uid = unsafe { libc::geteuid() };
        let directory = std::env::temp_dir().join(format!("paraco-control-{uid}"));
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("cannot create control directory: {error}")),
        }
        let metadata = fs::symlink_metadata(&directory).map_err(|e| e.to_string())?;
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o777 != 0o700 {
            return Err(format!(
                "control directory {} must be owned by the current user with permissions 0700 and must not be a symlink",
                directory.display()
            ));
        }
        Ok(directory.join(format!("{port}.sock")))
    }

    pub struct Server {
        stop: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
        path: PathBuf,
    }

    impl Server {
        pub fn start(
            port: u16,
            handler: impl Fn(Command) -> Result<Vec<Status>, String> + Send + 'static,
        ) -> Result<Self, String> {
            let path = endpoint(port)?;
            let listener = UnixListener::bind(&path).map_err(|e| format!("cannot bind control socket {}: {e}; if a previous server was killed, remove its stale socket after checking that it is no longer running", path.display()))?;
            // Own cleanup immediately, including failures during setup.
            let mut server = Self {
                stop: Arc::new(AtomicBool::new(false)),
                worker: None,
                path,
            };
            fs::set_permissions(&server.path, fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
            listener.set_nonblocking(true).map_err(|e| e.to_string())?;
            let stop = server.stop.clone();
            server.worker = Some(thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                            let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                            let result = read_frame(&mut stream, 4096)
                                .and_then(|bytes| {
                                    serde_json::from_slice(&bytes)
                                        .map_err(|_| "invalid control request".into())
                                })
                                .and_then(&handler);
                            let reply = match result {
                                Ok(apps) => Reply { apps, error: None },
                                Err(error) => Reply {
                                    apps: vec![],
                                    error: Some(error),
                                },
                            };
                            let _ = write_frame(&mut stream, &reply);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10))
                        }
                        Err(_) => break,
                    }
                }
            }));
            Ok(server)
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            let _ = fs::remove_file(&self.path);
        }
    }

    fn read_frame(stream: &mut UnixStream, limit: usize) -> Result<Vec<u8>, String> {
        let timeout = stream
            .read_timeout()
            .map_err(|e| e.to_string())?
            .unwrap_or(Duration::from_secs(1));
        let deadline = Instant::now() + timeout;
        let mut bytes = Vec::new();
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|duration| !duration.is_zero())
                .ok_or("control message read timed out")?;
            stream
                .set_read_timeout(Some(remaining))
                .map_err(|e| e.to_string())?;
            let mut buffer = [0u8; 4096];
            let count = stream
                .read(&mut buffer)
                .map_err(|e| format!("cannot read control message: {e}"))?;
            if count == 0 {
                return Err("incomplete control message".into());
            }
            let end = buffer[..count].iter().position(|byte| *byte == b'\n');
            let count = end.map_or(count, |index| index + 1);
            if bytes.len() + count > limit {
                return Err("control message is too large".into());
            }
            bytes.extend_from_slice(&buffer[..count]);
            if end.is_some() {
                return Ok(bytes);
            }
        }
    }

    fn write_frame(stream: &mut UnixStream, value: &impl Serialize) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        let timeout = stream
            .write_timeout()
            .map_err(|e| e.to_string())?
            .unwrap_or(Duration::from_secs(1));
        let deadline = Instant::now() + timeout;
        let mut remaining = bytes.as_slice();
        while !remaining.is_empty() {
            let timeout = deadline
                .checked_duration_since(Instant::now())
                .filter(|duration| !duration.is_zero())
                .ok_or("control message write timed out")?;
            stream
                .set_write_timeout(Some(timeout))
                .map_err(|e| e.to_string())?;
            let count = stream
                .write(remaining)
                .map_err(|e| format!("cannot write control message: {e}"))?;
            if count == 0 {
                return Err("control connection closed".into());
            }
            remaining = &remaining[count..];
        }
        Ok(())
    }

    pub(super) fn request(port: u16, command: Command) -> Result<Reply, String> {
        let path = endpoint(port)?;
        let metadata = fs::symlink_metadata(&path).map_err(|e| {
            format!(
                "cannot find local server control socket {}: {e}",
                path.display()
            )
        })?;
        use std::os::unix::fs::FileTypeExt;
        if !metadata.file_type().is_socket()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o600
        {
            return Err(
                "control socket must be owned by the current user with permissions 0600".into(),
            );
        }
        let mut stream = UnixStream::connect(&path)
            .map_err(|e| format!("cannot connect to local server: {e}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .map_err(|e| e.to_string())?;
        write_frame(&mut stream, &command)?;
        serde_json::from_slice(&read_frame(&mut stream, 1024 * 1024)?)
            .map_err(|_| "invalid control response".into())
    }
}

#[cfg(not(unix))]
mod transport {
    use super::*;
    pub struct Server;
    impl Server {
        pub fn start(
            _port: u16,
            _handler: impl Fn(Command) -> Result<Vec<Status>, String> + Send + 'static,
        ) -> Result<Self, String> {
            // Preserve foreground hosting on platforms without this transport.
            Ok(Self)
        }
    }
    pub(super) fn request(_port: u16, _command: Command) -> Result<Reply, String> {
        Err("local lifecycle commands currently require Unix".into())
    }
}
