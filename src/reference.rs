//! Embedded first-run Cron application. Generated files have a private ownership
//! ledger; user data and edited files are never replaced by a binary update.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const MARKER: &str = ".paraco-cron-owner.json";
const LOCK: &str = ".launcher.lock";
const OWNER: &str = "paraco-embedded-cron-v1";
const ASSETS: &[(&str, &str)] = &[
    ("app/main.ts", include_str!("../examples/cron/main.ts")),
    (
        "app/index.html",
        include_str!("../examples/cron/index.html"),
    ),
    ("app/app.js", include_str!("../examples/cron/app.js")),
    (
        "app/paraco.json",
        include_str!("../examples/cron/paraco.json"),
    ),
];
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Ownership {
    owner: String,
    files: BTreeMap<String, String>,
}
#[derive(Debug)]
struct Generated {
    config: PathBuf,
    _lock: File,
}

pub fn serve(
    port: u16,
    log_dir: &Path,
    ai_config: Option<&Path>,
    network: Vec<String>,
) -> Result<(), String> {
    let generated = prepare(log_dir, ai_config, network)?;
    // Keep the lock and canonical generated source location alive throughout
    // serving. Updating the binary does not allocate a new deployment identity.
    crate::server::serve(&generated.config, port, log_dir)
}
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn private_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(error)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!(
            "refusing non-directory or symlink Cron root: {}",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err("Cron root must be owned by the current user".into());
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("Cron root must be private (permissions 0700)".into());
        }
    }
    Ok(())
}
fn create_directory(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}
fn regular_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(error)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "refusing non-file or symlink generated path: {}",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1 {
            return Err("generated Cron files must be privately owned without hard links".into());
        }
    }
    Ok(())
}
fn exclusive_file(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.write(true).read(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path).map_err(error)
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("missing generated parent")?)
            .map_err(error)?;
    temp.write_all(bytes).map_err(error)?;
    temp.as_file().sync_all().map_err(error)?;
    temp.persist(path).map_err(error)?;
    #[cfg(unix)]
    File::open(path.parent().unwrap())
        .and_then(|dir| dir.sync_all())
        .map_err(error)?;
    Ok(())
}
fn lock_root(path: &Path, created: bool) -> Result<File, String> {
    let lock = if created {
        exclusive_file(path)?
    } else {
        regular_file(path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        options.open(path).map_err(error)?
    };
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("Cron launcher root is already owned by another running launcher".into());
        }
    }
    #[cfg(not(unix))]
    return Err("Cron launcher currently requires Unix file locking".into());
    Ok(lock)
}
fn prepare(
    log_dir: &Path,
    ai_config: Option<&Path>,
    network: Vec<String>,
) -> Result<Generated, String> {
    let proposed = log_dir.with_file_name("cron");
    let parent = proposed
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(error)?;
    let root = parent.canonicalize().map_err(error)?.join("cron");
    let created = match create_directory(&root) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(e) => return Err(error(e)),
    };
    private_directory(&root)?;
    // Existing directories require our exclusively created marker. Never adopt
    // an arbitrary empty directory or an app copied there by a user.
    if created {
        let mut marker = exclusive_file(&root.join(MARKER))?;
        marker
            .write_all(
                &serde_json::to_vec(&Ownership {
                    owner: OWNER.into(),
                    files: BTreeMap::new(),
                })
                .map_err(error)?,
            )
            .map_err(error)?;
        marker.sync_all().map_err(error)?;
    }
    regular_file(&root.join(MARKER))
        .map_err(|_| "refusing Cron root without a valid ownership marker".to_string())?;
    let lock = lock_root(&root.join(LOCK), created)?;
    if fs::metadata(root.join(MARKER)).map_err(error)?.len() > 16384 {
        return Err("invalid Cron ownership marker".into());
    }
    let marker = fs::read(root.join(MARKER)).map_err(error)?;
    let mut ownership: Ownership =
        serde_json::from_slice(&marker).map_err(|_| "invalid Cron ownership marker")?;
    if ownership.owner != OWNER {
        return Err("refusing foreign Cron ownership marker".into());
    }
    match create_directory(&root.join("app")) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(error(e)),
    }
    private_directory(&root.join("app"))?;
    let ai_config = ai_config
        .map(|path| path.canonicalize().map_err(error))
        .transpose()?;
    let mut app = serde_json::json!({"path":"app","taskAuthoring":true,"taskNetwork":network});
    if let Some(path) = ai_config {
        app["aiConfig"] = serde_json::to_value(path).map_err(error)?;
    }
    let config = serde_json::to_string_pretty(&serde_json::json!({"apps":[app]})).map_err(error)?;
    let mut desired: Vec<(&str, &[u8])> = ASSETS
        .iter()
        .map(|(path, text)| (*path, text.as_bytes()))
        .collect();
    desired.push(("server.json", config.as_bytes()));
    // Validate every destination before replacing any source. On an interrupted
    // update, either the previous ledger digest or this binary's bytes are valid.
    for (name, bytes) in &desired {
        let path = root.join(name);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                regular_file(&path)?;
                if fs::metadata(&path).map_err(error)?.len() > 1024 * 1024 {
                    return Err(format!("refusing oversized generated Cron file: {name}"));
                }
                let actual = digest(&fs::read(&path).map_err(error)?);
                if ownership.files.get(*name) != Some(&actual) && actual != digest(bytes) {
                    return Err(format!(
                        "refusing to replace modified or unowned Cron file: {name}"
                    ));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(error(e)),
        }
    }
    for (name, bytes) in desired {
        let path = root.join(name);
        if fs::read(&path).ok().as_deref() != Some(bytes) {
            atomic_write(&path, bytes)?;
        }
        ownership.files.insert(name.into(), digest(bytes));
    }
    atomic_write(
        &root.join(MARKER),
        &serde_json::to_vec(&ownership).map_err(error)?,
    )?;
    Ok(Generated {
        config: root.join("server.json"),
        _lock: lock,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn materializes_embedded_app_with_stable_paths_and_preserves_user_data() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("space and \"quotes\"");
        fs::create_dir(&parent).unwrap();
        let logs = parent.join("logs");
        let ai = parent.join("AI \"settings\".json");
        fs::write(&ai, "{}").unwrap();
        let first = prepare(&logs, Some(&ai), vec!["example.com:443".into()]).unwrap();
        let config = first.config.clone();
        let root = config.parent().unwrap();
        for (file, expected) in ASSETS {
            assert_eq!(fs::read_to_string(root.join(file)).unwrap(), *expected);
        }
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(
            value["apps"][0]["aiConfig"],
            ai.canonicalize().unwrap().to_str().unwrap()
        );
        assert_eq!(value["apps"][0]["taskNetwork"][0], "example.com:443");
        fs::write(root.join("keep.txt"), "user data").unwrap();
        fs::create_dir(root.join("state")).unwrap();
        fs::write(root.join("state/keep"), "durable").unwrap();
        assert!(
            prepare(&logs, None, vec![])
                .unwrap_err()
                .contains("another running launcher")
        );
        drop(first);
        let second = prepare(&logs, None, vec![]).unwrap();
        assert_eq!(second.config, config);
        assert_eq!(
            fs::read_to_string(root.join("keep.txt")).unwrap(),
            "user data"
        );
        assert_eq!(
            fs::read_to_string(root.join("state/keep")).unwrap(),
            "durable"
        );
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert!(value["apps"][0].get("aiConfig").is_none());
    }
    #[test]
    fn refuses_foreign_roots_and_modified_generated_files() {
        let temp = tempfile::tempdir().unwrap();
        let logs = temp.path().join("logs");
        create_directory(&temp.path().join("cron")).unwrap();
        fs::write(temp.path().join("cron/unrelated"), "keep").unwrap();
        assert!(prepare(&logs, None, vec![]).is_err());
        fs::remove_dir_all(temp.path().join("cron")).unwrap();
        let generated = prepare(&logs, None, vec![]).unwrap();
        drop(generated);
        fs::write(temp.path().join("cron/app/main.ts"), "user edits").unwrap();
        assert!(
            prepare(&logs, None, vec![])
                .unwrap_err()
                .contains("modified or unowned")
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("cron/app/main.ts")).unwrap(),
            "user edits"
        );
    }
    #[cfg(unix)]
    #[test]
    fn refuses_symlink_root_assets_marker_and_app_directory() {
        use std::os::unix::fs::symlink;
        for target in ["root", "app", "app/main.ts", MARKER, LOCK] {
            let temp = tempfile::tempdir().unwrap();
            let logs = temp.path().join("logs");
            let outside = temp.path().join("outside");
            create_directory(&outside).unwrap();
            if target == "root" {
                symlink(&outside, temp.path().join("cron")).unwrap();
            } else {
                drop(prepare(&logs, None, vec![]).unwrap());
                let path = temp.path().join("cron").join(target);
                if path.is_dir() {
                    fs::remove_dir_all(&path).unwrap();
                } else {
                    fs::remove_file(&path).unwrap();
                }
                symlink(&outside, &path).unwrap();
            }
            assert!(
                prepare(&logs, None, vec![]).is_err(),
                "must refuse {target}"
            );
            assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        }
    }
}
