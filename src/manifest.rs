use serde::Deserialize;
use std::fmt;
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub struct App {
    pub name: String,
    pub root: PathBuf,
    pub entrypoint: PathBuf,
}

#[derive(Debug)]
pub enum Error {
    AppDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    ReadManifest {
        path: PathBuf,
        source: std::io::Error,
    },
    ParseManifest {
        path: PathBuf,
        source: serde_json::Error,
    },
    InvalidName,
    InvalidEntrypoint(String),
    MissingEntrypoint {
        path: PathBuf,
        source: std::io::Error,
    },
    EntrypointOutsideApp {
        path: PathBuf,
    },
    UnsupportedCapability(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AppDirectory { path, source } => write!(
                f,
                "cannot read application directory {}: {source}",
                path.display()
            ),
            Self::ReadManifest { path, source } => {
                write!(f, "cannot read manifest {}: {source}", path.display())
            }
            Self::ParseManifest { path, source } => {
                write!(f, "invalid manifest {}: {source}", path.display())
            }
            Self::InvalidName => write!(
                f,
                "manifest field `name` must be 1-63 lowercase letters, digits, or hyphens and start with a letter"
            ),
            Self::InvalidEntrypoint(reason) => {
                write!(f, "invalid manifest field `entrypoint`: {reason}")
            }
            Self::MissingEntrypoint { path, source } => {
                write!(f, "cannot read entrypoint {}: {source}", path.display())
            }
            Self::EntrypointOutsideApp { path } => write!(
                f,
                "entrypoint {} resolves outside the application directory",
                path.display()
            ),
            Self::UnsupportedCapability(capability) => {
                write!(f, "unsupported requested capability `{capability}`")
            }
        }
    }
}

impl std::error::Error for Error {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    name: String,
    entrypoint: String,
    capabilities: Vec<String>,
}

pub fn load(app_dir: &Path) -> Result<App, Error> {
    let app_dir = app_dir
        .canonicalize()
        .map_err(|source| Error::AppDirectory {
            path: app_dir.to_path_buf(),
            source,
        })?;
    let manifest_path = app_dir.join("paraco.json");
    let contents =
        std::fs::read_to_string(&manifest_path).map_err(|source| Error::ReadManifest {
            path: manifest_path.clone(),
            source,
        })?;
    let manifest: Manifest =
        serde_json::from_str(&contents).map_err(|source| Error::ParseManifest {
            path: manifest_path,
            source,
        })?;

    if !valid_name(&manifest.name) {
        return Err(Error::InvalidName);
    }
    if let Some(capability) = manifest.capabilities.into_iter().next() {
        return Err(Error::UnsupportedCapability(capability));
    }

    let requested = Path::new(&manifest.entrypoint);
    if requested.as_os_str().is_empty() || requested.is_absolute() {
        return Err(Error::InvalidEntrypoint(
            "it must be a non-empty relative path".into(),
        ));
    }
    if requested.components().any(|part| {
        matches!(
            part,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(Error::InvalidEntrypoint(
            "`..` and absolute path components are not allowed".into(),
        ));
    }

    let entrypoint =
        app_dir
            .join(requested)
            .canonicalize()
            .map_err(|source| Error::MissingEntrypoint {
                path: app_dir.join(requested),
                source,
            })?;
    if !entrypoint.starts_with(&app_dir) {
        return Err(Error::EntrypointOutsideApp { path: entrypoint });
    }
    if !entrypoint.is_file() {
        return Err(Error::InvalidEntrypoint("it must name a file".into()));
    }

    Ok(App {
        name: manifest.name,
        root: app_dir,
        entrypoint,
    })
}

fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=63).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn app_dir(manifest: &str, entry: bool) -> TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("paraco.json"), manifest).unwrap();
        if entry {
            std::fs::write(directory.path().join("main.ts"), "export default {};").unwrap();
        }
        directory
    }

    #[test]
    fn loads_valid_manifest() {
        let directory = app_dir(
            r#"{"name":"hello","entrypoint":"main.ts","capabilities":[]}"#,
            true,
        );
        let app = load(directory.path()).unwrap();
        assert_eq!(app.name, "hello");
        assert_eq!(app.root, directory.path().canonicalize().unwrap());
        assert!(app.entrypoint.ends_with("main.ts"));
    }

    #[test]
    fn requires_all_schema_fields_and_rejects_unknown_fields() {
        let directory = app_dir(r#"{"name":"hello","entrypoint":"main.ts"}"#, true);
        assert!(matches!(
            load(directory.path()),
            Err(Error::ParseManifest { .. })
        ));
        std::fs::write(
            directory.path().join("paraco.json"),
            r#"{"name":"hello","entrypoint":"main.ts","capabilities":[],"extra":true}"#,
        )
        .unwrap();
        assert!(matches!(
            load(directory.path()),
            Err(Error::ParseManifest { .. })
        ));
    }

    #[test]
    fn rejects_bad_name_and_unsupported_capability() {
        let directory = app_dir(
            r#"{"name":"Hello","entrypoint":"main.ts","capabilities":[]}"#,
            true,
        );
        assert!(matches!(load(directory.path()), Err(Error::InvalidName)));
        std::fs::write(
            directory.path().join("paraco.json"),
            r#"{"name":"hello","entrypoint":"main.ts","capabilities":["ai"]}"#,
        )
        .unwrap();
        assert!(
            matches!(load(directory.path()), Err(Error::UnsupportedCapability(capability)) if capability == "ai")
        );
    }

    #[test]
    fn rejects_missing_and_escaping_entrypoints() {
        let directory = app_dir(
            r#"{"name":"hello","entrypoint":"missing.ts","capabilities":[]}"#,
            false,
        );
        assert!(matches!(
            load(directory.path()),
            Err(Error::MissingEntrypoint { .. })
        ));
        std::fs::write(
            directory.path().join("paraco.json"),
            r#"{"name":"hello","entrypoint":"../main.ts","capabilities":[]}"#,
        )
        .unwrap();
        assert!(matches!(
            load(directory.path()),
            Err(Error::InvalidEntrypoint(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_entrypoint_symlinked_outside_app() {
        let directory = app_dir(
            r#"{"name":"hello","entrypoint":"main.ts","capabilities":[]}"#,
            false,
        );
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("outside.ts");
        std::fs::write(&target, "export default {};").unwrap();
        std::os::unix::fs::symlink(&target, directory.path().join("main.ts")).unwrap();

        assert!(matches!(
            load(directory.path()),
            Err(Error::EntrypointOutsideApp { .. })
        ));
    }
}
