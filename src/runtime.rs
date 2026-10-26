//! Resolution of the workerd executable used by a packaged Paraco release.
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BundleMetadata {
    paraco_version: String,
    workerd_version: String,
    target: String,
    source_revision: String,
    // Local developer bundles may be dirty; publisher release verification
    // enforces cleanliness. Older local bundle metadata omitted this marker.
    #[serde(rename = "sourceDirty", default)]
    _source_dirty: Option<bool>,
    notices: Vec<String>,
    checksums: BTreeMap<String, String>,
    trusted_input_provenance: String,
}

pub fn command_for_host() -> Result<PathBuf, String> {
    let executable =
        std::env::current_exe().map_err(|e| format!("cannot locate Paraco executable: {e}"))?;
    resolve_from_executable(&executable)
}

fn resolve_from_executable(executable: &Path) -> Result<PathBuf, String> {
    let executable = executable
        .canonicalize()
        .map_err(|e| format!("cannot resolve Paraco executable: {e}"))?;
    let release = executable.parent().and_then(Path::parent);
    let bundled = release.map(|root| root.join("libexec/paraco/workerd"));
    let metadata = release.map(|root| root.join("bundle.json"));
    // A bundle is identified by its metadata, never by the directory name of
    // the launcher. Source installs commonly also put the binary in `bin`.
    let expected = metadata
        .as_deref()
        .filter(|p| p.exists())
        .map(read_version)
        .transpose()?;
    if let Some(override_path) = std::env::var_os("PARACO_WORKERD") {
        let expected = expected
            .or_else(|| std::env::var("PARACO_WORKERD_VERSION").ok())
            .ok_or(
                "PARACO_WORKERD requires PARACO_WORKERD_VERSION when not running a release bundle",
            )?;
        return validate(
            PathBuf::from(override_path),
            &expected,
            "development override",
        );
    }
    if let (Some(path), Some(expected)) = (bundled, expected) {
        return validate(path, &expected, "private bundled workerd");
    }
    resolve_path_runtime()
}

fn resolve_path_runtime() -> Result<PathBuf, String> {
    let path = std::env::var_os("PATH")
        .ok_or("workerd was not found on PATH; install workerd and try again")?;
    resolve_path_runtime_from(&path)
}

fn resolve_path_runtime_from(path: &std::ffi::OsStr) -> Result<PathBuf, String> {
    for directory in std::env::split_paths(path) {
        let candidate = directory.join("workerd");
        if executable_file(&candidate) {
            return candidate.canonicalize().map_err(|e| {
                format!(
                    "cannot resolve workerd found on PATH at {}: {e}",
                    candidate.display()
                )
            });
        }
    }
    Err("workerd was not found on PATH; install workerd and try again".into())
}

fn read_version(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read release metadata: {e}"))?;
    let metadata: BundleMetadata =
        serde_json::from_slice(&bytes).map_err(|e| format!("invalid release metadata: {e}"))?;
    // Deserialize the whole release contract. The fields besides the workerd pin
    // are release provenance, but accepting only a hand-written subset lets
    // builder and launcher silently drift.
    if metadata.paraco_version.is_empty()
        || metadata.target.is_empty()
        || metadata.source_revision.is_empty()
        || metadata.notices.is_empty()
        || !metadata.checksums.contains_key("bin/paraco")
        || !metadata.checksums.contains_key("libexec/paraco/workerd")
        || !metadata.checksums.contains_key("libexec/paraco/esbuild")
        || metadata.trusted_input_provenance.is_empty()
    {
        return Err("release metadata is incomplete".into());
    }
    parse_version(&metadata.workerd_version)
        .ok_or_else(|| "release metadata has an invalid workerd version".to_owned())
}
fn validate(path: PathBuf, expected: &str, description: &str) -> Result<PathBuf, String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("cannot resolve {description}: {e}"))?;
    if !executable_file(&path) {
        return Err(format!(
            "{description} is not an executable file: {}",
            path.display()
        ));
    }
    let output = Command::new(&path)
        .arg("--version")
        .output()
        .map_err(|e| format!("cannot inspect {description}: {e}"))?;
    let actual = output
        .status
        .success()
        .then(|| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("workerd "))
                .and_then(|version| version.split_whitespace().next())
                .and_then(parse_version)
        })
        .flatten()
        .ok_or_else(|| format!("{description} did not report a valid workerd version"))?;
    if actual != expected {
        return Err(format!(
            "{description} version {actual} does not match required {expected}"
        ));
    }
    Ok(path)
}

fn executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}
fn parse_version(value: &str) -> Option<String> {
    let mut parts = value.split('-');
    let (Some(year), Some(month), Some(day), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    if year.len() != 4
        || month.len() != 2
        || day.len() != 2
        || ![year, month, day]
            .iter()
            .all(|part| part.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let year: u32 = year.parse().ok()?;
    let month: u32 = month.parse().ok()?;
    let day: u32 = day.parse().ok()?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    (day >= 1 && day <= days_in_month).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    fn fake_workerd(path: &Path, version: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, format!("#!/bin/sh\necho 'workerd {version}'\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[test]
    #[cfg(unix)]
    fn resolves_private_runtime_after_launcher_symlink() {
        let root = tempfile::tempdir().unwrap();
        let release = root.path().join("release");
        let bin = release.join("bin");
        let lib = release.join("libexec/paraco");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(release.join("bundle.json"), metadata("2025-01-02")).unwrap();
        std::fs::write(bin.join("paraco"), "stub").unwrap();
        fake_workerd(&lib.join("workerd"), "2025-01-02");
        let launcher = root.path().join("launcher");
        std::os::unix::fs::symlink(bin.join("paraco"), &launcher).unwrap();
        assert_eq!(
            resolve_from_executable(&launcher).unwrap(),
            lib.join("workerd").canonicalize().unwrap()
        );
    }
    #[test]
    #[cfg(unix)]
    fn rejects_wrong_runtime_version() {
        let root = tempfile::tempdir().unwrap();
        let workerd = root.path().join("workerd");
        fake_workerd(&workerd, "2025-01-01");
        assert!(validate(workerd, "2025-01-02", "test").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn source_install_in_bin_resolves_path_to_an_absolute_runtime() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let workerd = bin.join("workerd");
        fake_workerd(&workerd, "2025-01-02");
        let result = resolve_path_runtime_from(bin.as_os_str()).unwrap();
        assert_eq!(result, workerd.canonicalize().unwrap());
    }

    #[test]
    fn accepts_builder_cleanliness_marker_without_claiming_release_authenticity() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("bundle.json");
        let mut document: serde_json::Value =
            serde_json::from_str(&metadata("2024-02-29")).unwrap();
        for dirty in [false, true] {
            document["sourceDirty"] = serde_json::json!(dirty);
            std::fs::write(&path, document.to_string()).unwrap();
            assert_eq!(read_version(&path).unwrap(), "2024-02-29");
        }
        document["sourceDirty"] = serde_json::json!("false");
        std::fs::write(&path, document.to_string()).unwrap();
        assert!(read_version(&path).is_err());
        document.as_object_mut().unwrap().remove("sourceDirty");
        std::fs::write(&path, document.to_string()).unwrap();
        assert_eq!(read_version(&path).unwrap(), "2024-02-29");
    }

    fn metadata(version: &str) -> String {
        format!(
            r#"{{"paracoVersion":"0.1.0","workerdVersion":"{version}","target":"x86_64-unknown-linux-gnu","sourceRevision":"abc","sourceDirty":false,"notices":["notices/LICENSE"],"checksums":{{"bin/paraco":"x","libexec/paraco/workerd":"y","libexec/paraco/esbuild":"z"}},"trustedInputProvenance":"test"}}"#
        )
    }
}
