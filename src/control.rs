//! Private local CLI transport. Management never shares an HTTP origin with apps.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Status,
    Open,
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
    pub deployment_id: String,
    pub desired: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct Reply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub apps: Vec<Status>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn execute(port: u16, action: Action, app: Option<String>) -> Result<(), String> {
    let reply = transport::request(port, Command { action, app })?;
    if let Some(error) = reply.error {
        return Err(error);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&reply.apps)
            .map_err(|e| format!("control socket operation failed: {e}"))?
    );
    Ok(())
}

pub fn open(port: u16, print: bool) -> Result<(), String> {
    let reply = transport::request(
        port,
        Command {
            action: Action::Open,
            app: None,
        },
    )?;
    if let Some(error) = reply.error {
        return Err(error);
    }
    let url = reply.url.ok_or("server did not return a management URL")?;
    if print {
        println!("{url}");
        return Ok(());
    }
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let status = std::process::Command::new(program)
        .arg(&url)
        .status()
        .map_err(|e| format!("cannot open browser: {e}; use --print"))?;
    if !status.success() {
        return Err("browser opener failed; use --print".into());
    }
    Ok(())
}

pub use transport::Server;

#[cfg(unix)]
mod transport {
    use super::*;
    use std::{
        fs,
        io::{Read, Write},
        os::fd::AsRawFd,
        os::unix::{
            fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
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
        let metadata = fs::symlink_metadata(&directory)
            .map_err(|e| format!("control socket operation failed: {e}"))?;
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o777 != 0o700 {
            return Err(format!(
                "control directory {} must be owned by the current user with permissions 0700 and must not be a symlink",
                directory.display()
            ));
        }
        Ok(directory.join(format!("{port}.sock")))
    }

    // Keep this inode permanently: unlinking locks would permit multiple owners.
    // Guardians inherit the open-file description and retain it through cleanup.
    fn acquire_lease(path: &std::path::Path) -> Result<fs::File, String> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path.with_extension("lock"))
            .map_err(|e| format!("cannot open control ownership lock: {e}"))?;
        let metadata = file
            .metadata()
            .map_err(|e| format!("control socket operation failed: {e}"))?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o600
            || metadata.nlink() != 1
        {
            return Err(
                "control ownership lock must be a private, single-link regular file".into(),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(file);
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::WouldBlock {
                return Err(format!("cannot lock control endpoint: {error}"));
            }
            if Instant::now() >= deadline {
                return Err(
                    "control endpoint is owned by a live runtime or application guardian".into(),
                );
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn recover_socket(path: &std::path::Path) -> Result<(), String> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("cannot inspect control socket: {error}")),
        };
        if !metadata.file_type().is_socket()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o600
        {
            return Err("refusing to replace an unexpected control endpoint".into());
        }
        // Protect live listeners from older runtimes that do not hold a lease too.
        match UnixStream::connect(path) {
            Ok(_) => return Err("control socket belongs to a live listener".into()),
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {}
            Err(error) => return Err(format!("cannot verify stale control socket: {error}")),
        }
        fs::remove_file(path).map_err(|e| format!("cannot remove stale control socket: {e}"))
    }

    pub struct Server {
        stop: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
        path: PathBuf,
        _lease: Arc<fs::File>,
    }

    impl Server {
        pub fn start(
            port: u16,
            handler: impl Fn(Command) -> Result<Reply, String> + Send + 'static,
        ) -> Result<Self, String> {
            let path = endpoint(port)?;
            let lease = Arc::new(acquire_lease(&path)?);
            recover_socket(&path)?;
            let listener = UnixListener::bind(&path)
                .map_err(|e| format!("cannot bind control socket {}: {e}", path.display()))?;
            // Own cleanup immediately, including failures during setup.
            let mut server = Self {
                stop: Arc::new(AtomicBool::new(false)),
                worker: None,
                path,
                _lease: lease,
            };
            fs::set_permissions(&server.path, fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("control socket operation failed: {e}"))?;
            listener
                .set_nonblocking(true)
                .map_err(|e| format!("control socket operation failed: {e}"))?;
            let stop = server.stop.clone();
            server.worker = Some(thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            // macOS inherits O_NONBLOCK from the listening socket.
                            // Normalize accepted sockets before configuring their deadlines.
                            // Framed IO switches to nonblocking reads/writes guarded by poll.
                            if stream.set_nonblocking(false).is_err() {
                                continue;
                            }
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                            let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                            let result = read_frame(&mut stream, 4096)
                                .and_then(|bytes| {
                                    serde_json::from_slice(&bytes)
                                        .map_err(|_| "invalid control request".into())
                                })
                                .and_then(&handler);
                            let reply = match result {
                                Ok(reply) => reply,
                                Err(error) => Reply {
                                    url: None,
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

    fn ready(stream: &UnixStream, events: libc::c_short, timeout: Duration) -> Result<(), String> {
        let mut descriptor = libc::pollfd {
            fd: stream.as_raw_fd(),
            events,
            revents: 0,
        };
        let result = unsafe {
            libc::poll(
                &mut descriptor,
                1,
                timeout.as_millis().max(1).min(i32::MAX as u128) as i32,
            )
        };
        if result == 0 {
            return Err("control message deadline exceeded".into());
        }
        if result < 0 {
            return Err(format!(
                "cannot poll control socket: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    fn read_frame(stream: &mut UnixStream, limit: usize) -> Result<Vec<u8>, String> {
        let timeout = stream
            .read_timeout()
            .map_err(|e| format!("control socket operation failed: {e}"))?
            .unwrap_or(Duration::from_secs(1));
        stream.set_nonblocking(true).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + timeout;
        let mut bytes = Vec::new();
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|duration| !duration.is_zero())
                .ok_or("control message read timed out")?;
            ready(stream, libc::POLLIN, remaining)?;
            let mut buffer = [0u8; 4096];
            let count = match stream.read(&mut buffer) {
                Ok(count) => count,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                Err(e) => return Err(format!("cannot read control message: {e}")),
            };
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
        let mut bytes = serde_json::to_vec(value)
            .map_err(|e| format!("control socket operation failed: {e}"))?;
        bytes.push(b'\n');
        let timeout = stream
            .write_timeout()
            .map_err(|e| format!("control socket operation failed: {e}"))?
            .unwrap_or(Duration::from_secs(1));
        stream.set_nonblocking(true).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + timeout;
        let mut remaining = bytes.as_slice();
        while !remaining.is_empty() {
            let timeout = deadline
                .checked_duration_since(Instant::now())
                .filter(|duration| !duration.is_zero())
                .ok_or("control message write timed out")?;
            ready(stream, libc::POLLOUT, timeout)?;
            let count = match stream.write(remaining) {
                Ok(count) => count,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                Err(e) => return Err(format!("cannot write control message: {e}")),
            };
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
            .map_err(|e| format!("control socket operation failed: {e}"))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .map_err(|e| format!("control socket operation failed: {e}"))?;
        write_frame(&mut stream, &command)?;
        serde_json::from_slice(&read_frame(&mut stream, 1024 * 1024)?)
            .map_err(|_| "invalid control response".into())
    }
    #[cfg(test)]
    mod ownership_tests {
        use super::*;

        #[test]
        fn reads_large_reply_after_peer_closes() {
            let (mut reader, mut writer) = UnixStream::pair().unwrap();
            reader
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let message = format!("{}\n", "x".repeat(16000));
            let expected = message.clone();
            let worker = thread::spawn(move || {
                writer.write_all(message.as_bytes()).unwrap();
            });
            assert_eq!(read_frame(&mut reader, 20000).unwrap(), expected.as_bytes());
            worker.join().unwrap();
        }

        #[test]
        fn ownership_survives_until_the_last_guardian_duplicate_closes() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("test.sock");
            let runtime = acquire_lease(&path).unwrap();
            let guardian = runtime.try_clone().unwrap();
            drop(runtime);
            let (tx, rx) = std::sync::mpsc::channel();
            let waiting = thread::spawn(move || {
                let lease = acquire_lease(&path).unwrap();
                tx.send(()).unwrap();
                lease
            });
            assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
            drop(guardian);
            rx.recv_timeout(Duration::from_secs(2)).unwrap();
            drop(waiting.join().unwrap());
        }

        #[test]
        fn ownership_rejects_symlinks_and_hardlinks_without_modifying_targets() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("test.sock");
            let target = dir.path().join("keep");
            fs::write(&target, "preserve").unwrap();
            fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
            std::os::unix::fs::symlink(&target, path.with_extension("lock")).unwrap();
            assert!(acquire_lease(&path).is_err());
            fs::remove_file(path.with_extension("lock")).unwrap();
            fs::hard_link(&target, path.with_extension("lock")).unwrap();
            assert!(acquire_lease(&path).is_err());
            assert_eq!(fs::read_to_string(target).unwrap(), "preserve");
        }
    }
}

#[cfg(not(unix))]
mod transport {
    use super::*;
    pub struct Server;
    impl Server {
        pub fn start(
            _port: u16,
            _handler: impl Fn(Command) -> Result<Reply, String> + Send + 'static,
        ) -> Result<Self, String> {
            // Preserve foreground hosting on platforms without this transport.
            Ok(Self)
        }
    }
    pub(super) fn request(_port: u16, _command: Command) -> Result<Reply, String> {
        Err("local lifecycle commands currently require Unix".into())
    }
}
