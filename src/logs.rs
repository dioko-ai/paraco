//! Persistent, bounded JSONL storage shared by local runtime instances.
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const FILE_BYTES: u64 = 2 * 1024 * 1024;
const FILE_COUNT: usize = 5;
pub const LINE_BYTES: usize = 16 * 1024;
const QUEUE_RECORDS: usize = 1024;
const MAX_PERSISTENCE_WORKERS: usize = 32;
static PERSISTENCE_WORKERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Snapshot of records deliberately lost at the asynchronous persistence boundary.
#[allow(dead_code)] // consumed by management diagnostics in the next M1 slice
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LossCounters {
    pub queue_drops: u64,
    pub truncations: u64,
    pub write_failures: u64,
    pub shutdown_discards: u64,
    /// A flush barrier whose persistence result is unknown, not a record loss.
    pub flush_timeouts: u64,
    /// A flush barrier that could not enter the bounded queue.
    pub flush_enqueue_failures: u64,
}

struct Losses {
    queue_drops: std::sync::atomic::AtomicU64,
    truncations: std::sync::atomic::AtomicU64,
    write_failures: std::sync::atomic::AtomicU64,
    #[allow(dead_code)]
    shutdown_discards: std::sync::atomic::AtomicU64,
    flush_timeouts: std::sync::atomic::AtomicU64,
    flush_enqueue_failures: std::sync::atomic::AtomicU64,
}

impl Losses {
    #[allow(dead_code)]
    fn snapshot(&self) -> LossCounters {
        LossCounters {
            queue_drops: self.queue_drops.load(Ordering::Relaxed),
            truncations: self.truncations.load(Ordering::Relaxed),
            write_failures: self.write_failures.load(Ordering::Relaxed),
            shutdown_discards: self.shutdown_discards.load(Ordering::Relaxed),
            flush_timeouts: self.flush_timeouts.load(Ordering::Relaxed),
            flush_enqueue_failures: self.flush_enqueue_failures.load(Ordering::Relaxed),
        }
    }
}

enum QueueItem {
    Record(Record),
    Flush(mpsc::SyncSender<()>),
}

struct Sink {
    sender: SyncSender<QueueItem>,
    losses: Arc<Losses>,
}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
    file_bytes: u64,
    sink: Option<Arc<Sink>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Record {
    pub schema_version: u8,
    pub timestamp_unix_ms: u64,
    pub app: String,
    pub mode: String,
    pub port: u16,
    pub run_id: String,
    pub pid: Option<u32>,
    pub stream: String,
    pub event: String,
    pub message: String,
    pub truncated: bool,
}

#[derive(Clone)]
pub struct AppLog {
    store: Store,
    app: String,
    mode: String,
    port: u16,
    run_id: String,
    pid: Arc<AtomicU32>,
}

pub fn directory(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        if path.as_os_str().is_empty() {
            return Err("log directory must not be empty".into());
        }
        return Ok(path.to_owned());
    }
    if let Some(path) = std::env::var_os("PARACO_LOG_DIR") {
        if path.is_empty() {
            return Err("PARACO_LOG_DIR must not be empty".into());
        }
        return Ok(path.into());
    }
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let home = std::env::var_os(variable)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{variable} is unset; specify --log-dir or PARACO_LOG_DIR"))?;
    Ok(PathBuf::from(home).join(".paraco").join("logs"))
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::open_with_limit(path, FILE_BYTES)
    }

    fn open_with_limit(path: &Path, file_bytes: u64) -> Result<Self, String> {
        Self::open_inner(path, file_bytes)
    }

    fn open_inner(path: &Path, file_bytes: u64) -> Result<Self, String> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(path)
            .map_err(|e| format!("cannot create log directory: {e}"))?;
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !metadata.is_dir() {
            return Err("log directory must be a real directory, not a symlink".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
                return Err(
                    "log directory must be owned by the current user with permissions 0700".into(),
                );
            }
        }
        let bare = Self {
            path: path.canonicalize().map_err(|e| e.to_string())?,
            file_bytes,
            sink: None,
        };
        let _lock = bare.lock().map_err(|e| format!("cannot lock logs: {e}"))?;
        bare.repair()
            .map_err(|e| format!("cannot initialize logs: {e}"))?;
        let (sender, receiver) = mpsc::sync_channel(QUEUE_RECORDS);
        let losses = Arc::new(Losses {
            queue_drops: std::sync::atomic::AtomicU64::new(0),
            truncations: std::sync::atomic::AtomicU64::new(0),
            write_failures: std::sync::atomic::AtomicU64::new(0),
            shutdown_discards: std::sync::atomic::AtomicU64::new(0),
            flush_timeouts: std::sync::atomic::AtomicU64::new(0),
            flush_enqueue_failures: std::sync::atomic::AtomicU64::new(0),
        });
        if PERSISTENCE_WORKERS
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |count| {
                (count < MAX_PERSISTENCE_WORKERS).then_some(count + 1)
            })
            .is_err()
        {
            return Err(
                "persistent log worker limit reached; retry after an existing store closes".into(),
            );
        }
        let worker_store = bare.clone();
        let worker_losses = losses.clone();
        thread::spawn(move || {
            while let Ok(item) = receiver.recv() {
                match item {
                    QueueItem::Record(record) => {
                        if worker_store.append(&record).is_err() {
                            // Never synchronously fall back to stderr: it may be the blocked sink.
                            worker_losses.write_failures.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    QueueItem::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
            PERSISTENCE_WORKERS.fetch_sub(1, Ordering::AcqRel);
        });
        Ok(Self {
            sink: Some(Arc::new(Sink { sender, losses })),
            ..bare
        })
    }

    #[cfg(test)]
    fn with_blocked_writer(path: &Path) -> (Self, mpsc::Receiver<()>, mpsc::SyncSender<()>) {
        let mut store = Self::open(path).unwrap();
        let bare = Self {
            path: store.path.clone(),
            file_bytes: store.file_bytes,
            sink: None,
        };
        let (sender, receiver) = mpsc::sync_channel(QUEUE_RECORDS);
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let losses = Arc::new(Losses {
            queue_drops: std::sync::atomic::AtomicU64::new(0),
            truncations: std::sync::atomic::AtomicU64::new(0),
            write_failures: std::sync::atomic::AtomicU64::new(0),
            shutdown_discards: std::sync::atomic::AtomicU64::new(0),
            flush_timeouts: std::sync::atomic::AtomicU64::new(0),
            flush_enqueue_failures: std::sync::atomic::AtomicU64::new(0),
        });
        thread::spawn(move || {
            if let Ok(QueueItem::Record(record)) = receiver.recv() {
                let _ = entered_tx.send(());
                let _ = release_rx.recv();
                let _ = bare.append(&record);
            }
        });
        store.sink = Some(Arc::new(Sink { sender, losses }));
        (store, entered_rx, release_tx)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Live store-scoped counters; they reset when this runtime/store closes.
    pub fn losses(&self) -> LossCounters {
        self.sink
            .as_ref()
            .map(|sink| sink.losses.snapshot())
            .unwrap_or_default()
    }

    pub fn outside(&self, app_root: &Path) -> Result<(), String> {
        if self.path.starts_with(app_root) {
            return Err("log directory must be outside application directories".into());
        }
        Ok(())
    }

    pub fn app(&self, app: &str, mode: &str, port: u16) -> Result<AppLog, String> {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| "cannot generate log run ID")?;
        Ok(AppLog {
            store: self.clone(),
            app: app.into(),
            mode: mode.into(),
            port,
            run_id: random.iter().map(|b| format!("{b:02x}")).collect(),
            pid: Arc::new(AtomicU32::new(0)),
        })
    }

    fn file(&self, index: usize) -> PathBuf {
        self.path.join(if index == 0 {
            "current.jsonl".into()
        } else {
            format!("archive-{index}.jsonl")
        })
    }

    // Each operation opens its own lock handle, serializing both threads and
    // independent processes. OS locks release automatically after a crash.
    fn lock(&self) -> io::Result<File> {
        let file = private_file(&self.path.join(".lock"), true)?;
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(2))
                }
                Err(error) => return Err(io::Error::other(error)),
            }
        }
    }

    fn repair(&self) -> io::Result<()> {
        // Remove an incomplete last record left by a killed writer. Also check
        // all recognized archives before touching any during rotation.
        for index in 0..FILE_COUNT {
            let path = self.file(index);
            if index != 0 && !path.try_exists()? {
                continue;
            }
            let mut file = private_file(&path, index == 0)?;
            let len = file.metadata()?.len();
            if len > self.file_bytes {
                return Err(io::Error::other(
                    "existing log file exceeds the storage limit",
                ));
            }
            if len == 0 {
                continue;
            }
            let count = len.min(self.file_bytes) as usize;
            file.seek(SeekFrom::End(-(count as i64)))?;
            let mut bytes = vec![0; count];
            file.read_exact(&mut bytes)?;
            let valid = bytes
                .iter()
                .rposition(|b| *b == b'\n')
                .map_or(0, |pos| pos + 1);
            file.set_len(len - count as u64 + valid as u64)?;
        }
        Ok(())
    }

    fn append(&self, record: &Record) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(record)?;
        bytes.push(b'\n');
        if bytes.len() as u64 > self.file_bytes {
            return Err(io::Error::other("log record exceeds file limit"));
        }
        let _lock = self.lock()?;
        // Repair only the active tail on append (a previous writer may have died).
        let path = self.file(0);
        let mut file = private_file(&path, true)?;
        let mut length = file.metadata()?.len();
        if length > self.file_bytes {
            return Err(io::Error::other(
                "existing log file exceeds the storage limit",
            ));
        }
        if length > 0 {
            file.seek(SeekFrom::End(-1))?;
            let mut last = [0];
            file.read_exact(&mut last)?;
            if last[0] != b'\n' {
                drop(file);
                self.repair()?;
                file = private_file(&path, false)?;
                length = file.metadata()?.len();
            }
        }
        if length + bytes.len() as u64 > self.file_bytes {
            drop(file);
            // Fixed names and a fixed count bound the entire store, across all
            // apps, launches, and server ports. Never prune unrelated files.
            for index in (1..FILE_COUNT).rev() {
                let target = self.file(index);
                if target.try_exists()? {
                    private_file(&target, false)?;
                    fs::remove_file(&target)?;
                }
                let source = self.file(index - 1);
                if source.try_exists()? {
                    private_file(&source, false)?;
                    fs::rename(source, target)?;
                }
            }
            file = private_file(&path, true)?;
        }
        let original = file.metadata()?.len();
        file.seek(SeekFrom::End(0))?;
        if let Err(error) = file.write_all(&bytes) {
            let _ = file.set_len(original);
            return Err(error);
        }
        Ok(())
    }

    pub fn tail(
        &self,
        app: Option<&str>,
        port: Option<u16>,
        count: usize,
    ) -> Result<Vec<Record>, String> {
        self.flush(Duration::from_secs(2));
        self.tail_launch(app, port, None, count)
    }

    pub fn tail_launch(
        &self,
        app: Option<&str>,
        port: Option<u16>,
        run_id: Option<&str>,
        count: usize,
    ) -> Result<Vec<Record>, String> {
        self.flush(Duration::from_secs(2));
        if count > 10_000 {
            return Err("--tail must be at most 10000".into());
        }
        if count == 0 {
            return Ok(vec![]);
        }
        let _lock = self.lock().map_err(|e| e.to_string())?;
        let mut records = VecDeque::new();
        for index in (0..FILE_COUNT).rev() {
            let path = self.file(index);
            if !path.try_exists().map_err(|e| e.to_string())? {
                continue;
            }
            let file = private_file(&path, false).map_err(|e| e.to_string())?;
            if file.metadata().map_err(|e| e.to_string())?.len() > self.file_bytes {
                return Err("log file exceeds storage limit".into());
            }
            for line in BufReader::new(file).lines() {
                let line = line.map_err(|e| e.to_string())?;
                let Ok(record) = serde_json::from_str::<Record>(&line) else {
                    continue;
                };
                if app.is_none_or(|name| name == record.app)
                    && port.is_none_or(|p| p == record.port)
                    && run_id.is_none_or(|id| id == record.run_id)
                {
                    if records.len() == count {
                        records.pop_front();
                    }
                    records.push_back(record);
                }
            }
        }
        Ok(records.into_iter().collect())
    }

    /// Best-effort bounded persistence barrier for readers and tests. It never
    /// waits for a filesystem operation already stalled in the sink worker.
    fn flush(&self, timeout: Duration) {
        let Some(sink) = &self.sink else { return };
        let (done_tx, done_rx) = mpsc::sync_channel(0);
        match sink.sender.try_send(QueueItem::Flush(done_tx)) {
            Ok(()) if done_rx.recv_timeout(timeout).is_err() => {
                sink.losses.flush_timeouts.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                sink.losses
                    .flush_enqueue_failures
                    .fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }
}

fn private_file(path: &Path, create: bool) -> io::Result<File> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.is_file() {
            return Err(io::Error::other(
                "log files must be regular files, not symlinks",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o077 != 0
                || metadata.nlink() != 1
            {
                return Err(io::Error::other(
                    "log files must be private, owned by the current user, and not hard links",
                ));
            }
        }
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

impl AppLog {
    pub fn pid(&self, pid: u32) {
        self.pid.store(pid, Ordering::Relaxed);
    }
    pub fn event(&self, event: &str, message: &str) {
        self.write("supervisor", event, message, false);
    }
    /// Optional deadline-bounded lifecycle flush. It is never a shutdown
    /// dependency: a full or stalled queue is recorded as a discard instead.
    pub fn flush(&self, timeout: Duration) {
        let Some(sink) = &self.store.sink else { return };
        let (done_tx, done_rx) = mpsc::sync_channel(0);
        match sink.sender.try_send(QueueItem::Flush(done_tx)) {
            Ok(()) if done_rx.recv_timeout(timeout).is_err() => {
                sink.losses.flush_timeouts.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                sink.losses
                    .flush_enqueue_failures
                    .fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }
    #[allow(dead_code)]
    pub fn losses(&self) -> LossCounters {
        self.store
            .sink
            .as_ref()
            .map(|sink| sink.losses.snapshot())
            .unwrap_or_default()
    }
    pub fn write(&self, stream: &str, event: &str, message: &str, truncated: bool) {
        let end = message.floor_char_boundary(message.len().min(LINE_BYTES));
        let pid = self.pid.load(Ordering::Relaxed);
        let was_truncated = truncated || end < message.len();
        let record = Record {
            schema_version: 1,
            timestamp_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            app: self.app.clone(),
            mode: self.mode.clone(),
            port: self.port,
            run_id: self.run_id.clone(),
            pid: (pid != 0).then_some(pid),
            stream: stream.into(),
            event: event.into(),
            message: message[..end].into(),
            truncated: was_truncated,
        };
        let Some(sink) = &self.store.sink else { return };
        if was_truncated {
            sink.losses.truncations.fetch_add(1, Ordering::Relaxed);
        }
        match sink.sender.try_send(QueueItem::Record(record)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                sink.losses.queue_drops.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {
                sink.losses
                    .shutdown_discards
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

pub fn print(
    path: &Path,
    app: Option<&str>,
    port: Option<u16>,
    count: usize,
) -> Result<(), String> {
    let store = Store::open(path)?;
    let records = store.tail(app, port, count)?;
    let stdout = io::stdout();
    let mut output = stdout.lock();
    for record in records {
        serde_json::to_writer(&mut output, &record).map_err(|e| e.to_string())?;
        writeln!(output).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Emit a bounded prefix and discard the remainder of an oversized line.
/// Invalid UTF-8 is replaced by the caller; no unbounded line allocations.
pub fn lines(reader: impl Read, mut emit: impl FnMut(&[u8], bool)) -> io::Result<()> {
    let mut reader = BufReader::new(reader);
    let mut line = Vec::with_capacity(LINE_BYTES);
    let mut discarding = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            if !discarding && !line.is_empty() {
                emit(&line, false);
            }
            return Ok(());
        }
        let newline = buffer.iter().position(|b| *b == b'\n');
        let count = newline.map_or(buffer.len(), |n| n + 1);
        let data = &buffer[..newline.unwrap_or(count)];
        if !discarding {
            let available = LINE_BYTES - line.len();
            line.extend_from_slice(&data[..data.len().min(available)]);
            if data.len() > available {
                emit(&line, true);
                line.clear();
                discarding = true;
            }
        }
        if newline.is_some() {
            if !discarding {
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                emit(&line, false);
            }
            line.clear();
            discarding = false;
        }
        reader.consume(count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for_records(store: &Store, expected: usize) -> Vec<Record> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let records = store.tail(None, None, 1000).unwrap();
            if records.len() >= expected || Instant::now() >= deadline {
                return records;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn blocked_writer_drops_bounded_records_without_blocking_producers() {
        let dir = tempfile::tempdir().unwrap();
        let (store, entered, release) = Store::with_blocked_writer(&dir.path().join("logs"));
        let log = store.app("hello", "run", 3000).unwrap();
        log.event("starting", "first record blocks the fake writer");
        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        let done = thread::spawn({
            let log = log.clone();
            move || {
                for number in 0..(QUEUE_RECORDS * 2) {
                    log.write("stdout", "output", &format!("{number}"), false);
                }
            }
        });
        done.join().unwrap(); // an external watchdog would fail this if try_send regressed to send.
        assert!(log.losses().queue_drops > 0);
        release.send(()).unwrap();
    }

    #[test]
    fn timed_out_flush_is_an_unknown_outcome_not_a_discard() {
        let dir = tempfile::tempdir().unwrap();
        let (store, entered, release) = Store::with_blocked_writer(&dir.path().join("logs"));
        let log = store.app("hello", "run", 3000).unwrap();
        log.event("starting", "block the writer");
        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        log.flush(Duration::from_millis(1));
        assert_eq!(log.losses().flush_timeouts, 1);
        assert_eq!(log.losses().shutdown_discards, 0);
        release.send(()).unwrap();
        log.flush(Duration::from_secs(1));
        assert_eq!(log.losses().shutdown_discards, 0);
    }

    #[test]
    fn rotates_prunes_and_keeps_latest_records_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_with_limit(&dir.path().join("logs"), 1024).unwrap();
        fs::write(store.path().join("unrelated.txt"), "keep me").unwrap();
        let log = store.app("hello", "serve", 3000).unwrap();
        for n in 0..100 {
            log.write(
                "stdout",
                "output",
                &format!("record {n}: {}", "x".repeat(150)),
                false,
            );
        }
        let tail = store.tail(Some("hello"), Some(3000), 3).unwrap();
        assert_eq!(tail.len(), 3);
        assert!(tail[0].message.starts_with("record 97:"));
        assert!(tail[2].message.starts_with("record 99:"));
        let mut total = 0;
        for index in 0..FILE_COUNT {
            let bytes = fs::read(store.file(index)).unwrap();
            assert!(bytes.len() <= 1024);
            total += bytes.len();
            for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
                serde_json::from_slice::<Record>(line).unwrap();
            }
        }
        assert!(total <= 5 * 1024);
        assert_eq!(
            fs::read_to_string(store.path().join("unrelated.txt")).unwrap(),
            "keep me"
        );
        assert_eq!(fs::read_dir(store.path()).unwrap().count(), 7);
        let reopened = Store::open(&dir.path().join("logs")).unwrap();
        assert!(
            reopened.tail(None, None, 1).unwrap()[0]
                .message
                .starts_with("record 99:")
        );
        assert!(reopened.tail(Some("other"), None, 1).unwrap().is_empty());
        assert!(reopened.tail(None, Some(3001), 1).unwrap().is_empty());
        assert!(reopened.tail(None, None, 10001).is_err());
    }

    #[test]
    fn independent_store_handles_serialize_writes_and_escape_messages() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("logs")).unwrap();
        let workers: Vec<_> = (0..4)
            .map(|n| {
                let independent = Store::open(&dir.path().join("logs")).unwrap();
                thread::spawn(move || {
                    let log = independent
                        .app(&format!("app-{n}"), "run", 3000 + n)
                        .unwrap();
                    log.pid(42);
                    for i in 0..50 {
                        log.write(
                            "stderr",
                            "output",
                            &format!("{i}\n\"quoted\"\u{0} 😀"),
                            false,
                        );
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let records = wait_for_records(&store, 200);
        assert_eq!(records.len(), 200);
        for record in records {
            assert!(record.timestamp_unix_ms > 0);
            assert_eq!(record.pid, Some(42));
            assert_eq!(record.run_id.len(), 32);
            assert!(record.message.contains('\n'));
        }
        assert_eq!(store.tail(Some("app-2"), None, 100).unwrap().len(), 50);
    }

    #[test]
    fn repairs_interrupted_append_before_next_write_and_after_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("logs")).unwrap();
        let log = store.app("hello", "serve", 3000).unwrap();
        log.event("running", "first");
        OpenOptions::new()
            .append(true)
            .open(store.file(0))
            .unwrap()
            .write_all(b"{\"partial\":")
            .unwrap();
        log.event("stopped", "second");
        assert_eq!(store.tail(None, None, 10).unwrap().len(), 2);
        OpenOptions::new()
            .append(true)
            .open(store.file(0))
            .unwrap()
            .write_all(b"unfinished")
            .unwrap();
        let reopened = Store::open(&dir.path().join("logs")).unwrap();
        assert_eq!(reopened.tail(None, None, 10).unwrap().len(), 2);
        assert!(fs::read(store.file(0)).unwrap().ends_with(b"\n"));
    }

    #[test]
    fn busy_store_has_a_bounded_wait() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("logs")).unwrap();
        let _held = store.lock().unwrap();
        let start = Instant::now();
        assert!(store.lock().is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn bounded_lines_handle_large_output_invalid_utf8_and_partial_eof() {
        let mut bytes = b"normal\r\n\n".to_vec();
        bytes.extend(vec![b'x'; LINE_BYTES * 20]);
        bytes.extend_from_slice(b"\n\xff\nlast");
        let mut output = vec![];
        lines(bytes.as_slice(), |line, truncated| {
            output.push((line.to_vec(), truncated))
        })
        .unwrap();
        assert_eq!(output.len(), 5);
        assert_eq!(output[0], (b"normal".to_vec(), false));
        assert_eq!(output[1], (vec![], false));
        assert_eq!(output[2].0.len(), LINE_BYTES);
        assert!(output[2].1);
        assert_eq!(String::from_utf8_lossy(&output[3].0), "�");
        assert_eq!(output[4], (b"last".to_vec(), false));
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("logs")).unwrap();
        store.app("hello", "run", 3000).unwrap().write(
            "stdout",
            "output",
            &"😀".repeat(LINE_BYTES),
            false,
        );
        let record = store.tail(None, None, 1).unwrap().remove(0);
        assert!(record.truncated);
        assert!(record.message.len() <= LINE_BYTES);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_public_symlinked_and_hardlinked_storage() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs");
        let store = Store::open(&path).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(store.file(0)).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let target = dir.path().join("target");
        fs::write(&target, "preserve").unwrap();
        fs::remove_file(store.file(0)).unwrap();
        symlink(&target, store.file(0)).unwrap();
        assert!(Store::open(&path).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "preserve");
        fs::remove_file(store.file(0)).unwrap();
        fs::hard_link(&target, store.file(0)).unwrap();
        assert!(Store::open(&path).is_err());
        let alias = dir.path().join("alias");
        symlink(&path, &alias).unwrap();
        assert!(Store::open(&alias).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Store::open(&path).is_err());
    }
}
