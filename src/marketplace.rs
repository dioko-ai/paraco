//! Curated source packages. Catalog membership approves exact bytes, never grants capabilities.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const MAX_CATALOG: u64 = 1024 * 1024;
const MAX_PACKAGE: u64 = 32 * 1024 * 1024;
const MAX_FILES: usize = 1024;

#[derive(clap::Subcommand)]
pub enum Command {
    /// Validate and list approved versions. Defaults to the catalog shipped in this binary.
    List {
        #[arg(long)]
        catalog: Option<PathBuf>,
    },
    /// Download, verify and install an approved version without executing app code.
    Install {
        name: String,
        #[arg(long)]
        catalog: Option<PathBuf>,
        /// Select an approved version; required when the catalog lists more than one.
        #[arg(long)]
        version: Option<String>,
        /// New directory to create. Existing destinations are never updated.
        #[arg(long)]
        output: PathBuf,
        /// Verify a previously downloaded package instead of using the network.
        #[arg(long)]
        package: Option<PathBuf>,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Catalog {
    schema_version: u32,
    apps: Vec<Entry>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry {
    name: String,
    title: String,
    description: String,
    repository: String,
    version: String,
    commit: String,
    package: String,
    sha256: String,
    capabilities: Vec<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Package {
    schema_version: u32,
    name: String,
    version: String,
    files: Vec<PackageFile>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PackageFile {
    path: String,
    content: String,
}
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn version(value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|s| {
            !s.is_empty()
                && s.len() <= 9
                && (s.len() == 1 || !s.starts_with('0'))
                && s.bytes().all(|b| b.is_ascii_digit())
        })
}
fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(error)?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() as u64 > limit {
        return Err("file exceeds size limit".into());
    }
    Ok(bytes)
}
fn parse_catalog(bytes: &[u8]) -> Result<Catalog, String> {
    if bytes.len() as u64 > MAX_CATALOG {
        return Err("catalog exceeds size limit".into());
    }
    let catalog: Catalog = serde_norway::from_slice(bytes).map_err(error)?;
    if catalog.schema_version != 1 || catalog.apps.len() > 1000 {
        return Err("unsupported catalog schema or too many entries".into());
    }
    let mut seen = BTreeSet::new();
    for entry in &catalog.apps {
        paraco::slug::validate(&entry.name).map_err(error)?;
        if !version(&entry.version) || !hex(&entry.sha256, 64) || !hex(&entry.commit, 40) {
            return Err(format!(
                "invalid version, SHA-256 or source commit for {}",
                entry.name
            ));
        }
        if entry.title.is_empty()
            || entry.description.is_empty()
            || !seen.insert((&entry.name, &entry.version))
        {
            return Err("empty catalog metadata or duplicate app version".into());
        }
        let repo = entry
            .repository
            .strip_prefix("https://github.com/")
            .ok_or("repository must be an HTTPS GitHub repository")?;
        let parts: Vec<_> = repo.split('/').collect();
        if parts.len() != 2
            || parts.iter().any(|p| {
                p.is_empty()
                    || p.starts_with('.')
                    || !p
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            })
        {
            return Err("invalid GitHub repository".into());
        }
        let expected = format!(
            "{}/releases/download/v{}/{}.paraco.json",
            entry.repository, entry.version, entry.name
        );
        if entry.package != expected {
            return Err(
                "package must be the versioned release asset in its declared repository".into(),
            );
        }
        let mut caps = BTreeSet::new();
        for cap in &entry.capabilities {
            if !["ai", "storage", "tasks"].contains(&cap.as_str()) || !caps.insert(cap) {
                return Err("invalid or duplicate capability".into());
            }
        }
    }
    Ok(catalog)
}
fn catalog(path: Option<&Path>) -> Result<Catalog, String> {
    match path {
        Some(path) => parse_catalog(&bounded(path, MAX_CATALOG)?),
        None => parse_catalog(include_bytes!("../marketplace/catalog.yaml")),
    }
}
fn select<'a>(
    catalog: &'a Catalog,
    name: &str,
    version: Option<&str>,
) -> Result<&'a Entry, String> {
    let entries: Vec<_> = catalog
        .apps
        .iter()
        .filter(|e| e.name == name && version.is_none_or(|v| e.version == v))
        .collect();
    match entries.as_slice() {
        [entry] => Ok(entry),
        [] => Err("app/version is not approved in this catalog".into()),
        _ => Err("multiple approved versions; specify --version".into()),
    }
}
async fn download(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(error)?;
    let mut response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, "paraco-marketplace/1")
        .send()
        .await
        .map_err(error)?
        .error_for_status()
        .map_err(error)?;
    if response.content_length().is_some_and(|n| n > MAX_PACKAGE) {
        return Err("package exceeds size limit".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(error)? {
        if bytes.len() + chunk.len() > MAX_PACKAGE as usize {
            return Err("package exceeds size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
// Portable names prevent traversal, Windows device names and case-folding collisions.
fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 240
        && path.split('/').all(|part| {
            let stem = part.split('.').next().unwrap_or("").to_ascii_lowercase();
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.starts_with('.')
                && !part.ends_with('.')
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                && ![
                    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6",
                    "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7",
                    "lpt8", "lpt9",
                ]
                .contains(&stem.as_str())
        })
}
// Atomically refuse an existing destination, including one created during staging.
fn publish_new(source: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::ffi::OsStrExt;
        let source = std::ffi::CString::new(source.as_os_str().as_bytes()).map_err(error)?;
        let destination =
            std::ffi::CString::new(destination.as_os_str().as_bytes()).map_err(error)?;
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(target_os = "macos")]
        let result =
            unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
        if result != 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        fs::rename(source, destination).map_err(error)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (source, destination);
        Err("atomic installation is unsupported on this platform".into())
    }
}
fn install(entry: &Entry, bytes: &[u8], output: &Path) -> Result<(), String> {
    if bytes.len() as u64 > MAX_PACKAGE || digest(bytes) != entry.sha256 {
        return Err("package SHA-256 does not match the approved catalog".into());
    }
    let package: Package = serde_json::from_slice(bytes).map_err(error)?;
    if package.schema_version != 1 || package.name != entry.name || package.version != entry.version
    {
        return Err("package identity/version does not match approval".into());
    }
    if package.files.is_empty() || package.files.len() > MAX_FILES {
        return Err("invalid package file count".into());
    }
    let mut seen = BTreeSet::new();
    let mut decoded = Vec::new();
    for file in package.files {
        let folded = file.path.to_ascii_lowercase();
        if !valid_path(&file.path) || folded == "paraco-install.json" || !seen.insert(folded) {
            return Err("unsafe, reserved or duplicate package path".into());
        }
        decoded.push((file.path, STANDARD.decode(&file.content).map_err(error)?));
    }
    // Reject file/directory conflicts including case-insensitive parent collisions.
    let mut spelling = std::collections::BTreeMap::new();
    for (path, _) in &decoded {
        let mut prefix = String::new();
        for part in path.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            let folded = prefix.to_ascii_lowercase();
            if let Some(previous) = spelling.insert(folded.clone(), prefix.clone())
                && previous != prefix
            {
                return Err("case-colliding paths".into());
            }
            if prefix != *path && seen.contains(&folded) {
                return Err("file/directory path conflict".into());
            }
        }
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(error)?;
    let parent = parent.canonicalize().map_err(error)?;
    let name = output
        .file_name()
        .ok_or("output must name a new directory")?;
    let output = parent.join(name);
    // Serialize installers in a destination parent; don't leave a stale lock on failure.
    let lock_path = parent.join(".paraco-install.lock");
    let lock = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .map_err(|e| format!("cannot acquire install lock (another install or stale lock): {e}"))?;
    struct Lock(PathBuf, fs::File);
    impl Drop for Lock {
        fn drop(&mut self) {
            let _ = self.1.sync_all();
            let _ = fs::remove_file(&self.0);
        }
    }
    let _lock = Lock(lock_path, lock);
    if fs::symlink_metadata(&output).is_ok() {
        return Err("installation destination already exists; choose a new directory".into());
    }
    let staging = tempfile::Builder::new()
        .prefix(".paraco-install-")
        .tempdir_in(&parent)
        .map_err(error)?;
    for (path, content) in decoded {
        let target = staging.path().join(path);
        fs::create_dir_all(target.parent().unwrap()).map_err(error)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .map_err(error)?;
        file.write_all(&content).map_err(error)?;
    }
    let app = crate::manifest::load(staging.path()).map_err(error)?;
    let capabilities: BTreeSet<_> = [
        ("ai", app.requests_ai),
        ("storage", app.requests_storage),
        ("tasks", app.requests_tasks),
    ]
    .into_iter()
    .filter_map(|(c, requested)| requested.then_some(c))
    .collect();
    if app.name != entry.name
        || capabilities != entry.capabilities.iter().map(String::as_str).collect()
    {
        return Err("manifest name/capabilities differ from the approved catalog".into());
    }
    fs::write(
        staging.path().join("paraco-install.json"),
        serde_json::to_vec_pretty(entry).map_err(error)?,
    )
    .map_err(error)?;
    publish_new(staging.path(), &output)?;
    Ok(())
}
pub fn execute(command: Command) -> Result<(), String> {
    match command {
        Command::List { catalog: path } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&catalog(path.as_deref())?).map_err(error)?
            );
            Ok(())
        }
        Command::Install {
            name,
            catalog: path,
            version,
            output,
            package,
        } => {
            let catalog = catalog(path.as_deref())?;
            let entry = select(&catalog, &name, version.as_deref())?;
            let bytes = match package {
                Some(path) => bounded(&path, MAX_PACKAGE)?,
                None => tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(error)?
                    .block_on(download(&entry.package))?,
            };
            install(entry, &bytes, &output)?;
            println!(
                "Installed {} {} at {}. Run with: paraco run {}",
                entry.name,
                entry.version,
                output.display(),
                output.display()
            );
            Ok(())
        }
    }
}

#[cfg(test)]
#[path = "marketplace_tests.rs"]
mod tests;
