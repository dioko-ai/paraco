//! Durable host state. Deployment identities belong to canonical host sources,
//! never to application-controlled manifest names.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

const FORMAT: u32 = 2;

#[derive(Clone, Serialize, Deserialize)]
pub struct Deployment {
    pub id: String,
    pub desired_running: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct Disk {
    format: u32,
    // Canonical application root -> host-owned deployment. The source is the
    // replacement boundary: changing a configured source allocates a new ID.
    deployments: BTreeMap<String, Deployment>,
}

pub struct Store {
    db: rusqlite::Connection,
    _lock: File,
    disk: Disk,
}

impl Store {
    pub fn open(root: &Path) -> Result<Self, String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(root)
                .map_err(|e| format!("cannot create state root: {e}"))?;
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let metadata = fs::symlink_metadata(root).map_err(|e| e.to_string())?;
            if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
                return Err("state root must be a private owned directory".into());
            }
            fs::set_permissions(root, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        #[cfg(not(unix))]
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("runtime.lock"))
            .map_err(|e| format!("cannot open state ownership lease: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err("state root is already owned by another runtime".into());
            }
        }
        let mut db =
            rusqlite::Connection::open(root.join("state.sqlite3")).map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")
            .map_err(|e| e.to_string())?;
        let version: u32 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if version > 1 {
            return Err("state schema is newer than this runtime".into());
        }
        if version == 0 {
            let legacy = root.join("state.json");
            let disk = if legacy.exists() {
                let disk: Disk =
                    serde_json::from_slice(&fs::read(legacy).map_err(|e| e.to_string())?)
                        .map_err(|e| format!("invalid legacy state: {e}"))?;
                if disk.format != FORMAT {
                    return Err("unsupported legacy state format".into());
                }
                disk
            } else {
                Disk {
                    format: FORMAT,
                    deployments: BTreeMap::new(),
                }
            };
            let tx = db.transaction().map_err(|e| e.to_string())?;
            tx.execute_batch("CREATE TABLE deployments(source TEXT PRIMARY KEY, id TEXT UNIQUE NOT NULL, desired INTEGER NOT NULL CHECK(desired IN (0,1)));
                CREATE TABLE storage(deployment TEXT NOT NULL, namespace TEXT NOT NULL CHECK(namespace IN ('config','data')), key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(deployment,namespace,key));
                PRAGMA user_version=1;") .map_err(|e| e.to_string())?;
            for (source, deployment) in disk.deployments {
                tx.execute(
                    "INSERT INTO deployments VALUES (?1,?2,?3)",
                    rusqlite::params![source, deployment.id, deployment.desired_running],
                )
                .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())?;
        }
        let deployments = {
            let mut query = db
                .prepare("SELECT source,id,desired FROM deployments")
                .map_err(|e| e.to_string())?;
            query
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        Deployment {
                            id: row.get(1)?,
                            desired_running: row.get(2)?,
                        },
                    ))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<BTreeMap<_, _>, _>>()
                .map_err(|e| e.to_string())?
        };
        Ok(Self {
            db,
            _lock: lock,
            disk: Disk {
                format: FORMAT,
                deployments,
            },
        })
    }

    /// Export a consistent SQLite snapshot. The destination must not exist.
    pub fn backup(&self, destination: &Path) -> Result<(), String> {
        if destination.exists() {
            return Err("backup destination already exists".into());
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let output = options.open(destination).map_err(|e| e.to_string())?;
        self.db
            .execute(
                "VACUUM INTO ?1",
                [destination.to_str().ok_or("invalid backup path")?],
            )
            .map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn storage(
        &mut self,
        deployment: &str,
        namespace: &str,
        key: &str,
        value: Option<serde_json::Value>,
        delete: bool,
    ) -> Result<serde_json::Value, String> {
        use rusqlite::OptionalExtension;
        if !self.disk.deployments.values().any(|d| d.id == deployment) {
            return Err("unknown deployment".into());
        }
        if !matches!(namespace, "config" | "data") || key.is_empty() || key.len() > 128 {
            return Err("invalid storage namespace or key".into());
        }
        if delete && value.is_some() {
            return Err("ambiguous storage operation".into());
        }
        if let Some(value) = value {
            let encoded = serde_json::to_string(&value).map_err(|e| e.to_string())?;
            if encoded.len() > 65536 {
                return Err("storage value exceeds 64 KiB".into());
            }
            let tx = self.db.transaction().map_err(|e| e.to_string())?;
            tx.execute("INSERT INTO storage VALUES (?1,?2,?3,?4) ON CONFLICT(deployment,namespace,key) DO UPDATE SET value=excluded.value", rusqlite::params![deployment,namespace,key,encoded]).map_err(|e| e.to_string())?;
            let size: u64 = tx.query_row("SELECT coalesce(sum(length(CAST(value AS BLOB))),0) + count(*)*128 FROM storage WHERE deployment=?1", [deployment], |r| r.get(0)).map_err(|e| e.to_string())?;
            if size > 8 * 1024 * 1024 {
                return Err("deployment storage exceeds 8 MiB".into());
            }
            tx.commit().map_err(|e| e.to_string())?;
        } else if delete {
            self.db
                .execute(
                    "DELETE FROM storage WHERE deployment=?1 AND namespace=?2 AND key=?3",
                    rusqlite::params![deployment, namespace, key],
                )
                .map_err(|e| e.to_string())?;
        }
        let value: Option<String> = self
            .db
            .query_row(
                "SELECT value FROM storage WHERE deployment=?1 AND namespace=?2 AND key=?3",
                rusqlite::params![deployment, namespace, key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        value
            .map(|v| serde_json::from_str(&v).map_err(|e| e.to_string()))
            .unwrap_or(Ok(serde_json::Value::Null))
    }

    pub fn install(&mut self, source: &Path) -> Result<Deployment, String> {
        let source = canonical_source(source)?;
        let mut proposed = self.disk.clone();
        let deployment = match proposed.deployments.get(&source) {
            Some(deployment) => deployment.clone(),
            None => {
                let deployment = Deployment {
                    id: fresh_id()?,
                    desired_running: true,
                };
                proposed.deployments.insert(source, deployment.clone());
                deployment
            }
        };
        self.publish(proposed)?;
        Ok(deployment)
    }

    /// Retire sources absent from the current server configuration. A later
    /// re-add is an explicit new installation and receives a fresh identity.
    pub fn reconcile(&mut self, sources: impl IntoIterator<Item = PathBuf>) -> Result<(), String> {
        let sources: Result<BTreeSet<_>, _> =
            sources.into_iter().map(|p| canonical_source(&p)).collect();
        let sources = sources?;
        let mut proposed = self.disk.clone();
        proposed
            .deployments
            .retain(|source, _| sources.contains(source));
        self.publish(proposed)
    }

    pub fn start(&mut self, source: &Path) -> Result<(), String> {
        self.set_desired(source, true)
    }
    pub fn stop(&mut self, source: &Path) -> Result<(), String> {
        self.set_desired(source, false)
    }

    fn set_desired(&mut self, source: &Path, desired_running: bool) -> Result<(), String> {
        let source = canonical_source(source)?;
        let mut proposed = self.disk.clone();
        let deployment = proposed
            .deployments
            .get_mut(&source)
            .ok_or("unknown deployment")?;
        deployment.desired_running = desired_running;
        self.publish(proposed)
    }

    pub fn guardian_lease(&self) -> Result<File, String> {
        self._lock
            .try_clone()
            .map_err(|e| format!("cannot duplicate state ownership lease: {e}"))
    }

    #[cfg(test)]
    fn desired_running(&self, source: &Path) -> Option<bool> {
        let source = canonical_source(source).ok()?;
        self.disk
            .deployments
            .get(&source)
            .map(|deployment| deployment.desired_running)
    }

    fn publish(&mut self, proposed: Disk) -> Result<(), String> {
        let tx = self.db.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM deployments", [])
            .map_err(|e| e.to_string())?;
        for (source, deployment) in &proposed.deployments {
            tx.execute(
                "INSERT INTO deployments VALUES (?1,?2,?3)",
                rusqlite::params![source, deployment.id, deployment.desired_running],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        self.disk = proposed;
        Ok(())
    }
}

fn canonical_source(source: &Path) -> Result<String, String> {
    source
        .canonicalize()
        .map_err(|e| format!("cannot canonicalize deployment source: {e}"))?
        .into_os_string()
        .into_string()
        .map_err(|_| "deployment source is not valid UTF-8".into())
}

fn fresh_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| "cannot generate deployment identity")?;
    Ok(format!(
        "d{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_tracks_canonical_source_and_retirement_is_fresh() {
        let root = tempfile::tempdir().unwrap();
        let one = root.path().join("one");
        let two = root.path().join("two");
        fs::create_dir(&one).unwrap();
        fs::create_dir(&two).unwrap();
        let state = root.path().join("state");
        let first = {
            let mut store = Store::open(&state).unwrap();
            store.install(&one).unwrap()
        };
        let again = {
            let mut store = Store::open(&state).unwrap();
            store.install(&one).unwrap()
        };
        assert_eq!(first.id, again.id);
        let replacement = {
            let mut store = Store::open(&state).unwrap();
            store.install(&two).unwrap()
        };
        assert_ne!(first.id, replacement.id);
        {
            let mut store = Store::open(&state).unwrap();
            store.reconcile([two.clone()]).unwrap();
        }
        let fresh = {
            let mut store = Store::open(&state).unwrap();
            store.install(&one).unwrap()
        };
        assert_ne!(first.id, fresh.id);
    }

    #[test]
    fn failed_commit_does_not_publish_memory() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("app");
        fs::create_dir(&app).unwrap();
        let state = root.path().join("state");
        let mut store = Store::open(&state).unwrap();
        let deployment = store.install(&app).unwrap();
        store.db.execute_batch("PRAGMA query_only=ON").unwrap();
        assert!(store.stop(&app).is_err());
        assert_eq!(store.desired_running(&app), Some(true));
        store.db.execute_batch("PRAGMA query_only=OFF").unwrap();
        drop(store);
        let mut reopened = Store::open(&state).unwrap();
        assert!(reopened.install(&app).unwrap().desired_running);
        assert_eq!(deployment.id, reopened.install(&app).unwrap().id);
    }

    #[test]
    fn migrates_legacy_and_exports_storage_without_reimporting() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("app");
        fs::create_dir(&app).unwrap();
        let source = canonical_source(&app).unwrap();
        let legacy = serde_json::json!({"format":2,"deployments":{source:{"id":"legacy-id","desired_running":false}}});
        fs::write(root.path().join("state.json"), legacy.to_string()).unwrap();
        let mut store = Store::open(root.path()).unwrap();
        let deployment = store.install(&app).unwrap();
        assert_eq!(deployment.id, "legacy-id");
        assert!(!deployment.desired_running);
        store
            .storage(
                &deployment.id,
                "data",
                "draft",
                Some(serde_json::json!({"source":"return 42"})),
                false,
            )
            .unwrap();
        assert!(
            store
                .storage("other", "data", "draft", None, false)
                .is_err()
        );
        store.backup(&root.path().join("backup.sqlite3")).unwrap();
        assert!(store.backup(&root.path().join("backup.sqlite3")).is_err());
        store.start(&app).unwrap();
        drop(store);
        let mut store = Store::open(root.path()).unwrap();
        assert!(store.install(&app).unwrap().desired_running);
        assert_eq!(
            store
                .storage(&deployment.id, "data", "draft", None, false)
                .unwrap()["source"],
            "return 42"
        );
        drop(store);
        let restore = root.path().join("restored");
        fs::create_dir(&restore).unwrap();
        fs::copy(
            root.path().join("backup.sqlite3"),
            restore.join("state.sqlite3"),
        )
        .unwrap();
        let mut restored = Store::open(&restore).unwrap();
        assert!(!restored.install(&app).unwrap().desired_running);
        assert_eq!(
            restored
                .storage(&deployment.id, "data", "draft", None, false)
                .unwrap()["source"],
            "return 42"
        );
    }

    #[test]
    fn rejects_newer_schema_and_failed_legacy_import_without_partial_schema() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("state.json"),
            r#"{"format":999,"deployments":{}}"#,
        )
        .unwrap();
        assert!(Store::open(root.path()).is_err());
        let db = rusqlite::Connection::open(root.path().join("state.sqlite3")).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            0
        );
        db.execute_batch("PRAGMA user_version=99").unwrap();
        drop(db);
        assert!(Store::open(root.path()).is_err());
    }

    #[test]
    fn migration_constraint_failure_rolls_back_schema_and_keeps_legacy() {
        let root = tempfile::tempdir().unwrap();
        let legacy = r#"{"format":2,"deployments":{"one":{"id":"duplicate","desired_running":true},"two":{"id":"duplicate","desired_running":false}}}"#;
        fs::write(root.path().join("state.json"), legacy).unwrap();
        assert!(Store::open(root.path()).is_err());
        let db = rusqlite::Connection::open(root.path().join("state.sqlite3")).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='deployments'",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            fs::read_to_string(root.path().join("state.json")).unwrap(),
            legacy
        );
    }

    #[test]
    fn excludes_other_runtime() {
        let root = tempfile::tempdir().unwrap();
        let _store = Store::open(root.path()).unwrap();
        assert!(Store::open(root.path()).is_err());
    }
}
