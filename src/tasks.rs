//! Durable reviewed tasks, transactional occurrence claims and bounded fresh processes.
use chrono::{Datelike, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_LOG: usize = 65536;
const MAX_RUNS: i64 = 100;
const MAX_CONCURRENCY: usize = 4;
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Schedule {
    Manual,
    Once {
        at: i64,
    },
    Cron {
        expression: String,
        timezone: String,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    #[serde(default)]
    pub job_id: Option<String>,
    pub name: String,
    pub source: String,
    pub schedule: Schedule,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub notify_success: bool,
    #[serde(default = "yes")]
    pub notify_failure: bool,
}
fn default_timeout() -> u64 {
    60_000
}
fn yes() -> bool {
    true
}
struct Running {
    cancel: Arc<AtomicBool>,
    output: Arc<Mutex<(Vec<u8>, bool)>>,
    events: Arc<Mutex<Vec<String>>>,
}
pub struct Tasks {
    db: Arc<Mutex<Connection>>,
    root: PathBuf,
    workerd: PathBuf,
    running: Mutex<HashMap<String, Running>>,
    artifacts: Mutex<()>,
    stop: AtomicBool,
    grants: Mutex<HashMap<String, Vec<String>>>,
    lease: Mutex<Option<fs::File>>,
    scheduler: Mutex<Option<thread::JoinHandle<()>>>,
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
fn id(prefix: &str) -> Result<String, String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(err)?;
    Ok(format!(
        "{prefix}{}",
        b.iter().map(|v| format!("{v:02x}")).collect::<String>()
    ))
}
impl Tasks {
    pub fn open(root: &Path, workerd: PathBuf) -> Result<Arc<Self>, String> {
        let db = Connection::open(root.join("state.sqlite3")).map_err(err)?;
        db.busy_timeout(Duration::from_secs(3)).map_err(err)?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;
          CREATE TABLE IF NOT EXISTS task_schema(version INTEGER NOT NULL);
          INSERT INTO task_schema SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM task_schema);
          CREATE TABLE IF NOT EXISTS task_jobs(id TEXT PRIMARY KEY,deployment TEXT NOT NULL,name TEXT NOT NULL,current TEXT,archived INTEGER NOT NULL DEFAULT 0,next_at INTEGER,once_claimed INTEGER NOT NULL DEFAULT 0);
          CREATE TABLE IF NOT EXISTS task_revisions(id TEXT PRIMARY KEY,job TEXT NOT NULL,deployment TEXT NOT NULL,body TEXT NOT NULL,digest TEXT NOT NULL,created INTEGER NOT NULL,published INTEGER);
          CREATE TABLE IF NOT EXISTS task_runs(id TEXT PRIMARY KEY,job TEXT NOT NULL,deployment TEXT NOT NULL,revision TEXT NOT NULL,occurrence TEXT NOT NULL UNIQUE,status TEXT NOT NULL,started INTEGER NOT NULL,ended INTEGER,logs TEXT NOT NULL DEFAULT '',truncated INTEGER NOT NULL DEFAULT 0,diagnostic TEXT);
          CREATE UNIQUE INDEX IF NOT EXISTS task_busy ON task_runs(job) WHERE status IN ('pending','running');
          CREATE TABLE IF NOT EXISTS task_notifications(id TEXT PRIMARY KEY,deployment TEXT NOT NULL,run TEXT NOT NULL,message TEXT NOT NULL,created INTEGER NOT NULL,read INTEGER NOT NULL DEFAULT 0,delivery TEXT NOT NULL DEFAULT 'local');
          CREATE TABLE IF NOT EXISTS task_watermarks(job TEXT NOT NULL,revision TEXT NOT NULL,occurrence TEXT NOT NULL,PRIMARY KEY(job,revision));
          CREATE TABLE IF NOT EXISTS task_clock(id INTEGER PRIMARY KEY CHECK(id=1),last_tick INTEGER);
          INSERT OR IGNORE INTO task_clock VALUES(1,NULL);") .map_err(err)?;
        let version: i64 = db
            .query_row("SELECT version FROM task_schema", [], |r| r.get(0))
            .map_err(err)?;
        if version != 1 {
            return Err("unsupported task schema".into());
        }
        // The runtime lease prevents reopening this database until old guardians
        // have reaped children. Persisted PIDs are never used as ownership proof.
        db.execute("UPDATE task_runs SET status='interrupted',ended=?1,diagnostic='Runtime stopped before a result was established; not replayed' WHERE status IN ('pending','running')",[now()]).map_err(err)?;
        let root = root.join("tasks");
        fs::create_dir_all(&root).map_err(err)?;
        let tasks = Arc::new(Self {
            db: Arc::new(Mutex::new(db)),
            root,
            workerd,
            running: Mutex::new(HashMap::new()),
            artifacts: Mutex::new(()),
            stop: AtomicBool::new(false),
            grants: Mutex::new(HashMap::new()),
            lease: Mutex::new(None),
            scheduler: Mutex::new(None),
        });
        {
            let _artifacts = tasks.artifacts.lock().unwrap();
            let db = tasks.db.lock().unwrap();
            let _ = tasks.cleanup_artifacts(&db);
        }
        Ok(tasks)
    }
    pub fn set_grant(&self, deployment: String, network: Vec<String>) {
        self.grants.lock().unwrap().insert(deployment, network);
    }
    fn allowed(&self, deployment: &str, network: &[String]) -> Result<(), String> {
        let grants = self.grants.lock().unwrap();
        let permitted = grants
            .get(deployment)
            .ok_or("task authoring grant is absent or withdrawn")?;
        if network.iter().any(|host| !permitted.contains(host)) {
            return Err("task requests network access outside deployment grants".into());
        }
        Ok(())
    }
    pub fn set_lease(&self, lease: fs::File) {
        *self.lease.lock().unwrap() = Some(lease);
    }
    pub fn start(self: &Arc<Self>) {
        let this = self.clone();
        let mut slot = self.scheduler.lock().unwrap();
        if slot.is_some() {
            return;
        }
        *slot = Some(thread::spawn(move || {
            while !this.stop.load(Ordering::Relaxed) {
                let _ = this.tick(now());
                thread::sleep(Duration::from_millis(250));
            }
        }));
    }
    pub fn shutdown(&self) {
        {
            let running = self.running.lock().unwrap();
            self.stop.store(true, Ordering::Relaxed);
            for r in running.values() {
                r.cancel.store(true, Ordering::Relaxed);
            }
        }
        let _=self.db.lock().unwrap().execute("UPDATE task_runs SET status='cancelled',ended=?1,diagnostic='Runtime shutting down before dispatch' WHERE status='pending'",[now()]);
        if let Some(handle) = self.scheduler.lock().unwrap().take() {
            let _ = handle.join();
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        while !self.running.lock().unwrap().is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
    }
    pub fn command(
        self: &Arc<Self>,
        deployment: &str,
        action: &str,
        body: Value,
    ) -> Result<Value, String> {
        if self.stop.load(Ordering::Relaxed) {
            return Err("task engine is shutting down".into());
        }
        if deployment.is_empty() {
            return Err("deployment required".into());
        }
        self.allowed(deployment, &[])?;
        match action {
            "draft"=>{let draft:Draft=serde_json::from_value(body).map_err(err)?;self.draft(deployment,draft)},
            "publish"=>self.publish(deployment,&body),
            "list"=>self.rows("SELECT json_object('id',id,'name',name,'current_revision',current,'archived',json(CASE archived WHEN 1 THEN 'true' ELSE 'false' END),'next_at',next_at,'once_claimed',once_claimed) FROM task_jobs WHERE deployment=?1 ORDER BY name",deployment,None),
            "revisions"=>{let job=string(&body,"job_id")?;self.owned(deployment,job)?;let offset=body.get("offset").and_then(Value::as_u64).unwrap_or(0).min(2000);self.rows(&format!("SELECT json_object('id',id,'job_id',job,'body',json(body),'digest',digest,'created',created,'published',published) FROM task_revisions WHERE deployment=?1 AND job=?2 ORDER BY created DESC,rowid DESC LIMIT 10 OFFSET {offset}"),deployment,Some(job))},
            "history"=>{let job=body.get("job_id").and_then(Value::as_str);if let Some(job)=job{self.owned(deployment,job)?;}let offset=body.get("offset").and_then(Value::as_u64).unwrap_or(0).min(1000000);self.rows(&format!("SELECT json_object('id',id,'job_id',job,'revision_id',revision,'occurrence',occurrence,'status',status,'started',started,'ended',ended,'logs',substr(logs,1,1024),'logs_preview',1,'truncated',truncated,'diagnostic',diagnostic) FROM task_runs WHERE deployment=?1 AND (?2 IS NULL OR job=?2) ORDER BY started DESC,rowid DESC LIMIT 500 OFFSET {offset}"),deployment,job)},
            "revision"=>{let revision=string(&body,"revision_id")?;let db=self.db.lock().unwrap();let result:String=db.query_row("SELECT json_object('id',id,'job_id',job,'body',json(body),'digest',digest,'created',created,'published',published) FROM task_revisions WHERE id=?1 AND deployment=?2",params![revision,deployment],|r|r.get(0)).map_err(err)?;serde_json::from_str(&result).map_err(err)},
            "logs"=>{let run=string(&body,"run_id")?;let db=self.db.lock().unwrap();let result:String=db.query_row("SELECT json_object('id',id,'logs',logs,'truncated',truncated,'diagnostic',diagnostic) FROM task_runs WHERE id=?1 AND deployment=?2",params![run,deployment],|r|r.get(0)).map_err(err)?;drop(db);let mut value:Value=serde_json::from_str(&result).map_err(err)?;self.live(&mut value,true);Ok(value)},
            "notifications"=>self.rows("SELECT json_object('id',id,'run_id',run,'message',message,'created',created,'read',read,'delivery',delivery) FROM task_notifications WHERE deployment=?1 ORDER BY created DESC LIMIT 500",deployment,None),
            "read_notification"=>{let db=self.db.lock().unwrap();db.execute("UPDATE task_notifications SET read=1 WHERE deployment=?1 AND id=?2",params![deployment,string(&body,"id")?]).map_err(err)?;Ok(json!({"ok":true}))},
            "archive"|"unarchive"=>self.archive(deployment,string(&body,"job_id")?,action=="archive"),
            "delete"=>self.delete(deployment,string(&body,"job_id")?),
            "run"=>{let job=string(&body,"job_id")?;self.owned(deployment,job)?;let run=self.admit(job,&format!("manual:{}",id("o")?),now(),true)?;self.dispatch_pending()?;Ok(json!({"run_id":run}))},
            "cancel"=>{let run=string(&body,"run_id")?;let db=self.db.lock().unwrap();let owner:Option<String>=db.query_row("SELECT deployment FROM task_runs WHERE id=?1",[run],|r|r.get(0)).optional().map_err(err)?;if owner.as_deref()!=Some(deployment){return Err("unknown run".into())}db.execute("UPDATE task_runs SET status='cancelled',ended=?2 WHERE id=?1 AND status='pending'",params![run,now()]).map_err(err)?;drop(db);if let Some(r)=self.running.lock().unwrap().get(run){r.cancel.store(true,Ordering::Relaxed);}Ok(json!({"ok":true}))},
            "preview"=>{let schedule:Schedule=serde_json::from_value(body.get("schedule").cloned().ok_or("schedule required")?).map_err(err)?;let mut at=body.get("after").and_then(Value::as_i64).unwrap_or_else(now);let mut dates=vec![];for _ in 0..5{if let Some(next)=next(&schedule,at)?{dates.push(next);at=next;}else{break}}Ok(json!({"times":dates}))},
            "status"=>{let db=self.db.lock().unwrap();let last:Option<i64>=db.query_row("SELECT last_tick FROM task_clock WHERE id=1",[],|r|r.get(0)).map_err(err)?;Ok(json!({"last_tick":last,"global_concurrency":MAX_CONCURRENCY,"default_timeout_ms":default_timeout(),"max_timeout_ms":3600000,"max_log_bytes":MAX_LOG,"runs_retained_per_job":1000,"log_runs_retained_per_job":MAX_RUNS,"machine_off":"Jobs cannot execute while the machine or runtime is off. Missed recurring runs are skipped."}))},
            _=>Err("unknown task action".into())
        }
    }
    fn rows(&self, sql: &str, deployment: &str, job: Option<&str>) -> Result<Value, String> {
        let db = self.db.lock().unwrap();
        let mut q = db.prepare(sql).map_err(err)?;
        let mut rows = if q.parameter_count() == 2 {
            q.query(params![deployment, job])
        } else {
            q.query([deployment])
        }
        .map_err(err)?;
        let mut out = vec![];
        while let Some(row) = rows.next().map_err(err)? {
            let s: String = row.get(0).map_err(err)?;
            out.push(serde_json::from_str::<Value>(&s).map_err(err)?);
        }
        drop(rows);
        drop(q);
        drop(db);
        for value in &mut out {
            self.live(value, false);
        }
        Ok(Value::Array(out))
    }
    fn live(&self, value: &mut Value, full: bool) {
        if value.get("logs").is_none() {
            return;
        }
        let Some(id) = value.get("id").and_then(Value::as_str) else {
            return;
        };
        let running = self.running.lock().unwrap();
        if let Some(state) = running.get(id) {
            let output = state.output.lock().unwrap();
            let logs = String::from_utf8_lossy(&output.0);
            value["logs"] = json!(if full {
                logs.into_owned()
            } else {
                logs.chars().take(1024).collect()
            });
            value["truncated"] = json!(output.1);
        }
    }
    fn owned(&self, deployment: &str, job: &str) -> Result<(), String> {
        let db = self.db.lock().unwrap();
        let yes: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM task_jobs WHERE id=?1 AND deployment=?2)",
                params![job, deployment],
                |r| r.get(0),
            )
            .map_err(err)?;
        if yes {
            Ok(())
        } else {
            Err("unknown job".into())
        }
    }
    fn draft(&self, deployment: &str, mut draft: Draft) -> Result<Value, String> {
        if draft.name.trim().is_empty()
            || draft.name.len() > 160
            || draft.source.len() > 262144
            || draft.source.is_empty()
        {
            return Err("name required (160 characters max); source required (256 KiB max)".into());
        }
        if !(100..=3_600_000).contains(&draft.timeout_ms) {
            return Err("timeout must be between 100 and 3600000 ms".into());
        }
        if draft.network.len() > 32
            || draft.network.iter().any(|s| {
                s.is_empty()
                    || s.contains('/')
                    || s.contains('*')
                    || s.contains(',')
                    || s.starts_with('-')
            })
        {
            return Err("network grants must be explicit host[:port] values; no wildcard".into());
        }
        for host in &draft.network {
            let parsed =
                url::Url::parse(&format!("http://{host}")).map_err(|_| "invalid network grant")?;
            if parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
                || host.chars().any(char::is_whitespace)
            {
                return Err("invalid network grant".into());
            }
        }
        self.allowed(deployment, &draft.network)?;
        next(&draft.schedule, now())?;
        if let Some(job) = &draft.job_id {
            self.owned(deployment, job)?;
        }
        let existing_job = draft.job_id.is_some();
        let job = draft.job_id.clone().unwrap_or(id("j")?);
        draft.job_id = Some(job.clone());
        let revision = id("v")?;
        let _artifacts = self.artifacts.lock().unwrap();
        let body = serde_json::to_string(&draft).map_err(err)?;
        let digest = format!("{:x}", Sha256::digest(body.as_bytes()));
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(err)?;
        if let Some(existing) = draft.job_id.as_deref() {
            let owner: Option<String> = tx
                .query_row(
                    "SELECT deployment FROM task_jobs WHERE id=?1",
                    [existing],
                    |r| r.get(0),
                )
                .optional()
                .map_err(err)?;
            if owner.as_deref().is_some_and(|owner| owner != deployment) {
                return Err("unknown job".into());
            }
            // A supplied job must still exist after the earlier ownership check.
            if owner.is_none() && existing_job {
                return Err("unknown job".into());
            }
        }
        tx.execute(
            "INSERT OR IGNORE INTO task_jobs(id,deployment,name) VALUES(?1,?2,?3)",
            params![job, deployment, draft.name],
        )
        .map_err(err)?;
        tx.execute(
            "INSERT INTO task_revisions VALUES(?1,?2,?3,?4,?5,?6,NULL)",
            params![revision, job, deployment, body, digest, now()],
        )
        .map_err(err)?;
        // Keep each job's newest unpublished revision, current revisions, and up to
        // 100 additional recent drafts. Rowid breaks ties when timestamps match.
        tx.execute("DELETE FROM task_revisions WHERE deployment=?1 AND published IS NULL AND id NOT IN (SELECT current FROM task_jobs WHERE current IS NOT NULL) AND rowid NOT IN (SELECT max(rowid) FROM task_revisions WHERE deployment=?1 AND published IS NULL GROUP BY job) AND rowid NOT IN (SELECT rowid FROM task_revisions WHERE deployment=?1 AND published IS NULL ORDER BY created DESC, rowid DESC LIMIT 100)", [deployment]).map_err(err)?;
        let usage = |tx: &rusqlite::Transaction<'_>| -> Result<(i64, i64, i64, i64), String> {
            tx.query_row("SELECT (SELECT count(*) FROM task_jobs WHERE deployment=?1),count(*),coalesce(sum(length(CAST(body AS BLOB))),0),(SELECT count(*) FROM task_jobs) FROM task_revisions WHERE deployment=?1", [deployment], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(err)
        };
        let mut counts = usage(&tx)?;
        if counts.1 > 2000 || counts.2 > 32 * 1024 * 1024 {
            // Under pressure retain the latest draft per job and all current revisions.
            tx.execute("DELETE FROM task_revisions WHERE deployment=?1 AND published IS NULL AND id NOT IN (SELECT current FROM task_jobs WHERE current IS NOT NULL) AND rowid NOT IN (SELECT max(rowid) FROM task_revisions WHERE deployment=?1 AND published IS NULL GROUP BY job)", [deployment]).map_err(err)?;
            counts = usage(&tx)?;
        }
        if counts.3 > 10000 {
            return Err("runtime task quota reached (10000 jobs)".into());
        }
        if counts.0 > 100 || counts.1 > 2000 || counts.2 > 32 * 1024 * 1024 {
            return Err(
                "task scope quota reached (100 jobs, 2000 revisions, 32 MiB source history)".into(),
            );
        }
        tx.commit().map_err(err)?;
        let _ = self.cleanup_artifacts(&db);
        Ok(json!({"revision_id":revision,"job_id":job,"digest":digest,"body":draft}))
    }
    fn publish(&self, deployment: &str, body: &Value) -> Result<Value, String> {
        let _artifacts = self.artifacts.lock().unwrap();
        let revision = string(body, "revision_id")?;
        if !body
            .as_object()
            .is_some_and(|o| o.contains_key("expected_current_revision"))
        {
            return Err("expected_current_revision is required (null for a new job)".into());
        }
        if !body["expected_current_revision"].is_null()
            && !body["expected_current_revision"].is_string()
        {
            return Err("expected_current_revision must be a revision ID or null".into());
        }
        let expected = body
            .get("expected_current_revision")
            .and_then(Value::as_str);
        let (job, encoded, published): (String, String, Option<i64>) = {
            let db = self.db.lock().unwrap();
            db.query_row(
                "SELECT job,body,published FROM task_revisions WHERE id=?1 AND deployment=?2",
                params![revision, deployment],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| "unknown revision".to_string())?
        };
        {
            let db = self.db.lock().unwrap();
            let current: Option<String> = db
                .query_row("SELECT current FROM task_jobs WHERE id=?1", [&job], |r| {
                    r.get(0)
                })
                .map_err(err)?;
            if current.as_deref() == Some(revision) && published.is_some() {
                return Ok(json!({"job_id":job,"revision_id":revision,"ok":true}));
            }
            if current.as_deref() != expected {
                return Err("publication conflict: current revision changed; review again".into());
            }
            if published.is_some() {
                return Err("restore creates a new draft; published history is immutable".into());
            }
        }
        let draft: Draft = serde_json::from_str(&encoded).map_err(err)?;
        self.allowed(deployment, &draft.network)?;
        // Compilation/dependency graph loading never executes task top-level code.
        self.prepare(revision, &draft)?;
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(err)?;
        self.allowed(deployment, &draft.network)?;
        let verified:Option<(String,Option<i64>)>=tx.query_row("SELECT body,published FROM task_revisions WHERE id=?1 AND deployment=?2 AND job=?3",params![revision,deployment,job],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
        let Some((verified_body, verified_published)) = verified else {
            return Err("draft expired during preparation; create a new draft".into());
        };
        if verified_body != encoded || verified_published != published {
            return Err("revision changed during preparation; review again".into());
        }
        let current: Option<String> = tx
            .query_row("SELECT current FROM task_jobs WHERE id=?1", [&job], |r| {
                r.get(0)
            })
            .map_err(err)?;
        if current.as_deref() == Some(revision) && published.is_some() {
            return Ok(json!({"job_id":job,"revision_id":revision,"ok":true}));
        }
        if current.as_deref() != expected {
            return Err("publication conflict: current revision changed; review again".into());
        }
        if published.is_some() {
            return Err("restore creates a new draft; published history is immutable".into());
        }
        let claimed: bool = tx
            .query_row(
                "SELECT once_claimed FROM task_jobs WHERE id=?1",
                [&job],
                |r| r.get(0),
            )
            .map_err(err)?;
        let preserve_once = if let (true, Some(current), Schedule::Once { at: new_at }) =
            (claimed, current.as_deref(), &draft.schedule)
        {
            let old: String = tx
                .query_row(
                    "SELECT body FROM task_revisions WHERE id=?1",
                    [current],
                    |r| r.get(0),
                )
                .map_err(err)?;
            let old: Draft = serde_json::from_str(&old).map_err(err)?;
            matches!(old.schedule,Schedule::Once{at} if at==*new_at)
        } else {
            false
        };
        let at = if preserve_once {
            None
        } else {
            match draft.schedule {
                Schedule::Once { at } => Some(at),
                _ => next(&draft.schedule, now())?,
            }
        };
        tx.execute(
            "UPDATE task_revisions SET published=?2 WHERE id=?1",
            params![revision, now()],
        )
        .map_err(err)?;
        tx.execute(
            "UPDATE task_jobs SET current=?2,name=?3,next_at=?4,once_claimed=?5 WHERE id=?1",
            params![job, revision, draft.name, at, preserve_once],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(json!({"job_id":job,"revision_id":revision,"ok":true}))
    }
    fn prepare(&self, revision: &str, draft: &Draft) -> Result<(), String> {
        self.prepare_interruptible(revision, draft, None)
    }
    fn prepare_interruptible(
        &self,
        revision: &str,
        draft: &Draft,
        cancel: Option<&AtomicBool>,
    ) -> Result<(), String> {
        let target = self.root.join(revision);
        if target.exists() {
            if fs::read_to_string(target.join("task.ts")).map_err(err)? != draft.source {
                return Err("prepared task source integrity check failed".into());
            }
            if target.join("task.js").is_file() {
                return Ok(());
            }
        }
        let staging = tempfile::Builder::new()
            .prefix(".prepare-")
            .tempdir_in(&self.root)
            .map_err(err)?;
        let dir = staging.path().canonicalize().map_err(err)?;
        let source = dir.join("task.ts");
        fs::write(&source, &draft.source).map_err(err)?;
        // Transform only: the compiler never resolves or reads task imports.
        let mut command = Command::new(crate::worker::compiler()?);
        command
            .arg(&source)
            .args([
                "--format=esm",
                "--platform=browser",
                "--target=es2022",
                "--log-level=error",
            ])
            .arg(format!("--outfile={}", dir.join("task.js").display()))
            .arg(format!("--metafile={}", dir.join("meta.json").display()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let output = bounded_command(command, Duration::from_secs(30), || {
            self.stop.load(Ordering::Relaxed) || cancel.is_some_and(|c| c.load(Ordering::Relaxed))
        })?;
        if !output.0 {
            return Err(format!("task validation failed: {}", output.1));
        }
        let meta: Value =
            serde_json::from_slice(&fs::read(dir.join("meta.json")).map_err(err)?).map_err(err)?;
        if meta["outputs"]
            .as_object()
            .ok_or("missing compiler metadata")?
            .values()
            .any(|v| v["imports"].as_array().is_some_and(|a| !a.is_empty()))
        {
            return Err(
                "Task dependencies are not supported; use a self-contained TypeScript module"
                    .into(),
            );
        }
        if target.exists() {
            fs::rename(dir.join("task.js"), target.join("task.js")).map_err(err)?;
            return Ok(());
        }
        if let Err(e) = fs::rename(dir, &target)
            && (!target.exists()
                || fs::read_to_string(target.join("task.ts")).map_err(err)? != draft.source)
        {
            return Err(err(e));
        }
        Ok(())
    }
    // Called with the artifact lock held after committing database changes.
    fn cleanup_artifacts(&self, db: &Connection) -> Result<(), String> {
        for entry in fs::read_dir(&self.root).map_err(err)? {
            let entry = entry.map_err(err)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.len() == 33
                && name.starts_with('v')
                && name[1..].bytes().all(|b| b.is_ascii_hexdigit())
            {
                let retained: bool = db
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM task_revisions WHERE id=?1)",
                        [&name],
                        |r| r.get(0),
                    )
                    .map_err(err)?;
                if !retained {
                    fs::remove_dir_all(entry.path()).map_err(err)?;
                }
            }
        }
        Ok(())
    }
    fn delete(&self, deployment: &str, job: &str) -> Result<Value, String> {
        let _artifacts = self.artifacts.lock().unwrap();
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(err)?;
        let archived: Option<bool> = tx
            .query_row(
                "SELECT archived FROM task_jobs WHERE id=?1 AND deployment=?2",
                params![job, deployment],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        match archived {
            None => return Err("unknown job".into()),
            Some(false) => return Err("archive job before deleting it".into()),
            Some(true) => {}
        }
        let busy: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM task_runs WHERE job=?1 AND status IN ('pending','running'))", [job], |r| r.get(0)).map_err(err)?;
        if busy {
            return Err("job has an active run".into());
        }
        tx.execute(
            "DELETE FROM task_notifications WHERE run IN (SELECT id FROM task_runs WHERE job=?1)",
            [job],
        )
        .map_err(err)?;
        tx.execute("DELETE FROM task_watermarks WHERE job=?1", [job])
            .map_err(err)?;
        tx.execute("DELETE FROM task_runs WHERE job=?1", [job])
            .map_err(err)?;
        tx.execute(
            "DELETE FROM task_revisions WHERE job=?1 AND deployment=?2",
            params![job, deployment],
        )
        .map_err(err)?;
        tx.execute(
            "DELETE FROM task_jobs WHERE id=?1 AND deployment=?2",
            params![job, deployment],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        let artifacts_cleaned = self.cleanup_artifacts(&db).is_ok();
        Ok(json!({"ok":true,"artifacts_cleaned":artifacts_cleaned}))
    }
    fn archive(&self, deployment: &str, job: &str, archive: bool) -> Result<Value, String> {
        self.owned(deployment, job)?;
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(err)?;
        let (revision, was_archived, claimed): (Option<String>, bool, bool) = tx
            .query_row(
                "SELECT current,archived,once_claimed FROM task_jobs WHERE id=?1",
                [job],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(err)?;
        let mut next_at = None;
        if !archive
            && was_archived
            && let Some(revision) = revision
        {
            let source: String = tx
                .query_row(
                    "SELECT body FROM task_revisions WHERE id=?1",
                    [revision],
                    |r| r.get(0),
                )
                .map_err(err)?;
            let draft: Draft = serde_json::from_str(&source).map_err(err)?;
            next_at = match draft.schedule {
                Schedule::Once { at } if !claimed => Some(at.max(now())),
                Schedule::Once { .. } => None,
                _ => next(&draft.schedule, now())?,
            };
        }
        tx.execute("UPDATE task_jobs SET archived=?2,next_at=CASE WHEN ?2=1 THEN NULL WHEN archived=1 THEN ?3 ELSE next_at END WHERE id=?1",params![job,archive,next_at]).map_err(err)?;
        if archive {
            tx.execute("UPDATE task_runs SET status='cancelled',ended=?2,diagnostic='Archived before dispatch' WHERE job=?1 AND status='pending'",params![job,now()]).map_err(err)?;
        }
        tx.commit().map_err(err)?;
        Ok(json!({"ok":true,"overdue_once_runs_now":next_at.is_some_and(|t|t<=now())}))
    }
    fn admit(&self, job: &str, occurrence: &str, at: i64, manual: bool) -> Result<String, String> {
        if self.stop.load(Ordering::Relaxed) {
            return Err("task engine is shutting down".into());
        }
        let mut db = self.db.lock().unwrap();
        if self.stop.load(Ordering::Relaxed) {
            return Err("task engine is shutting down".into());
        }
        let tx = db.transaction().map_err(err)?;
        let run = self.admit_tx(&tx, job, occurrence, at, manual)?;
        prune(&tx, job)?;
        tx.commit().map_err(err)?;
        Ok(run)
    }
    fn admit_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        job: &str,
        occurrence: &str,
        at: i64,
        manual: bool,
    ) -> Result<String, String> {
        let (deployment, revision, archived): (String, Option<String>, bool) = tx
            .query_row(
                "SELECT deployment,current,archived FROM task_jobs WHERE id=?1",
                [job],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(err)?;
        if archived {
            return Err("job is archived".into());
        }
        let revision = revision.ok_or("job has no published revision")?;
        let encoded: String = tx
            .query_row(
                "SELECT body FROM task_revisions WHERE id=?1",
                [&revision],
                |r| r.get(0),
            )
            .map_err(err)?;
        let draft: Draft = serde_json::from_str(&encoded).map_err(err)?;
        self.allowed(&deployment, &draft.network)?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM task_runs WHERE occurrence=?1",
                [occurrence],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        if let Some(existing) = existing {
            return Ok(existing);
        }
        let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM task_runs WHERE job=?1 AND status IN('pending','running'))",[job],|r|r.get(0)).map_err(err)?;
        let count: i64 = tx
            .query_row(
                "SELECT count(*) FROM task_runs WHERE status IN('pending','running')",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        if manual && (busy || count >= MAX_CONCURRENCY as i64) {
            return Err(if busy {
                "job is already running"
            } else {
                "global concurrency limit reached"
            }
            .into());
        }
        let run = id("r")?;
        let skip = busy || count >= MAX_CONCURRENCY as i64;
        tx.execute("INSERT INTO task_runs(id,job,deployment,revision,occurrence,status,started,ended,diagnostic) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![run,job,deployment,revision,occurrence,if skip{"skipped"}else{"pending"},at,if skip{Some(at)}else{None},if skip{Some("No overlap or global concurrency limit")}else{None}]).map_err(err)?;
        Ok(run)
    }
    /// Deterministic wall-clock input; claim and advance share one transaction.
    pub fn tick(self: &Arc<Self>, at: i64) -> Result<(), String> {
        self.evaluate(at)?;
        self.dispatch_pending()
    }
    fn evaluate(&self, at: i64) -> Result<(), String> {
        if self.stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(err)?;
        let jobs: Vec<(String, String, String, String, i64)> = {
            let mut q=tx.prepare("SELECT j.id,j.deployment,j.current,r.body,j.next_at FROM task_jobs j JOIN task_revisions r ON r.id=j.current WHERE archived=0 AND next_at<=?1 ORDER BY next_at").map_err(err)?;
            q.query_map([at], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?
        };
        for (job, deployment, revision, body, due) in jobs {
            let draft: Draft = serde_json::from_str(&body).map_err(err)?;
            if self.allowed(&deployment, &draft.network).is_err() {
                continue;
            }
            let occurrence = match &draft.schedule {
                Schedule::Cron { timezone, .. } => {
                    let zone: chrono_tz::Tz = timezone.parse().map_err(err)?;
                    let local = Utc
                        .timestamp_millis_opt(due)
                        .single()
                        .ok_or("invalid time")?
                        .with_timezone(&zone);
                    format!("{job}:{revision}:{}", local.format("%Y-%m-%dT%H:%M"))
                }
                _ => format!("{job}:{revision}:once"),
            };
            if matches!(draft.schedule, Schedule::Cron { .. }) {
                let watermark: Option<String> = tx
                    .query_row(
                        "SELECT occurrence FROM task_watermarks WHERE job=?1 AND revision=?2",
                        params![job, revision],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(err)?;
                if watermark.as_ref().is_some_and(|w| occurrence <= *w) {
                    tx.execute(
                        "UPDATE task_jobs SET next_at=?2 WHERE id=?1",
                        params![job, next(&draft.schedule, at)?],
                    )
                    .map_err(err)?;
                    continue;
                }
                tx.execute("INSERT INTO task_watermarks VALUES(?1,?2,?3) ON CONFLICT(job,revision) DO UPDATE SET occurrence=excluded.occurrence",params![job,revision,occurrence]).map_err(err)?;
            }
            let missed =
                matches!(draft.schedule, Schedule::Cron { .. }) && at.saturating_sub(due) >= 60000;
            if missed {
                tx.execute("INSERT OR IGNORE INTO task_runs(id,job,deployment,revision,occurrence,status,started,ended,diagnostic) VALUES(?1,?2,?3,?4,?5,'skipped',?6,?6,'Missed recurring occurrences during downtime or clock change; no catch-up')",params![id("r")?,job,deployment,revision,occurrence,at]).map_err(err)?;
            } else {
                self.admit_tx(&tx, &job, &occurrence, at, false)?;
            }
            let future = match draft.schedule {
                Schedule::Once { .. } => None,
                _ => next(&draft.schedule, at)?,
            };
            tx.execute("UPDATE task_jobs SET next_at=?3,once_claimed=CASE WHEN ?4 THEN 1 ELSE once_claimed END WHERE id=?1 AND current=?2 AND archived=0",params![job,revision,future,matches!(draft.schedule,Schedule::Once{..})]).map_err(err)?;
            prune(&tx, &job)?;
        }
        tx.execute("UPDATE task_clock SET last_tick=?1 WHERE id=1", [at])
            .map_err(err)?;
        tx.commit().map_err(err)
    }
    fn dispatch_pending(self: &Arc<Self>) -> Result<(), String> {
        let mut running = self.running.lock().unwrap();
        if self.stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(err)?;
        let pending: Vec<(String, String, String, String, String)> = {
            let mut q=tx.prepare("SELECT x.id,x.job,x.deployment,x.revision,r.body FROM task_runs x JOIN task_revisions r ON r.id=x.revision JOIN task_jobs j ON j.id=x.job WHERE x.status='pending' AND j.archived=0 LIMIT 4").map_err(err)?;
            q.query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?
        };
        let mut launch = vec![];
        for (run, job, deployment, revision, body) in pending {
            if running.len() >= MAX_CONCURRENCY {
                break;
            }
            tx.execute("UPDATE task_runs SET status='running' WHERE id=?1", [&run])
                .map_err(err)?;
            let cancel = Arc::new(AtomicBool::new(false));
            running.insert(
                run.clone(),
                Running {
                    cancel: cancel.clone(),
                    output: Arc::new(Mutex::new((Vec::new(), false))),
                    events: Arc::new(Mutex::new(Vec::new())),
                },
            );
            launch.push((run, job, deployment, revision, body, cancel));
        }
        tx.commit().map_err(err)?;
        drop(db);
        drop(running);
        for (run, job, deployment, revision, body, cancel) in launch {
            let this = self.clone();
            thread::spawn(move || {
                let draft: Result<Draft, _> = serde_json::from_str(&body);
                let result = match draft {
                    Ok(draft) => {
                        let result =
                            this.execute(&run, &job, &deployment, &revision, &draft, &cancel);
                        this.finish(&run, &job, &deployment, &draft, result)
                    }
                    Err(e) => Err(err(e)),
                };
                if let Err(e) = result {
                    eprintln!("task completion persistence: {e}");
                }
                this.running.lock().unwrap().remove(&run);
            });
        }
        Ok(())
    }
    fn execute(
        &self,
        run: &str,
        job: &str,
        deployment: &str,
        revision: &str,
        draft: &Draft,
        cancel: &AtomicBool,
    ) -> (String, String, bool, String) {
        let result = (|| -> Result<(String, String, bool, String), String> {
            self.allowed(deployment, &draft.network)?;
            {
                let db = self.db.lock().unwrap();
                let (body, digest): (String, String) = db
                    .query_row(
                        "SELECT body,digest FROM task_revisions WHERE id=?1",
                        [revision],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .map_err(err)?;
                if format!("{:x}", Sha256::digest(body.as_bytes())) != digest {
                    return Err("revision integrity check failed".into());
                }
            }
            if self.stop.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed) {
                return Ok((
                    "cancelled".into(),
                    String::new(),
                    false,
                    "Cancelled before spawn".into(),
                ));
            }
            self.prepare_interruptible(revision, draft, Some(cancel))?;
            let dir = self.root.join(revision);
            let metadata = json!({"run_id":run,"job_id":job,"deployment_id":deployment,"revision_id":revision});
            let mut cmd = Command::new(std::env::current_exe().map_err(err)?);
            cmd.arg("__paraco_task_guardian");
            crate::guardian::inherit_lease(&mut cmd, self.lease.lock().unwrap().as_ref())?;
            cmd.arg(&self.workerd)
                .arg(dir.join("task.js"))
                .arg(metadata.to_string())
                .arg(serde_json::to_string(&draft.network).map_err(err)?)
                .env_clear()
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if std::env::var("PARACO_ALLOW_UNSANDBOXED").as_deref() == Ok("1") {
                cmd.env("PARACO_ALLOW_UNSANDBOXED", "1");
            }
            let spawning = self.running.lock().unwrap();
            self.allowed(deployment, &draft.network)?;
            if cancel.load(Ordering::Relaxed) || self.stop.load(Ordering::Relaxed) {
                return Ok((
                    "cancelled".into(),
                    String::new(),
                    false,
                    "Cancelled during preparation".into(),
                ));
            }
            let mut child = cmd.spawn().map_err(err)?;
            drop(spawning);
            let lease = child.stdin.take();
            let (output, events) = {
                let running = self.running.lock().unwrap();
                let state = running.get(run).ok_or("missing run owner")?;
                (state.output.clone(), state.events.clone())
            };
            let _notifications = NotificationPump::start(
                self.db.clone(),
                events.clone(),
                output.clone(),
                deployment.to_owned(),
                run.to_owned(),
            );
            let mut drains = vec![];
            if let Some(pipe) = child.stdout.take() {
                drains.push(drain_events(pipe, output.clone(), Some(events.clone())));
            }
            if let Some(pipe) = child.stderr.take() {
                drains.push(drain_events(pipe, output.clone(), Some(events.clone())));
            }
            let deadline = Instant::now() + Duration::from_millis(draft.timeout_ms);
            let mut outcome = None;
            let status = loop {
                if let Some(status) = child.try_wait().map_err(err)? {
                    break status;
                }
                if cancel.load(Ordering::Relaxed) || self.stop.load(Ordering::Relaxed) {
                    outcome = Some("cancelled");
                    break stop_guardian(&mut child, lease);
                }
                if Instant::now() >= deadline {
                    outcome = Some("timeout");
                    break stop_guardian(&mut child, lease);
                }
                thread::sleep(Duration::from_millis(10));
            };
            for handle in drains {
                let _ = handle.join();
            }
            let out = output.lock().unwrap();
            let logs = String::from_utf8_lossy(&out.0).into_owned();
            let status = outcome.unwrap_or(if status.success() { "success" } else { "error" });
            Ok((
                status.into(),
                logs,
                out.1,
                if status == "error" {
                    "Task process exited unsuccessfully".into()
                } else {
                    String::new()
                },
            ))
        })();
        result.unwrap_or_else(|e| {
            (
                if cancel.load(Ordering::Relaxed) || self.stop.load(Ordering::Relaxed) {
                    "cancelled"
                } else {
                    "error"
                }
                .into(),
                String::new(),
                false,
                e,
            )
        })
    }
    fn finish(
        &self,
        run: &str,
        job: &str,
        deployment: &str,
        draft: &Draft,
        result: (String, String, bool, String),
    ) -> Result<(), String> {
        let (status, logs, truncated, diagnostic) = result;
        let events = self
            .running
            .lock()
            .unwrap()
            .get(run)
            .map(|r| r.events.lock().unwrap().clone())
            .unwrap_or_default();
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().map_err(err)?;
        tx.execute("UPDATE task_runs SET status=?2,logs=?3,truncated=?4,diagnostic=?5,ended=?6 WHERE id=?1",params![run,status,logs,truncated,diagnostic,now()]).map_err(err)?;
        prune(&tx, job)?;
        tx.commit().map_err(err)?;
        // Delivery is independent: inbox failures cannot roll back task outcomes.
        let notify = |event: String, message: String| {
            if let Err(e)=db.execute("INSERT OR IGNORE INTO task_notifications(id,deployment,run,message,created) VALUES(?1,?2,?3,?4,?5)",params![event,deployment,run,message,now()]){eprintln!("task notification delivery failed: {e}");}
        };
        if (status == "success" && draft.notify_success)
            || (status != "success" && draft.notify_failure)
        {
            notify(
                format!("{run}:outcome"),
                format!("{}: {status}", draft.name),
            );
        }
        for (index, message) in events.into_iter().enumerate() {
            notify(format!("{run}:script:{index}"), message);
        }
        let _=db.execute("DELETE FROM task_notifications WHERE deployment=?1 AND id NOT IN(SELECT id FROM task_notifications WHERE deployment=?1 ORDER BY created DESC LIMIT 500)",[deployment]);
        Ok(())
    }
}
struct NotificationPump {
    done: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl NotificationPump {
    fn start(
        db: Arc<Mutex<Connection>>,
        events: Arc<Mutex<Vec<String>>>,
        output: Arc<Mutex<(Vec<u8>, bool)>>,
        deployment: String,
        run: String,
    ) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let signal = done.clone();
        let path = db.lock().unwrap().path().map(PathBuf::from);
        let handle = thread::spawn(move || {
            // Separate connection keeps disk delivery off admission/cancellation
            // locks. Bounded snapshots never hold the pipe buffer during I/O.
            let Some(path) = path else { return };
            let Ok(db) = Connection::open(path) else {
                return;
            };
            let _ = db.busy_timeout(Duration::from_millis(50));
            let mut seen = 0;
            let mut delivery_failed = false;
            let mut last_flush = Instant::now();
            let mut persisted = (0, false);
            loop {
                let complete = signal.load(Ordering::Relaxed);
                let batch = {
                    let events = events.lock().unwrap();
                    events[seen..].to_vec()
                };
                if !batch.is_empty() && !delivery_failed {
                    for message in batch {
                        let event = format!("{run}:script:{seen}");
                        seen += 1;
                        if let Err(e)=db.execute("INSERT OR IGNORE INTO task_notifications(id,deployment,run,message,created) VALUES(?1,?2,?3,?4,?5)",params![event,deployment,run,message,now()]){eprintln!("task notification delivery failed: {e}");delivery_failed=true;break;}
                    }
                    let _=db.execute("DELETE FROM task_notifications WHERE deployment=?1 AND id NOT IN(SELECT id FROM task_notifications WHERE deployment=?1 ORDER BY created DESC LIMIT 500)",[&deployment]);
                }
                if complete || last_flush.elapsed() >= Duration::from_millis(250) {
                    let snapshot = {
                        let output = output.lock().unwrap();
                        let changed = (output.0.len(), output.1) != persisted;
                        if changed {
                            Some((output.0.clone(), output.1))
                        } else {
                            None
                        }
                    };
                    if let Some((bytes, truncated)) = snapshot {
                        let logs = String::from_utf8_lossy(&bytes);
                        match db.execute("UPDATE task_runs SET logs=?2,truncated=?3 WHERE id=?1 AND status='running'",params![run,logs,truncated]){Ok(_)=>persisted=(bytes.len(),truncated),Err(e)=>eprintln!("task log snapshot failed: {e}")}
                    }
                    last_flush = Instant::now();
                }
                if complete {
                    break;
                }
                thread::sleep(Duration::from_millis(25));
            }
        });
        Self {
            done,
            handle: Some(handle),
        }
    }
}
impl Drop for NotificationPump {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
fn prune(tx: &rusqlite::Transaction<'_>, job: &str) -> Result<(), String> {
    tx.execute("UPDATE task_runs SET logs='',truncated=1,diagnostic=coalesce(diagnostic,'') || ' [Log content expired]' WHERE job=?1 AND id NOT IN(SELECT id FROM task_runs WHERE job=?1 ORDER BY started DESC LIMIT 100) AND logs<>''",[job]).map_err(err)?;
    tx.execute("DELETE FROM task_notifications WHERE run IN (SELECT id FROM task_runs WHERE job=?1 AND status NOT IN ('pending','running') AND id NOT IN (SELECT id FROM task_runs WHERE job=?1 ORDER BY started DESC LIMIT 1000))", [job]).map_err(err)?;
    tx.execute("DELETE FROM task_runs WHERE job=?1 AND status NOT IN('pending','running') AND id NOT IN(SELECT id FROM task_runs WHERE job=?1 ORDER BY started DESC LIMIT 1000)",[job]).map_err(err)?;
    Ok(())
}
fn string<'a>(body: &'a Value, key: &str) -> Result<&'a str, String> {
    body.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} required"))
}
fn drain(
    pipe: impl Read + Send + 'static,
    output: Arc<Mutex<(Vec<u8>, bool)>>,
) -> thread::JoinHandle<()> {
    drain_events(pipe, output, None)
}
fn drain_events(
    pipe: impl Read + Send + 'static,
    output: Arc<Mutex<(Vec<u8>, bool)>>,
    events: Option<Arc<Mutex<Vec<String>>>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut pipe = pipe;
        let mut buf = [0; 4096];
        let mut line = Vec::new();
        let mut oversized = false;
        loop {
            match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if let Some(events) = &events {
                        for byte in &buf[..n] {
                            if *byte == b'\n' {
                                if !oversized
                                    && let Some(message) = std::str::from_utf8(&line)
                                        .ok()
                                        .and_then(|s| {
                                            s.strip_prefix("PARACO_NOTIFICATION:").or_else(|| {
                                                s.strip_prefix("console.log: PARACO_NOTIFICATION:")
                                            })
                                        })
                                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                                        .and_then(|v| v["message"].as_str().map(str::to_owned))
                                {
                                    let mut events = events.lock().unwrap();
                                    if events.len() < 100 {
                                        events.push(message.chars().take(2048).collect());
                                    }
                                }
                                line.clear();
                                oversized = false;
                            } else if line.len() < 16384 {
                                line.push(*byte);
                            } else {
                                oversized = true;
                            }
                        }
                    }
                    let mut out = output.lock().unwrap();
                    let take = n.min(MAX_LOG.saturating_sub(out.0.len()));
                    out.0.extend_from_slice(&buf[..take]);
                    out.1 |= take < n;
                }
            }
        }
    })
}
fn stop_guardian(
    child: &mut std::process::Child,
    lease: Option<std::process::ChildStdin>,
) -> std::process::ExitStatus {
    drop(lease);
    child.wait().unwrap_or_else(|_| {
        let _ = child.kill();
        child.wait().expect("child reaped")
    })
}
fn bounded_command(
    mut cmd: Command,
    timeout: Duration,
    abort: impl Fn() -> bool,
) -> Result<(bool, String), String> {
    let mut child = cmd.spawn().map_err(err)?;
    let output = Arc::new(Mutex::new((Vec::new(), false)));
    let handle = child.stderr.take().map(|p| drain(p, output.clone()));
    let stdout = child.stdout.take().map(|p| drain(p, output.clone()));
    let deadline = Instant::now() + timeout;
    let success = loop {
        if let Some(status) = child.try_wait().map_err(err)? {
            break status.success();
        }
        if Instant::now() >= deadline || abort() {
            let _ = child.kill();
            let _ = child.wait();
            break false;
        }
        thread::sleep(Duration::from_millis(10));
    };
    if let Some(h) = handle {
        let _ = h.join();
    }
    if let Some(h) = stdout {
        let _ = h.join();
    }
    let text = String::from_utf8_lossy(&output.lock().unwrap().0).into_owned();
    Ok((success, text))
}

/// Restricted five-field numeric cron: *, comma lists, inclusive ranges, /steps.
/// DOM and DOW are OR when both restricted; Sunday is 0 or 7. No seconds/names.
#[derive(Debug)]
struct Cron {
    fields: Vec<Vec<u32>>,
    dom_any: bool,
    dow_any: bool,
}
impl Cron {
    fn parse(s: &str) -> Result<Self, String> {
        if s.len() > 256 {
            return Err("cron expression exceeds 256 bytes".into());
        }
        let fields: Vec<_> = s.split_whitespace().collect();
        if fields.len() != 5 {
            return Err("cron requires five fields: minute hour day month weekday".into());
        }
        let ranges = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 7)];
        let mut parsed = vec![];
        for (field, (lo, hi)) in fields.iter().zip(ranges) {
            let mut values = vec![];
            for part in field.split(',') {
                let mut step = part.split('/');
                let base = step.next().unwrap();
                let n = step
                    .next()
                    .map(|n| n.parse::<u32>().map_err(err))
                    .transpose()?
                    .unwrap_or(1);
                if n == 0 || step.next().is_some() {
                    return Err("invalid cron step".into());
                }
                let (a, b) = if base == "*" {
                    (lo, hi)
                } else if let Some((a, b)) = base.split_once('-') {
                    (a.parse().map_err(err)?, b.parse().map_err(err)?)
                } else {
                    let v = base.parse().map_err(err)?;
                    (v, v)
                };
                if a < lo || b > hi || a > b {
                    return Err("cron value outside field range".into());
                }
                values.extend((a..=b).step_by(n as usize));
            }
            values.sort_unstable();
            values.dedup();
            parsed.push(values);
        }
        Ok(Self {
            fields: parsed,
            dom_any: fields[2] == "*",
            dow_any: fields[4] == "*",
        })
    }
    fn day_matches(&self, t: chrono::NaiveDate) -> bool {
        let f = &self.fields;
        let dom = f[2].contains(&t.day());
        let weekday = t.weekday().num_days_from_sunday();
        let dow = f[4].contains(&weekday) || (weekday == 0 && f[4].contains(&7));
        f[3].contains(&t.month())
            && (if self.dom_any {
                dow
            } else if self.dow_any {
                dom
            } else {
                dom || dow
            })
    }
    #[cfg(test)]
    fn matches(&self, t: &chrono::DateTime<chrono_tz::Tz>) -> bool {
        use chrono::Timelike;
        self.fields[0].contains(&t.minute())
            && self.fields[1].contains(&t.hour())
            && self.day_matches(t.date_naive())
    }
}
pub fn next(schedule: &Schedule, after: i64) -> Result<Option<i64>, String> {
    if !(0..=253402128000000).contains(&after) {
        return Err("timestamp must be between 1970 and 9999".into());
    }
    if let Schedule::Once { at } = schedule
        && !(0..=253402128000000).contains(at)
    {
        return Err("one-off timestamp must be between 1970 and 9999".into());
    }
    match schedule {
        Schedule::Manual => Ok(None),
        Schedule::Once { at } => Ok(if *at > after { Some(*at) } else { None }),
        Schedule::Cron {
            expression,
            timezone,
        } => {
            let cron = Cron::parse(expression)?;
            let zone: chrono_tz::Tz = timezone
                .parse()
                .map_err(|_| "timezone must be an IANA timezone".to_string())?;
            let mut date = Utc
                .timestamp_millis_opt(after)
                .single()
                .ok_or("invalid timestamp")?
                .with_timezone(&zone)
                .date_naive();
            for _ in 0..(366 * 5) {
                if cron.day_matches(date) {
                    for hour in &cron.fields[1] {
                        for minute in &cron.fields[0] {
                            let wall = date
                                .and_hms_opt(*hour, *minute, 0)
                                .ok_or("invalid cron time")?;
                            let local = match zone.from_local_datetime(&wall) {
                                chrono::LocalResult::Single(t)
                                | chrono::LocalResult::Ambiguous(t, _) => Some(t),
                                chrono::LocalResult::None => None,
                            };
                            if let Some(local) = local {
                                let timestamp = local.timestamp_millis();
                                if timestamp > 253402128000000 {
                                    return Err("cron exceeds supported year 9999".into());
                                }
                                if timestamp > after {
                                    return Ok(Some(timestamp));
                                }
                            }
                        }
                    }
                }
                date = date
                    .checked_add_days(chrono::Days::new(1))
                    .ok_or("cron exceeds supported calendar")?;
            }
            Err("cron has no occurrence within five years".into())
        }
    }
}

/// Private child guardian; EOF from host means kill and reap workerd before dropping
/// the state ownership lease, on macOS as well as Linux.
pub fn guardian() -> i32 {
    fn inner() -> Result<i32, String> {
        let mut args = std::env::args_os().skip(2);
        let lease = args.next().ok_or("missing lease")?;
        #[cfg(unix)]
        let _lease = if lease != "-" {
            use std::os::fd::FromRawFd;
            let fd: i32 = lease
                .to_str()
                .ok_or("invalid lease")?
                .parse()
                .map_err(err)?;
            if fd < 3 {
                return Err("invalid lease".into());
            }
            unsafe {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            Some(unsafe { fs::File::from_raw_fd(fd) })
        } else {
            None
        };
        let workerd = args.next().ok_or("missing workerd")?;
        let entry = args.next().ok_or("missing entry")?;
        let metadata = args.next().ok_or("missing metadata")?;
        let network: Vec<String> =
            serde_json::from_str(&args.next().ok_or("missing network")?.to_string_lossy())
                .map_err(err)?;
        let gone = Arc::new(AtomicBool::new(false));
        let flag = gone.clone();
        thread::spawn(move || {
            let _ = std::io::stdin().read(&mut [0]);
            flag.store(true, Ordering::Relaxed);
        });
        let temp = tempfile::tempdir().map_err(err)?;
        let dir = temp.path();
        fs::copy(entry, dir.join("app.js")).map_err(err)?;
        fs::write(
            dir.join("adapter.js"),
            include_str!("../runtime/task_host.js"),
        )
        .map_err(err)?;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(err)?;
        let port = listener.local_addr().map_err(err)?.port();
        let metadata: Value = serde_json::from_str(&metadata.to_string_lossy()).map_err(err)?;
        let config = crate::worker::config(dir, port, None, &metadata)?;
        let bridge = crate::bridge::Bridge::start(json!({"network":network}), dir)?;
        crate::worker::task_network(&config, dir, &bridge.address, &bridge.token)?;
        let mut cmd = crate::worker::launch_command(Path::new(&workerd), dir, &config, &listener)?;
        let mut child = cmd.spawn().map_err(err)?;
        drop(listener);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        thread::spawn(move || {
            let result = (|| -> Result<bool, String> {
                let deadline = Instant::now() + Duration::from_secs(8);
                loop {
                    if let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", port)) {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(3600)))
                            .map_err(err)?;
                        stream.write_all(b"POST /run HTTP/1.1\r\nHost: task\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").map_err(err)?;
                        let mut bytes = [0; 128];
                        let n = stream.read(&mut bytes).map_err(err)?;
                        return Ok(String::from_utf8_lossy(&bytes[..n]).contains("200 OK"));
                    }
                    if Instant::now() > deadline {
                        return Err("task startup timed out".into());
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            })();
            let _ = tx.send(result);
        });
        loop {
            if let Some(status) = child.try_wait().map_err(err)? {
                return Ok(status.code().unwrap_or(1));
            }
            if let Ok(result) = rx.try_recv() {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(if result.unwrap_or(false) { 0 } else { 1 });
            }
            if gone.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(1);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
    match inner() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("task guardian: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Arc<Tasks>) {
        let dir = tempfile::tempdir().unwrap();
        let workerd = std::env::var_os("WORKERD_BIN")
            .or_else(|| std::env::var_os("PARACO_WORKERD"))
            .map(PathBuf::from)
            .map(|path| path.canonicalize().unwrap_or(path))
            .or_else(|| crate::runtime::command_for_host().ok())
            .unwrap_or_else(|| PathBuf::from("/nonexistent/paraco-test-workerd"));
        let tasks = Tasks::open(dir.path(), workerd).unwrap();
        tasks.set_grant("owner".into(), vec![]);
        (dir, tasks)
    }
    fn installed(tasks: &Tasks, schedule: Schedule, at: i64) -> (String, String) {
        let draft = Draft {
            job_id: None,
            name: "test".into(),
            source: "export default async function() {}".into(),
            schedule,
            timeout_ms: 1000,
            network: vec![],
            notify_success: false,
            notify_failure: true,
        };
        let proposal = tasks.draft("owner", draft).unwrap();
        let job = proposal["job_id"].as_str().unwrap().to_owned();
        let revision = proposal["revision_id"].as_str().unwrap().to_owned();
        let db = tasks.db.lock().unwrap();
        db.execute(
            "UPDATE task_jobs SET current=?2,next_at=?3 WHERE id=?1",
            params![job, revision, at],
        )
        .unwrap();
        db.execute(
            "UPDATE task_revisions SET published=1 WHERE id=?1",
            [&revision],
        )
        .unwrap();
        (job, revision)
    }
    #[test]
    fn draft_pruning_preserves_latest_draft_for_each_job() {
        let (_dir, tasks) = fixture();
        let (first, _) = installed(&tasks, Schedule::Manual, 0);
        let make = |job_id| Draft {
            job_id,
            name: "draft".into(),
            source: "export default {}".into(),
            schedule: Schedule::Manual,
            timeout_ms: 1000,
            network: vec![],
            notify_success: false,
            notify_failure: true,
        };
        let old = tasks.draft("owner", make(Some(first.clone()))).unwrap();
        let latest = tasks.draft("owner", make(Some(first.clone()))).unwrap();
        let other = tasks.draft("owner", make(None)).unwrap()["job_id"]
            .as_str()
            .unwrap()
            .to_owned();
        for _ in 0..110 {
            tasks.draft("owner", make(Some(other.clone()))).unwrap();
        }
        let db = tasks.db.lock().unwrap();
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM task_revisions WHERE id=?1)",
                [latest["revision_id"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(exists);
        let old_exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM task_revisions WHERE id=?1)",
                [old["revision_id"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!old_exists);
    }
    #[test]
    fn deleting_archived_jobs_recovers_quota_and_removes_owned_data() {
        let (_dir, tasks) = fixture();
        let (job, revision) = installed(&tasks, Schedule::Manual, 0);
        let artifact = tasks.root.join(&revision);
        fs::create_dir(&artifact).unwrap();
        assert!(
            tasks
                .command("owner", "delete", json!({"job_id":job}))
                .is_err()
        );
        tasks.archive("owner", &job, true).unwrap();
        {
            let db = tasks.db.lock().unwrap();
            db.execute("INSERT INTO task_runs(id,job,deployment,revision,occurrence,status,started) VALUES('busy',?1,'owner',?2,'busy','running',1)", params![job, revision]).unwrap();
        }
        assert!(
            tasks
                .command("owner", "delete", json!({"job_id":job}))
                .is_err()
        );
        {
            let db = tasks.db.lock().unwrap();
            db.execute("UPDATE task_runs SET status='finished' WHERE id='busy'", [])
                .unwrap();
            db.execute(
                "INSERT INTO task_notifications VALUES('n','owner','busy','message',1,0,'local')",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO task_watermarks VALUES(?1,?2,'x')",
                params![job, revision],
            )
            .unwrap();
            for i in 0..99 {
                db.execute("INSERT INTO task_jobs(id,deployment,name,archived) VALUES(?1,'owner','other',1)", [format!("other{i}")]).unwrap();
            }
        }
        tasks.set_grant("other".into(), vec![]);
        assert!(
            tasks
                .command("other", "delete", json!({"job_id":job}))
                .is_err()
        );
        tasks
            .command("owner", "delete", json!({"job_id":job}))
            .unwrap();
        assert!(!artifact.exists());
        let db = tasks.db.lock().unwrap();
        for table in [
            "task_jobs",
            "task_revisions",
            "task_runs",
            "task_notifications",
            "task_watermarks",
        ] {
            let count: i64 = db
                .query_row(
                    &format!(
                        "SELECT count(*) FROM {table} WHERE {}",
                        match table {
                            "task_jobs" => "id=?1",
                            "task_notifications" => "run='busy' AND ?1 IS NOT NULL",
                            _ => "job=?1",
                        }
                    ),
                    [&job],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "{table}");
        }
        drop(db);
        let draft = Draft {
            job_id: None,
            name: "new".into(),
            source: "export default {}".into(),
            schedule: Schedule::Manual,
            timeout_ms: 1000,
            network: vec![],
            notify_success: false,
            notify_failure: true,
        };
        tasks.draft("owner", draft).unwrap();
    }
    #[test]
    fn quota_pressure_reclaims_surplus_drafts_transactionally() {
        let (_dir, tasks) = fixture();
        let (job, _) = installed(&tasks, Schedule::Manual, 0);
        let draft = Draft {
            job_id: Some(job.clone()),
            name: "draft".into(),
            source: "export default {}".into(),
            schedule: Schedule::Manual,
            timeout_ms: 1000,
            network: vec![],
            notify_success: false,
            notify_failure: true,
        };
        for _ in 0..99 {
            tasks.draft("owner", draft.clone()).unwrap();
        }
        {
            let db = tasks.db.lock().unwrap();
            for i in 0..1900 {
                db.execute(
                    "INSERT INTO task_revisions VALUES(?1,?2,'owner','{}','digest',1,1)",
                    params![format!("published{i}"), job],
                )
                .unwrap();
            }
        }
        let saved = tasks.draft("owner", draft).unwrap();
        let db = tasks.db.lock().unwrap();
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM task_revisions WHERE deployment='owner'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(count < 2000);
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM task_revisions WHERE id=?1)",
                [saved["revision_id"].as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        assert!(exists);
    }

    #[test]
    fn pruning_run_history_removes_its_notifications() {
        let (_dir, tasks) = fixture();
        let (job, revision) = installed(&tasks, Schedule::Manual, 0);
        let mut db = tasks.db.lock().unwrap();
        let tx = db.transaction().unwrap();
        for i in 0..1001 {
            let run = format!("run{i}");
            tx.execute("INSERT INTO task_runs(id,job,deployment,revision,occurrence,status,started) VALUES(?1,?2,'owner',?3,?1,'success',?4)", params![run, job, revision, i]).unwrap();
            tx.execute(
                "INSERT INTO task_notifications VALUES(?1,'owner',?1,'message',1,0,'local')",
                [&run],
            )
            .unwrap();
        }
        prune(&tx, &job).unwrap();
        let count: i64 = tx
            .query_row(
                "SELECT count(*) FROM task_notifications WHERE run='run0'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        tx.commit().unwrap();
    }

    fn ts(s: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(s)
            .unwrap()
            .timestamp_millis()
    }
    #[test]
    fn cron_syntax_and_calendar_or() {
        let cron = Cron::parse("*/15 0-23 13 * 5").unwrap();
        let zone: chrono_tz::Tz = "UTC".parse().unwrap();
        assert!(cron.matches(&zone.with_ymd_and_hms(2026, 2, 13, 3, 15, 0).unwrap()));
        assert!(Cron::parse("* * * * * *").is_err());
        assert!(Cron::parse("60 * * * *").is_err());
        assert!(Cron::parse("*/0 * * * *").is_err());
    }
    #[test]
    fn dst_skips_gap_and_repeated_wall_time() {
        let gap = Schedule::Cron {
            expression: "30 2 * * *".into(),
            timezone: "America/Denver".into(),
        };
        assert_eq!(
            next(&gap, ts("2026-03-08T07:00:00Z")).unwrap(),
            Some(ts("2026-03-09T08:30:00Z"))
        );
        let repeated = Schedule::Cron {
            expression: "30 1 * * *".into(),
            timezone: "America/Denver".into(),
        };
        let first = next(&repeated, ts("2026-11-01T06:00:00Z"))
            .unwrap()
            .unwrap();
        assert_eq!(first, ts("2026-11-01T07:30:00Z"));
        assert_eq!(
            next(&repeated, first).unwrap(),
            Some(ts("2026-11-02T08:30:00Z"))
        );
    }
    #[test]
    fn claims_no_overlap_archive_and_manual_do_not_consume_schedule() {
        let (_dir, tasks) = fixture();
        let (job, revision) = installed(&tasks, Schedule::Once { at: 100 }, 100);
        let run = tasks.admit(&job, "manual:test", 90, true).unwrap();
        assert!(
            tasks
                .admit(&job, "manual:two", 91, true)
                .unwrap_err()
                .contains("already running")
        );
        assert_eq!(tasks.admit(&job, "manual:test", 90, true).unwrap(), run);
        tasks.evaluate(100).unwrap();
        let history = tasks
            .command("owner", "history", json!({"job_id":job}))
            .unwrap();
        assert_eq!(history.as_array().unwrap().len(), 2);
        assert!(
            history
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["revision_id"] == revision)
        );
        assert!(
            history
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["status"] == "skipped")
        );
        tasks.archive("owner", &job, true).unwrap();
        assert!(tasks.admit(&job, "manual:three", 101, true).is_err());
        tasks.archive("owner", &job, false).unwrap();
        tasks.evaluate(200).unwrap();
        let count: i64 = tasks
            .db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM task_runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }
    #[test]
    fn downtime_once_claims_once_and_grant_withdrawal_preserves_due() {
        let (_dir, tasks) = fixture();
        let (job, _) = installed(&tasks, Schedule::Once { at: 100 }, 100);
        tasks.grants.lock().unwrap().clear();
        tasks.evaluate(10000).unwrap();
        let claimed: bool = tasks
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT once_claimed FROM task_jobs WHERE id=?1",
                [&job],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!claimed);
        tasks.set_grant("owner".into(), vec![]);
        tasks.evaluate(10000).unwrap();
        tasks.evaluate(11000).unwrap();
        let count: i64 = tasks
            .db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM task_runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
    #[test]
    fn recurring_downtime_is_single_bounded_summary_and_backward_clock_no_replay() {
        let (_dir, tasks) = fixture();
        let (job, _) = installed(
            &tasks,
            Schedule::Cron {
                expression: "* * * * *".into(),
                timezone: "UTC".into(),
            },
            60000,
        );
        tasks.evaluate(86400000).unwrap();
        tasks.evaluate(60000).unwrap();
        let history = tasks
            .command("owner", "history", json!({"job_id":job}))
            .unwrap();
        assert_eq!(history.as_array().unwrap().len(), 1);
        assert_eq!(history[0]["status"], "skipped");
        let at: i64 = tasks
            .db
            .lock()
            .unwrap()
            .query_row("SELECT next_at FROM task_jobs WHERE id=?1", [job], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(at, 86460000);
    }
    #[test]
    fn revisions_pin_and_restart_does_not_replay_unknown() {
        let (dir, tasks) = fixture();
        let (job, old) = installed(&tasks, Schedule::Manual, 100);
        let run = tasks.admit(&job, "manual:old", 90, true).unwrap();
        let new = tasks
            .draft(
                "owner",
                Draft {
                    job_id: Some(job.clone()),
                    name: "new".into(),
                    source: "export default async function() {}".into(),
                    schedule: Schedule::Manual,
                    timeout_ms: 1000,
                    network: vec![],
                    notify_success: false,
                    notify_failure: true,
                },
            )
            .unwrap();
        tasks
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE task_jobs SET current=?2 WHERE id=?1",
                params![job, new["revision_id"].as_str().unwrap()],
            )
            .unwrap();
        let pinned: String = tasks
            .db
            .lock()
            .unwrap()
            .query_row("SELECT revision FROM task_runs WHERE id=?1", [run], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(pinned, old);
        drop(tasks);
        let reopened = Tasks::open(dir.path(), "/missing".into()).unwrap();
        let status: String = reopened
            .db
            .lock()
            .unwrap()
            .query_row("SELECT status FROM task_runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "interrupted");
    }
    #[test]
    fn shutdown_denies_all_admission() {
        let (_dir, tasks) = fixture();
        let (job, _) = installed(&tasks, Schedule::Manual, 100);
        tasks.shutdown();
        assert!(tasks.admit(&job, "manual:late", 101, true).is_err());
        tasks.evaluate(100).unwrap();
        assert!(
            tasks
                .command("owner", "run", json!({"job_id":job}))
                .is_err()
        );
    }
    #[test]
    fn durable_watermark_prevents_replay_after_history_rotation_and_clock_rollback() {
        let (_dir, tasks) = fixture();
        let (job, revision) = installed(
            &tasks,
            Schedule::Cron {
                expression: "* * * * *".into(),
                timezone: "UTC".into(),
            },
            60000,
        );
        tasks.evaluate(60000).unwrap();
        {
            let db = tasks.db.lock().unwrap();
            db.execute("DELETE FROM task_runs", []).unwrap();
            db.execute("UPDATE task_jobs SET next_at=60000 WHERE id=?1", [&job])
                .unwrap();
        }
        tasks.evaluate(60000).unwrap();
        let db = tasks.db.lock().unwrap();
        let count: i64 = db
            .query_row("SELECT count(*) FROM task_runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let watermark: String = db
            .query_row(
                "SELECT occurrence FROM task_watermarks WHERE job=?1 AND revision=?2",
                params![job, revision],
                |r| r.get(0),
            )
            .unwrap();
        assert!(watermark.ends_with("1970-01-01T00:01"));
    }
    #[test]
    fn extreme_timestamp_validation_and_impossible_cron_are_bounded() {
        let schedule = Schedule::Cron {
            expression: "0 0 31 2 *".into(),
            timezone: "Pacific/Kiritimati".into(),
        };
        let start = Instant::now();
        for at in [i64::MAX, i64::MIN, 253402128000000] {
            assert!(next(&schedule, at).is_err());
        }
        assert!(next(&schedule, 0).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn denied_due_scopes_cannot_starve_authorized_job() {
        let (_dir, tasks) = fixture();
        let (job, revision) = installed(&tasks, Schedule::Once { at: 2 }, 2);
        {
            let mut db = tasks.db.lock().unwrap();
            let tx = db.transaction().unwrap();
            for index in 0..300 {
                tx.execute("INSERT INTO task_jobs(id,deployment,name,current,next_at) VALUES(?1,'withdrawn','denied',?2,1)",params![format!("denied{index}"),revision]).unwrap();
            }
            tx.commit().unwrap();
        }
        tasks.evaluate(2).unwrap();
        let db = tasks.db.lock().unwrap();
        let count: i64 = db
            .query_row("SELECT count(*) FROM task_runs WHERE job=?1", [job], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }
    #[test]
    fn notifications_survive_log_truncation_and_delivery_failure_does_not_undo_outcome() {
        let (_dir, tasks) = fixture();
        let (job, _) = installed(&tasks, Schedule::Manual, 0);
        let run = tasks.admit(&job, "notify", 1, true).unwrap();
        let output = Arc::new(Mutex::new((Vec::new(), false)));
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut bytes = vec![b'x'; MAX_LOG * 2];
        bytes.extend_from_slice(b"\nPARACO_NOTIFICATION:{\"message\":\"after truncation\"}\n");
        drain_events(
            std::io::Cursor::new(bytes),
            output.clone(),
            Some(events.clone()),
        )
        .join()
        .unwrap();
        assert_eq!(*events.lock().unwrap(), vec!["after truncation"]);
        tasks.running.lock().unwrap().insert(
            run.clone(),
            Running {
                cancel: Arc::new(AtomicBool::new(false)),
                output: output.clone(),
                events,
            },
        );
        let history = tasks
            .command("owner", "history", json!({"job_id":job}))
            .unwrap();
        assert_eq!(history[0]["logs"].as_str().unwrap().len(), 1024);
        let live = tasks
            .command("owner", "logs", json!({"run_id":run}))
            .unwrap();
        assert_eq!(live["logs"].as_str().unwrap().len(), MAX_LOG);
        tasks.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_notification BEFORE INSERT ON task_notifications BEGIN SELECT RAISE(FAIL,'fixture inbox failure'); END;").unwrap();
        let draft: Draft = serde_json::from_value(
            tasks
                .command("owner", "revisions", json!({"job_id":job}))
                .unwrap()[0]["body"]
                .clone(),
        )
        .unwrap();
        tasks
            .finish(
                &run,
                &job,
                "owner",
                &draft,
                ("success".into(), "done".into(), false, String::new()),
            )
            .unwrap();
        let status: String = tasks
            .db
            .lock()
            .unwrap()
            .query_row("SELECT status FROM task_runs WHERE id=?1", [run], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status, "success");
    }
    #[test]
    fn notification_inbox_is_durable_before_run_completion() {
        let (_dir, tasks) = fixture();
        let (job, _) = installed(&tasks, Schedule::Manual, 0);
        let run = tasks.admit(&job, "live-notify", 1, true).unwrap();
        let events = Arc::new(Mutex::new(vec!["during execution".to_owned()]));
        let pump = NotificationPump::start(
            tasks.db.clone(),
            events,
            Arc::new(Mutex::new((Vec::new(), false))),
            "owner".into(),
            run.clone(),
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let count: i64 = tasks
                .db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM task_notifications WHERE run=?1",
                    [&run],
                    |r| r.get(0),
                )
                .unwrap();
            if count == 1 {
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        let status: String = tasks
            .db
            .lock()
            .unwrap()
            .query_row("SELECT status FROM task_runs WHERE id=?1", [run], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status, "pending");
        drop(pump);
    }
    #[test]
    fn log_snapshots_survive_restart_and_notification_failure() {
        let (dir, tasks) = fixture();
        let (job, _) = installed(&tasks, Schedule::Manual, 0);
        let run = tasks.admit(&job, "snapshot", 1, true).unwrap();
        tasks
            .db
            .lock()
            .unwrap()
            .execute("UPDATE task_runs SET status='running' WHERE id=?1", [&run])
            .unwrap();
        tasks.db.lock().unwrap().execute_batch("CREATE TRIGGER reject_notification BEFORE INSERT ON task_notifications BEGIN SELECT RAISE(FAIL,'fixture inbox failure'); END;").unwrap();
        let output = Arc::new(Mutex::new((b"durable prefix\n".to_vec(), false)));
        let events = Arc::new(Mutex::new(vec!["delivery failure independent".into()]));
        let pump = NotificationPump::start(
            tasks.db.clone(),
            events,
            output.clone(),
            "owner".into(),
            run.clone(),
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let logs: String = tasks
                .db
                .lock()
                .unwrap()
                .query_row("SELECT logs FROM task_runs WHERE id=?1", [&run], |r| {
                    r.get(0)
                })
                .unwrap();
            if logs == "durable prefix\n" {
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        {
            let mut output = output.lock().unwrap();
            output.0.extend_from_slice(b"next snapshot\n");
            output.1 = true;
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let (logs, truncated, status): (String, bool, String) = tasks
                .db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT logs,truncated,status FROM task_runs WHERE id=?1",
                    [&run],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .unwrap();
            assert_eq!(status, "running");
            if truncated && logs.ends_with("next snapshot\n") {
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        drop(pump);
        drop(tasks);
        let reopened = Tasks::open(dir.path(), "/missing".into()).unwrap();
        let (logs, truncated, status): (String, bool, String) = reopened
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT logs,truncated,status FROM task_runs WHERE id=?1",
                [run],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(logs, "durable prefix\nnext snapshot\n");
        assert!(truncated);
        assert_eq!(status, "interrupted");
    }
    #[test]
    fn publishing_code_or_restore_keeps_completed_once_until_date_changes() {
        let (_dir, tasks) = fixture();
        let (job, mut current) = installed(&tasks, Schedule::Once { at: 100 }, 100);
        tasks
            .db
            .lock()
            .unwrap()
            .execute(
                "UPDATE task_jobs SET once_claimed=1,next_at=NULL WHERE id=?1",
                [&job],
            )
            .unwrap();
        for (at, source) in [
            (
                100,
                "export default async function() {console.log('edited')}",
            ),
            (100, "export default async function() {}"),
            (200, "export default async function() {}"),
        ] {
            let proposal = tasks
                .draft(
                    "owner",
                    Draft {
                        job_id: Some(job.clone()),
                        name: "once".into(),
                        source: source.into(),
                        schedule: Schedule::Once { at },
                        timeout_ms: 1000,
                        network: vec![],
                        notify_success: false,
                        notify_failure: true,
                    },
                )
                .unwrap();
            let revision = proposal["revision_id"].as_str().unwrap();
            let prepared = tasks.root.join(revision);
            fs::create_dir(&prepared).unwrap();
            fs::write(prepared.join("task.ts"), source).unwrap();
            tasks
                .publish(
                    "owner",
                    &json!({"revision_id":revision,"expected_current_revision":current}),
                )
                .unwrap();
            current = revision.into();
            let (due, claimed): (Option<i64>, bool) = tasks
                .db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT next_at,once_claimed FROM task_jobs WHERE id=?1",
                    [&job],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(due, if at == 100 { None } else { Some(200) });
            assert_eq!(claimed, at == 100);
            tasks.evaluate(300).unwrap();
            let count: i64 = tasks
                .db
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM task_runs WHERE job=?1", [&job], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(count, if at == 100 { 0 } else { 1 });
        }
    }
    #[test]
    fn output_is_bounded_while_pipe_is_fully_drained() {
        let output = Arc::new(Mutex::new((vec![], false)));
        drain(
            std::io::Cursor::new(vec![b'x'; MAX_LOG * 4]),
            output.clone(),
        )
        .join()
        .unwrap();
        let out = output.lock().unwrap();
        assert_eq!(out.0.len(), MAX_LOG);
        assert!(out.1);
    }
    #[test]
    fn offline_preparation_checks_without_executing_top_level_and_rejects_imports() {
        let (_dir, tasks) = fixture();
        // Exercise the same directory alias that macOS uses for /var even on
        // hosts whose temporary directory already has a physical path.
        #[cfg(unix)]
        let tasks = {
            let alias = _dir.path().join("alias");
            std::os::unix::fs::symlink(_dir.path(), &alias).unwrap();
            Tasks::open(&alias, tasks.workerd.clone()).unwrap()
        };
        if Command::new(&tasks.workerd)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_err()
        {
            return;
        }
        let mut draft = Draft {
            job_id: None,
            name: "prep".into(),
            source: "throw new Error('must not execute'); export default async function() {}"
                .into(),
            schedule: Schedule::Manual,
            timeout_ms: 1000,
            network: vec![],
            notify_success: false,
            notify_failure: true,
        };
        tasks.prepare("valid", &draft).unwrap();
        draft.source = "export default async function(ctx) { syntax error here }".into();
        let error = tasks.prepare("syntax-error", &draft).unwrap_err();
        assert!(error.starts_with("task validation failed:"), "{error}");
        draft.source = "import 'file:///etc/passwd'; export default async function() {}".into();
        assert!(tasks.prepare("invalid", &draft).is_err());
        let private = _dir.path().join("private.ts");
        fs::write(
            &private,
            "export const secret: number = 'EXTERNAL_SECRET_MUST_NOT_LEAK';",
        )
        .unwrap();
        draft.source = format!(
            "import {{secret}} from '{}'; export default async function() {{console.log(secret)}}",
            url::Url::from_file_path(private).unwrap()
        );
        let error = tasks.prepare("private", &draft).unwrap_err();
        assert!(!error.contains("EXTERNAL_SECRET_MUST_NOT_LEAK"));
    }
}
