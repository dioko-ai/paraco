//! Durable host state. Deployment identities belong to canonical host sources,
//! never to application-controlled manifest names.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
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
    root: PathBuf,
    _lock: File,
    disk: Disk,
}

impl Store {
    pub fn open(root: &Path) -> Result<Self, String> {
        fs::create_dir_all(root).map_err(|e| format!("cannot create state root: {e}"))?;
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
        let path = root.join("state.json");
        let disk = if path.exists() {
            let disk: Disk = serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("invalid durable state: {e}"))?;
            if disk.format != FORMAT {
                return Err("durable state format predates source-bound identities; stop the old runtime and migrate or remove its state directory".into());
            }
            disk
        } else {
            Disk {
                format: FORMAT,
                deployments: BTreeMap::new(),
            }
        };
        Ok(Self {
            root: root.to_path_buf(),
            _lock: lock,
            disk,
        })
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
        let directory_synced = commit(&self.root, &proposed)?;
        self.disk = proposed;
        if !directory_synced {
            eprintln!(
                "paraco: state committed, but its containing directory could not be synchronized; retry before relying on crash durability"
            );
        }
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

fn commit(root: &Path, disk: &Disk) -> Result<bool, String> {
    let tmp = root.join("state.json.tmp");
    let bytes = serde_json::to_vec(disk).map_err(|e| e.to_string())?;
    let mut file = File::create(&tmp).map_err(|e| format!("cannot write durable state: {e}"))?;
    file.write_all(&bytes)
        .map_err(|e| format!("cannot write durable state: {e}"))?;
    file.sync_all()
        .map_err(|e| format!("cannot synchronize durable state: {e}"))?;
    fs::rename(&tmp, root.join("state.json"))
        .map_err(|e| format!("cannot commit durable state: {e}"))?;
    #[cfg(unix)]
    return Ok(File::open(root).and_then(|dir| dir.sync_all()).is_ok());
    #[cfg(not(unix))]
    Ok(true)
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
        fs::create_dir(state.join("state.json.tmp")).unwrap();
        assert!(store.stop(&app).is_err());
        assert_eq!(store.desired_running(&app), Some(true));
        fs::remove_dir(state.join("state.json.tmp")).unwrap();
        drop(store);
        let mut reopened = Store::open(&state).unwrap();
        assert!(reopened.install(&app).unwrap().desired_running);
        assert_eq!(deployment.id, reopened.install(&app).unwrap().id);
    }

    #[test]
    fn excludes_other_runtime() {
        let root = tempfile::tempdir().unwrap();
        let _store = Store::open(root.path()).unwrap();
        assert!(Store::open(root.path()).is_err());
    }
}
