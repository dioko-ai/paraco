//! Immutable offline JavaScript packages with a pinned private workerd runtime.
use crate::{manifest, worker};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
const FORMAT: u32 = 3;
const METADATA: &str = "paraco-artifact.json";
pub struct PreparedApp {
    pub app: manifest::App,
    pub runtime: PathBuf,
    pub code: PathBuf,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Metadata {
    format: u32,
    source_digest: String,
    code_digest: String,
    runtime_version: String,
    runtime_digest: String,
}
pub fn prepare(source: &Path, output: &Path, runtime: &Path) -> Result<(), String> {
    if output.exists() {
        return Err("artifact output already exists".into());
    }
    let output = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join(output);
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    let runtime = runtime.canonicalize().map_err(|e| e.to_string())?;
    let version = runtime_version(&runtime)?;
    let parent = output.parent().ok_or("artifact output needs parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let staging = tempfile::Builder::new()
        .prefix(".paraco-prepare-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    let root = staging.path();
    let app_dir = root.join("app");
    copy_tree(&source, &app_dir)?;
    let app = manifest::load(&app_dir).map_err(|e| e.to_string())?;
    app.validate_target("local").map_err(|e| e.to_string())?;
    let code = root.join("code");
    fs::create_dir(&code).map_err(|e| e.to_string())?;
    worker::compile(&app.entrypoint, &code.join("app.js"), &app.root)?;
    fs::create_dir(root.join("runtime")).map_err(|e| e.to_string())?;
    fs::copy(&runtime, root.join("runtime/workerd")).map_err(|e| e.to_string())?;
    let metadata = Metadata {
        format: FORMAT,
        source_digest: tree_digest(&app_dir)?,
        code_digest: tree_digest(&code)?,
        runtime_version: version,
        runtime_digest: file_digest(&runtime)?,
    };
    fs::write(
        root.join(METADATA),
        serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(root, &output).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn open(path: &Path) -> Result<PreparedApp, String> {
    let root = path.canonicalize().map_err(|e| e.to_string())?;
    let metadata: Metadata =
        serde_json::from_slice(&fs::read(root.join(METADATA)).map_err(|e| e.to_string())?)
            .map_err(|e| format!("invalid workerd artifact metadata: {e}"))?;
    if metadata.format != FORMAT {
        return Err("unsupported prepared artifact; prepare again for workerd".into());
    }
    let app_dir = root.join("app");
    let code = root.join("code");
    let runtime = root.join("runtime/workerd");
    if tree_digest(&app_dir)? != metadata.source_digest
        || tree_digest(&code)? != metadata.code_digest
        || file_digest(&runtime)? != metadata.runtime_digest
    {
        return Err("prepared artifact integrity check failed".into());
    }
    if runtime_version(&runtime)? != metadata.runtime_version {
        return Err("prepared workerd version mismatch".into());
    }
    let app = manifest::load(&app_dir).map_err(|e| e.to_string())?;
    app.validate_target("local").map_err(|e| e.to_string())?;
    Ok(PreparedApp { app, runtime, code })
}
fn runtime_version(runtime: &Path) -> Result<String, String> {
    let result = Command::new(runtime)
        .arg("--version")
        .output()
        .map_err(|e| e.to_string())?;
    let version = String::from_utf8(result.stdout).map_err(|e| e.to_string())?;
    if !result.status.success() || !version.starts_with("workerd ") {
        return Err("invalid workerd executable".into());
    }
    Ok(version)
}
fn file_digest(path: &Path) -> Result<String, String> {
    if !fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_file()
    {
        return Err("artifact file must be a regular file, not a symlink".into());
    }
    let mut hash = Sha256::new();
    hash.update(fs::read(path).map_err(|e| format!("cannot hash artifact file: {e}"))?);
    Ok(format!("{:x}", hash.finalize()))
}

pub(crate) fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
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
    if !fs::symlink_metadata(root)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_dir()
    {
        return Err("artifact directory must not be a symlink".into());
    }
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
