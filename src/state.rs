//! Durable host state. This is deliberately outside application roots.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const FORMAT: u32 = 1;
#[derive(Clone, Serialize, Deserialize)]
pub struct Deployment {
    pub id: String,
    pub desired_running: bool,
}
#[derive(Serialize, Deserialize)]
struct Disk {
    format: u32,
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
        let path = root.join("state.json");
        let disk = if path.exists() {
            let d: Disk = serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("invalid durable state: {e}"))?;
            if d.format != FORMAT {
                return Err("unsupported durable state migration".into());
            };
            d
        } else {
            Disk {
                format: FORMAT,
                deployments: BTreeMap::new(),
            }
        };
        // Validate before claiming ownership: a malformed/unsupported state must
        // not strand a lock file that prevents the operator from recovering it.
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(root.join("runtime.lock"))
            .map_err(|e| format!("cannot open state ownership lease: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err("state root is already owned by another runtime".into());
            }
        }
        Ok(Self {
            root: root.to_path_buf(),
            _lock: lock,
            disk,
        })
    }
    pub fn install(&mut self, name: &str) -> Result<Deployment, String> {
        if name.is_empty() {
            return Err("deployment name is empty".into());
        };
        let deployment = self
            .disk
            .deployments
            .entry(name.into())
            .or_insert_with(|| Deployment {
                id: fresh_id(),
                desired_running: true,
            })
            .clone();
        self.commit()?;
        Ok(deployment)
    }
    pub fn start(&mut self, name: &str) -> Result<(), String> {
        let d = self
            .disk
            .deployments
            .get_mut(name)
            .ok_or("unknown deployment")?;
        d.desired_running = true;
        self.commit()
    }
    pub fn stop(&mut self, name: &str) -> Result<(), String> {
        let d = self
            .disk
            .deployments
            .get_mut(name)
            .ok_or("unknown deployment")?;
        d.desired_running = false;
        self.commit()
    }
    pub fn guardian_lease(&self) -> Result<File, String> {
        self._lock
            .try_clone()
            .map_err(|e| format!("cannot duplicate state ownership lease: {e}"))
    }
    pub fn desired_running(&self, name: &str) -> Option<bool> {
        self.disk.deployments.get(name).map(|d| d.desired_running)
    }
    pub fn remove(&mut self, name: &str) -> Result<(), String> {
        self.disk.deployments.remove(name);
        self.commit()
    }
    fn commit(&self) -> Result<(), String> {
        let tmp = self.root.join("state.json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec(&self.disk).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(tmp, self.root.join("state.json"))
            .map_err(|e| format!("cannot commit durable state: {e}"))
    }
}
fn fresh_id() -> String {
    format!(
        "d{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_survives_restart_and_reinstall_is_fresh() {
        let root = tempfile::tempdir().unwrap();
        let first = {
            let mut s = Store::open(root.path()).unwrap();
            s.install("a").unwrap()
        };
        let again = {
            let mut s = Store::open(root.path()).unwrap();
            s.install("a").unwrap()
        };
        assert_eq!(first.id, again.id);
        {
            let mut s = Store::open(root.path()).unwrap();
            s.remove("a").unwrap();
        }
        let fresh = {
            let mut s = Store::open(root.path()).unwrap();
            s.install("a").unwrap()
        };
        assert_ne!(first.id, fresh.id)
    }
    #[test]
    fn excludes_other_runtime() {
        let root = tempfile::tempdir().unwrap();
        let _s = Store::open(root.path()).unwrap();
        assert!(Store::open(root.path()).is_err())
    }
}
