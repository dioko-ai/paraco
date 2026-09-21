//! Resolution of the Deno executable used by a packaged Paraco release.
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BundleMetadata {
    deno_version: String,
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
    let bundled = release.map(|root| root.join("libexec/paraco/deno"));
    let metadata = release.map(|root| root.join("bundle.json"));
    let expected = metadata
        .as_deref()
        .filter(|p| p.is_file())
        .map(read_version)
        .transpose()?;
    if let Some(override_path) = std::env::var_os("PARACO_DENO") {
        let expected = expected
            .or_else(|| std::env::var("PARACO_DENO_VERSION").ok())
            .ok_or("PARACO_DENO requires PARACO_DENO_VERSION when not running a release bundle")?;
        return validate(
            PathBuf::from(override_path),
            &expected,
            "development override",
        );
    }
    if let (Some(path), Some(expected)) = (bundled, expected) {
        return validate(path, &expected, "private bundled Deno");
    }
    if executable
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "bin")
    {
        return Err(
            "packaged Paraco is missing valid bundle.json; refusing PATH Deno fallback".into(),
        );
    }
    Ok(PathBuf::from("deno")) // source/development builds only
}

fn read_version(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read release metadata: {e}"))?;
    let metadata: BundleMetadata =
        serde_json::from_slice(&bytes).map_err(|e| format!("invalid release metadata: {e}"))?;
    parse_version(&metadata.deno_version)
        .ok_or_else(|| "release metadata has an invalid Deno version".to_owned())
}
fn validate(path: PathBuf, expected: &str, description: &str) -> Result<PathBuf, String> {
    if !path.is_file() {
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
                .and_then(|line| line.strip_prefix("deno "))
                .and_then(parse_version)
        })
        .flatten()
        .ok_or_else(|| format!("{description} did not report a valid Deno version"))?;
    if actual != expected {
        return Err(format!(
            "{description} version {actual} does not match required {expected}"
        ));
    }
    Ok(path)
}
fn parse_version(value: &str) -> Option<String> {
    let parts: Vec<_> = value.split('.').collect();
    (parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())))
    .then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    fn fake_deno(path: &Path, version: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, format!("#!/bin/sh\necho 'deno {version}'\n")).unwrap();
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
        std::fs::write(release.join("bundle.json"), r#"{"denoVersion":"1.2.3"}"#).unwrap();
        std::fs::write(bin.join("paraco"), "stub").unwrap();
        fake_deno(&lib.join("deno"), "1.2.3");
        let launcher = root.path().join("launcher");
        std::os::unix::fs::symlink(bin.join("paraco"), &launcher).unwrap();
        assert_eq!(
            resolve_from_executable(&launcher).unwrap(),
            lib.join("deno")
        );
    }
    #[test]
    #[cfg(unix)]
    fn rejects_wrong_runtime_version() {
        let root = tempfile::tempdir().unwrap();
        let deno = root.path().join("deno");
        fake_deno(&deno, "1.2.2");
        assert!(validate(deno, "1.2.3", "test").is_err());
    }
}
