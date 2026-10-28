//! Stable runtime data locations, independent of diagnostic log destinations.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub struct Directory {
    pub root: PathBuf,
    explicit: bool,
}

impl Directory {
    pub fn resolve(explicit: Option<&Path>) -> Result<Self, String> {
        let environment = std::env::var_os("PARACO_DATA_DIR").map(PathBuf::from);
        let selected = explicit.or(environment.as_deref());
        let root = match selected {
            Some(path) if path.as_os_str().is_empty() => {
                return Err("data directory must not be empty".into());
            }
            Some(path) => path.to_path_buf(),
            None => PathBuf::from(
                std::env::var_os("HOME")
                    .ok_or("HOME is unset; specify --data-dir or PARACO_DATA_DIR")?,
            )
            .join(".paraco"),
        };
        let root = if root.is_absolute() {
            root
        } else {
            std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(root)
        };
        Ok(Self {
            root,
            explicit: selected.is_some(),
        })
    }

    pub fn hosted(&self, config: &Path, log_dir: Option<&Path>) -> Result<PathBuf, String> {
        let key = host_key(config)?;
        let normal = self.root.join("state").join(&key);
        // Older user services nested state below ~/.paraco/state. Keep their
        // source locations intact: moving downloaded plugins would change identity.
        let old_service = self.root.join("state/state").join(&key);
        let chosen = if !self.explicit && has_state(&old_service) {
            if has_state(&normal) {
                return Err(format!(
                    "Two existing data stores were found. Choose the foreground data with --data-dir {} or the older service data with --data-dir {}. Neither store has been changed.",
                    self.root.display(),
                    self.root.join("state").display()
                ));
            }
            old_service
        } else {
            normal
        };
        if let Some(log_dir) = log_dir {
            self.check_old_logs(log_dir, "state", &key, &chosen)?;
        }
        Ok(chosen)
    }

    pub fn standalone(&self, source: &Path, log_dir: &Path) -> Result<PathBuf, String> {
        let key = source_key(source)?;
        let chosen = self.root.join("standalone-state").join(&key);
        self.check_old_logs(log_dir, "standalone-state", &key, &chosen)?;
        Ok(chosen)
    }

    pub fn cron_root(&self, log_dir: &Path) -> Result<PathBuf, String> {
        let chosen = self.root.join("cron");
        let old = absolute(&log_dir.with_file_name("cron"))?;
        if !self.explicit && old != chosen && old.join(".paraco-cron-owner.json").is_file() {
            return Err(recovery(&old));
        }
        Ok(chosen)
    }

    #[cfg(test)]
    pub fn service_root(&self, config: &Path) -> Result<PathBuf, String> {
        let state = self.hosted(config, None)?;
        Ok(state.parent().unwrap().parent().unwrap().to_path_buf())
    }

    fn check_old_logs(
        &self,
        logs: &Path,
        namespace: &str,
        key: &str,
        chosen: &Path,
    ) -> Result<(), String> {
        let old = absolute(&logs.with_file_name(namespace).join(key))?;
        if !self.explicit && old != chosen && has_state(&old) {
            return Err(recovery(old.parent().unwrap()));
        }
        Ok(())
    }
}
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path))
    }
}
pub fn host_key(config: &Path) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(config).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    match value.get("host_id") {
        None => source_key(config),
        Some(value) => {
            let id = value
                .as_str()
                .ok_or("host_id must be a 64-character lowercase hexadecimal identity")?;
            if id.len() != 64
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err("host_id must be a 64-character lowercase hexadecimal identity".into());
            }
            Ok(id.into())
        }
    }
}

impl Directory {
    pub fn discover(&self, config: Option<&Path>) -> Result<(), String> {
        if let Some(config) = config {
            println!(
                "State: {}\nTo relocate this configuration, preserve this top-level field:\n  \"host_id\": \"{}\"\nKeep the same --data-dir and resolve relative app/AI paths before moving it. Stop the host first.",
                self.hosted(config, None)?.display(),
                host_key(config)?
            );
        } else {
            for namespace in [self.root.join("state"), self.root.join("state/state")] {
                if !namespace.exists() {
                    continue;
                }
                for entry in std::fs::read_dir(namespace).map_err(|e| e.to_string())? {
                    let path = entry.map_err(|e| e.to_string())?.path();
                    if has_state(&path) {
                        println!(
                            "{} — host_id: {}",
                            path.display(),
                            path.file_name().unwrap().to_string_lossy()
                        );
                        if let Ok(db) = rusqlite::Connection::open_with_flags(
                            path.join("state.sqlite3"),
                            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                        ) && let Ok(mut query) =
                            db.prepare("SELECT source FROM deployments ORDER BY source")
                            && let Ok(rows) = query.query_map([], |row| row.get::<_, String>(0))
                        {
                            for source in rows.flatten() {
                                println!("  App source: {source}");
                            }
                        }
                    }
                }
            }
            println!(
                "Use the original store's host_id in the relocated configuration; keep the same data directory. Back up the store before recovery. No data was changed."
            );
        }
        Ok(())
    }
}

fn source_key(source: &Path) -> Result<String, String> {
    let source = source
        .canonicalize()
        .map_err(|e| format!("cannot resolve data source: {e}"))?;
    Ok(format!(
        "{:x}",
        Sha256::digest(source.as_os_str().as_encoded_bytes())
    ))
}
fn has_state(path: &Path) -> bool {
    path.join("state.sqlite3").is_file() || path.join("state.json").is_file()
}
fn recovery(namespace: &Path) -> String {
    format!(
        "Existing data was found beside the old log directory. Logs no longer select application data. Run again with --data-dir {} to keep using that data. No data was moved.",
        namespace.parent().unwrap_or(Path::new(".")).display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_identity_preserves_store_when_configuration_moves() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old.json");
        let moved = temp.path().join("moved.json");
        std::fs::write(&old, "{}").unwrap();
        let data = Directory {
            root: temp.path().join("data"),
            explicit: true,
        };
        let original = data.hosted(&old, None).unwrap();
        let id = host_key(&old).unwrap();
        std::fs::write(&old, serde_json::json!({"host_id":id}).to_string()).unwrap();
        std::fs::rename(&old, &moved).unwrap();
        assert_eq!(original, data.hosted(&moved, None).unwrap());
        std::fs::write(&moved, r#"{"host_id":"../escape"}"#).unwrap();
        assert!(data.hosted(&moved, None).is_err());
    }
    #[test]
    fn log_destinations_do_not_change_state_or_service_identity() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("server.json");
        std::fs::write(&config, "{}").unwrap();
        let data = Directory {
            root: temp.path().join("data"),
            explicit: true,
        };
        let one = data
            .hosted(&config, Some(&temp.path().join("one/logs")))
            .unwrap();
        let two = data
            .hosted(&config, Some(&temp.path().join("two/logs")))
            .unwrap();
        assert_eq!(one, two);
        assert_eq!(data.service_root(&config).unwrap(), data.root);
        assert_eq!(
            data.standalone(&config, Path::new("/a/logs")).unwrap(),
            data.standalone(&config, Path::new("/b/logs")).unwrap()
        );
        assert_eq!(
            data.cron_root(Path::new("/a/logs")).unwrap(),
            data.cron_root(Path::new("/b/logs")).unwrap()
        );
    }
    #[test]
    fn old_service_store_is_reused_and_ambiguity_requires_selection() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("server.json");
        std::fs::write(&config, "{}").unwrap();
        let data = Directory {
            root: temp.path().join("data"),
            explicit: false,
        };
        let old = data
            .root
            .join("state/state")
            .join(source_key(&config).unwrap());
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("state.sqlite3"), "").unwrap();
        assert_eq!(data.hosted(&config, None).unwrap(), old);
        assert_eq!(data.service_root(&config).unwrap(), data.root.join("state"));
        let normal = data.root.join("state").join(source_key(&config).unwrap());
        std::fs::create_dir_all(&normal).unwrap();
        std::fs::write(normal.join("state.sqlite3"), "").unwrap();
        assert!(
            data.hosted(&config, None)
                .unwrap_err()
                .contains("Two existing")
        );
        assert_eq!(
            Directory {
                root: data.root,
                explicit: true
            }
            .hosted(&config, None)
            .unwrap(),
            normal
        );
    }
    #[test]
    fn old_custom_log_state_is_not_silently_abandoned() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("app");
        std::fs::create_dir(&source).unwrap();
        let logs = temp.path().join("custom/logs");
        let old = logs
            .with_file_name("standalone-state")
            .join(source_key(&source).unwrap());
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("state.sqlite3"), "").unwrap();
        let data = Directory {
            root: temp.path().join("data"),
            explicit: false,
        };
        let error = data.standalone(&source, &logs).unwrap_err();
        assert!(error.contains("--data-dir"));
        assert!(error.contains(temp.path().join("custom").to_str().unwrap()));
        assert!(old.join("state.sqlite3").exists());
    }
}
