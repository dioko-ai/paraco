//! Bounded host-owned key/value data, scoped exclusively by deployment ID.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub const MAX_KEY: usize = 128;
pub const MAX_VALUE: usize = 16 * 1024;
pub const MAX_TOTAL: usize = 256 * 1024;
const FORMAT: u32 = 1;

#[derive(Default, Serialize, Deserialize)]
struct Disk {
    format: u32,
    values: BTreeMap<String, BTreeMap<String, Vec<u8>>>,
}

/// A host-owned store. Deployment IDs are selected by the host, never the app.
pub struct Store {
    root: Option<PathBuf>,
    disk: Disk,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            root: None,
            disk: Disk {
                format: FORMAT,
                values: BTreeMap::new(),
            },
        }
    }
}

impl Store {
    /// Opens a durable store rooted outside application directories.
    pub fn open(root: &Path) -> Result<Self, String> {
        fs::create_dir_all(root).map_err(|e| format!("cannot create storage root: {e}"))?;
        let path = root.join("storage.json");
        let disk = match fs::read(&path) {
            Ok(bytes) => {
                let disk: Disk =
                    serde_json::from_slice(&bytes).map_err(|e| format!("invalid storage: {e}"))?;
                if disk.format != FORMAT {
                    return Err("unsupported storage format".into());
                }
                validate(&disk)?;
                disk
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Disk {
                format: FORMAT,
                values: BTreeMap::new(),
            },
            Err(e) => return Err(format!("cannot read storage: {e}")),
        };
        Ok(Self {
            root: Some(root.to_path_buf()),
            disk,
        })
    }

    pub fn get(&self, deployment_id: &str, key: &str) -> Result<Option<Vec<u8>>, String> {
        valid_deployment(deployment_id)?;
        valid_key(key)?;
        Ok(self
            .disk
            .values
            .get(deployment_id)
            .and_then(|scope| scope.get(key))
            .cloned())
    }

    /// Atomically replaces one value after checking the resulting scope quota.
    pub fn put(&mut self, deployment_id: &str, key: &str, value: Vec<u8>) -> Result<(), String> {
        valid_deployment(deployment_id)?;
        valid_key(key)?;
        if value.len() > MAX_VALUE {
            return Err("storage value exceeds 16 KiB".into());
        }
        let scope = self.disk.values.entry(deployment_id.into()).or_default();
        let previous = scope.get(key).map_or(0, Vec::len);
        let total: usize = scope.values().map(Vec::len).sum();
        if total - previous + value.len() > MAX_TOTAL {
            return Err("storage quota exceeds 256 KiB".into());
        }
        scope.insert(key.into(), value);
        self.commit()
    }

    /// Removes data only when the host explicitly retires a deployment.
    pub fn remove_deployment(&mut self, deployment_id: &str) -> Result<(), String> {
        valid_deployment(deployment_id)?;
        self.disk.values.remove(deployment_id);
        self.commit()
    }

    /// A versioned portable backup. Secrets and other host state are not included.
    pub fn export(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(&self.disk).map_err(|e| e.to_string())
    }

    /// Replaces storage only after a fully validated, compatible backup is decoded.
    pub fn restore(&mut self, bytes: &[u8]) -> Result<(), String> {
        let disk: Disk =
            serde_json::from_slice(bytes).map_err(|e| format!("invalid storage backup: {e}"))?;
        if disk.format != FORMAT {
            return Err("unsupported storage backup format".into());
        }
        validate(&disk)?;
        self.commit_disk(&disk)?;
        self.disk = disk;
        Ok(())
    }

    fn commit(&self) -> Result<(), String> {
        let Some(root) = &self.root else {
            return Ok(());
        };
        self.commit_disk(&self.disk)
    }

    fn commit_disk(&self, disk: &Disk) -> Result<(), String> {
        let Some(root) = &self.root else {
            return Ok(());
        };
        let tmp = root.join("storage.json.tmp");
        fs::write(&tmp, serde_json::to_vec(disk).map_err(|e| e.to_string())?)
            .map_err(|e| format!("cannot write storage: {e}"))?;
        fs::rename(&tmp, root.join("storage.json"))
            .map_err(|e| format!("cannot commit storage: {e}"))
    }
}

fn valid_deployment(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err("invalid deployment scope".into());
    }
    Ok(())
}
fn valid_key(key: &str) -> Result<(), String> {
    if key.is_empty()
        || key.len() > MAX_KEY
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err("storage key must be 1-128 ASCII letters, digits, '.', '_' or '-'".into());
    }
    Ok(())
}
fn validate(disk: &Disk) -> Result<(), String> {
    for (id, scope) in &disk.values {
        valid_deployment(id)?;
        let mut total = 0;
        for (key, value) in scope {
            valid_key(key)?;
            if value.len() > MAX_VALUE {
                return Err("storage value exceeds 16 KiB".into());
            }
            total += value.len();
        }
        if total > MAX_TOTAL {
            return Err("storage quota exceeds 256 KiB".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scopes_quotas_and_persistence_are_isolated() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path()).unwrap();
        store.put("d-one", "key", b"one".to_vec()).unwrap();
        store.put("d-two", "key", b"two".to_vec()).unwrap();
        assert_eq!(store.get("d-one", "key").unwrap(), Some(b"one".to_vec()));
        assert_eq!(
            Store::open(root.path())
                .unwrap()
                .get("d-two", "key")
                .unwrap(),
            Some(b"two".to_vec())
        );
        assert!(store.put("d-one", "bad/key", vec![]).is_err());
        assert!(store.put("d-one", "large", vec![0; MAX_VALUE + 1]).is_err());
    }
    #[test]
    fn restore_is_versioned_and_atomic() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path()).unwrap();
        store.put("d-one", "old", b"old".to_vec()).unwrap();
        let backup = store.export().unwrap();
        assert!(store.restore(br#"{"format":2,"values":{}}"#).is_err());
        assert_eq!(store.get("d-one", "old").unwrap(), Some(b"old".to_vec()));
        store.remove_deployment("d-one").unwrap();
        store.restore(&backup).unwrap();
        assert_eq!(store.get("d-one", "old").unwrap(), Some(b"old".to_vec()));
    }
}
