//! Host-owned, immutable application preparation artifacts.
//!
//! Preparation is the only operation that may populate a Deno cache.  Launch
//! verifies the published metadata and runs from that cache with downloads
//! disabled.  This deliberately supports a small Deno configuration subset;
//! expanding it is a compatibility/security decision, not an accident of
//! Deno's ambient configuration discovery.
use crate::manifest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const FORMAT: u32 = 1;
const METADATA: &str = "paraco-artifact.json";

pub struct PreparedApp {
    pub app: manifest::App,
    pub runtime: PathBuf,
    pub cache: PathBuf,
    pub lock: PathBuf,
    pub config: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Metadata {
    format: u32,
    source_digest: String,
    lock_digest: String,
    cache_digest: String,
    runtime: PathBuf,
    runtime_version: String,
    has_config: bool,
}

pub fn prepare(source: &Path, output: &Path, deno: &Path) -> Result<(), String> {
    if output.exists() {
        return Err(format!(
            "artifact output {} already exists",
            output.display()
        ));
    }
    let source = source
        .canonicalize()
        .map_err(|e| format!("cannot read source app: {e}"))?;
    let runtime = deno
        .canonicalize()
        .map_err(|e| format!("cannot read Deno executable: {e}"))?;
    if !runtime.is_file() {
        return Err("Deno executable is not a file".into());
    }
    let version = deno_version(&runtime)?;
    let parent = output
        .parent()
        .ok_or("artifact output must have a parent directory")?;
    fs::create_dir_all(parent).map_err(|e| format!("cannot create artifact parent: {e}"))?;
    let staging = tempfile::Builder::new()
        .prefix(".paraco-prepare-")
        .tempdir_in(parent)
        .map_err(|e| format!("cannot create artifact staging directory: {e}"))?;
    let root = staging.path();
    let app_dir = root.join("app");
    copy_tree(&source, &app_dir)?;
    let app = manifest::load(&app_dir).map_err(|e| e.to_string())?;
    let config = validate_config(&app.root)?;
    validate_local_imports(&app.root)?;
    let cache = root.join("deno-cache");
    let lock = root.join("deno.lock");
    fs::create_dir(&cache).map_err(|e| format!("cannot create dependency cache: {e}"))?;
    let mut command = Command::new(&runtime);
    command
        .arg("cache")
        .arg("--quiet")
        .arg("--lock")
        .arg(&lock)
        .arg("--lock-write");
    if let Some(config) = &config {
        command.arg("--config").arg(config);
    } else {
        command.arg("--no-config");
    }
    let status = command
        .arg(&app.entrypoint)
        .env_clear()
        .env("DENO_DIR", &cache)
        .current_dir(&app.root)
        .status()
        .map_err(|e| format!("cannot start Deno preparation: {e}"))?;
    if !status.success() {
        return Err(format!("Deno preparation failed with {status}"));
    }
    let digest = tree_digest(&app_dir)?;
    let metadata = Metadata {
        format: FORMAT,
        source_digest: digest,
        lock_digest: file_digest(&lock)?,
        cache_digest: tree_digest(&cache)?,
        runtime,
        runtime_version: version,
        has_config: config.is_some(),
    };
    fs::write(
        root.join(METADATA),
        serde_json::to_vec_pretty(&metadata).map_err(|_| "cannot encode artifact metadata")?,
    )
    .map_err(|e| format!("cannot write artifact metadata: {e}"))?;
    let staged = staging.keep();
    fs::rename(&staged, output).map_err(|e| format!("cannot publish prepared artifact: {e}"))?;
    Ok(())
}

pub fn open(path: &Path) -> Result<PreparedApp, String> {
    let root = path
        .canonicalize()
        .map_err(|e| format!("cannot read prepared artifact: {e}"))?;
    let raw =
        fs::read(root.join(METADATA)).map_err(|e| format!("cannot read artifact metadata: {e}"))?;
    let metadata: Metadata =
        serde_json::from_slice(&raw).map_err(|e| format!("invalid artifact metadata: {e}"))?;
    if metadata.format != FORMAT {
        return Err(format!("unsupported artifact format {}", metadata.format));
    }
    let app_dir = root.join("app");
    if tree_digest(&app_dir)? != metadata.source_digest {
        return Err("prepared artifact source digest does not match metadata".into());
    }
    let app = manifest::load(&app_dir).map_err(|e| e.to_string())?;
    let runtime = metadata
        .runtime
        .canonicalize()
        .map_err(|e| format!("prepared Deno executable is unavailable: {e}"))?;
    if deno_version(&runtime)? != metadata.runtime_version {
        return Err("prepared Deno version no longer matches artifact metadata".into());
    }
    let lock = root.join("deno.lock");
    if !lock.is_file() {
        return Err("prepared artifact lockfile is missing".into());
    }
    let cache = root.join("deno-cache");
    if !cache.is_dir() {
        return Err("prepared artifact dependency cache is missing".into());
    }
    if file_digest(&lock)? != metadata.lock_digest {
        return Err("prepared artifact lockfile digest does not match metadata".into());
    }
    if tree_digest(&cache)? != metadata.cache_digest {
        return Err("prepared artifact dependency cache digest does not match metadata".into());
    }
    let config = if metadata.has_config {
        validate_config(&app.root)?
    } else {
        None
    };
    Ok(PreparedApp {
        app,
        runtime,
        cache,
        lock,
        config,
    })
}

fn deno_version(deno: &Path) -> Result<String, String> {
    let output = Command::new(deno)
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot inspect Deno version: {e}"))?;
    if !output.status.success() {
        return Err("Deno version command failed".into());
    }
    String::from_utf8(output.stdout).map_err(|_| "Deno version output is not UTF-8".into())
}

fn validate_config(root: &Path) -> Result<Option<PathBuf>, String> {
    let path = root.join("deno.json");
    if !path.exists() {
        return Ok(None);
    }
    let value: serde_json::Value = serde_json::from_slice(
        &fs::read(&path).map_err(|e| format!("cannot read deno.json: {e}"))?,
    )
    .map_err(|e| format!("invalid deno.json: {e}"))?;
    let object = value.as_object().ok_or("deno.json must be an object")?;
    if object.keys().any(|key| key != "imports") {
        return Err("deno.json supports only the `imports` field in prepared artifacts".into());
    }
    let imports = object
        .get("imports")
        .and_then(|v| v.as_object())
        .ok_or("deno.json `imports` must be an object")?;
    for value in imports.values() {
        let value = value
            .as_str()
            .ok_or("deno.json import values must be strings")?;
        if !(value.starts_with("https://") || value.starts_with("jsr:") || safe_relative(value)) {
            return Err("deno.json imports support only https, jsr, or contained ./ paths in prepared artifacts".into());
        }
    }
    Ok(Some(path))
}

fn safe_relative(value: &str) -> bool {
    value.starts_with("./") && !value.split('/').any(|part| part == "..")
}

fn validate_local_imports(root: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    for file in files.into_iter().filter(|p| {
        matches!(
            p.extension().and_then(|x| x.to_str()),
            Some("ts" | "tsx" | "js" | "jsx")
        )
    }) {
        let contents = fs::read_to_string(root.join(&file))
            .map_err(|e| format!("cannot read source import: {e}"))?;
        if contents.contains("../")
            || contents.contains("file:")
            || contents.contains("npm:")
            || contents.contains("from \"/")
            || contents.contains("from '/")
            || contents.contains("import \"/")
            || contents.contains("import '/")
        {
            return Err(format!(
                "unsupported escaping or ambient import in {}",
                file.display()
            ));
        }
    }
    Ok(())
}

fn file_digest(path: &Path) -> Result<String, String> {
    let mut hash = Sha256::new();
    hash.update(fs::read(path).map_err(|e| format!("cannot hash artifact file: {e}"))?);
    Ok(format!("{:x}", hash.finalize()))
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir(to).map_err(|e| format!("cannot create artifact source directory: {e}"))?;
    for entry in fs::read_dir(from).map_err(|e| format!("cannot read app source: {e}"))? {
        let entry = entry.map_err(|e| format!("cannot read source entry: {e}"))?;
        let kind = entry
            .file_type()
            .map_err(|e| format!("cannot inspect source entry: {e}"))?;
        if kind.is_symlink() {
            return Err(format!(
                "prepared sources cannot contain symlinks: {}",
                entry.path().display()
            ));
        }
        let destination = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &destination)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), destination)
                .map_err(|e| format!("cannot copy source file: {e}"))?;
        } else {
            return Err(format!(
                "prepared sources cannot contain special files: {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

fn tree_digest(root: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();
    let mut hash = Sha256::new();
    for relative in files {
        hash.update(relative.to_string_lossy().as_bytes());
        hash.update([0]);
        hash.update(
            fs::read(root.join(relative))
                .map_err(|e| format!("cannot hash artifact source: {e}"))?,
        );
        hash.update([0]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn collect_files(root: &Path, current: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(current).map_err(|e| format!("cannot scan artifact source: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            return Err("prepared artifact contains symlink".into());
        }
        if kind.is_dir() {
            collect_files(root, &entry.path(), files)?
        } else if kind.is_file() {
            files.push(
                entry
                    .path()
                    .strip_prefix(root)
                    .map_err(|_| "invalid artifact path")?
                    .to_path_buf(),
            )
        } else {
            return Err("prepared artifact contains special file".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_import_alias_configuration_is_accepted() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("deno.json"),
            r#"{"imports":{"lib/":"./lib/"}}"#,
        )
        .unwrap();
        assert!(validate_config(root.path()).unwrap().is_some());
        fs::write(root.path().join("deno.json"), r#"{"tasks":{"x":"echo x"}}"#).unwrap();
        assert!(validate_config(root.path()).is_err());
    }

    #[test]
    fn source_digest_changes_when_a_file_changes() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("app")).unwrap();
        let file = root.path().join("app/main.ts");
        fs::write(&file, "export default 1").unwrap();
        let first = tree_digest(&root.path().join("app")).unwrap();
        fs::write(&file, "export default 2").unwrap();
        assert_ne!(first, tree_digest(&root.path().join("app")).unwrap());
    }

    #[test]
    fn rejects_escaping_and_rooted_source_imports() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("main.ts"), "import '/tmp/secret.ts';").unwrap();
        assert!(validate_local_imports(root.path()).is_err());
        fs::write(root.path().join("main.ts"), "import '../secret.ts';").unwrap();
        assert!(validate_local_imports(root.path()).is_err());
    }

    #[test]
    fn rejects_existing_output_before_starting_deno() {
        let root = tempfile::tempdir().unwrap();
        assert!(prepare(root.path(), root.path(), Path::new("missing-deno")).is_err());
    }
}
