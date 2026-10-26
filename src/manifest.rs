use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub struct App {
    pub name: String,
    pub requests_ai: bool,
    pub requests_storage: bool,
    pub requests_tasks: bool,
    pub requests_system: bool,
    pub root: PathBuf,
    pub entrypoint: PathBuf,
    pub supported_targets: Vec<String>,
    pub outputs: BTreeMap<String, Output>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    pub media_type: String,
    pub schema: Option<Value>,
}

impl App {
    pub fn validate_target(&self, target: &str) -> Result<(), Error> {
        if !self
            .supported_targets
            .iter()
            .any(|supported| supported == target)
        {
            return Err(Error::UnsupportedTarget(target.into()));
        }
        if target == "cloudflare" && self.requests_system {
            return Err(Error::UnsupportedTarget(
                "cloudflare (system monitoring is local only)".into(),
            ));
        }
        if target == "cloudflare" && self.requests_tasks {
            return Err(Error::TasksUnsupportedTarget(target.into()));
        }
        Ok(())
    }
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
    DuplicateCapability(String),
    UnsupportedSchemaVersion(u32),
    InvalidTargets,
    InvalidOutput(String),
    UnsupportedTarget(String),
    TasksUnsupportedTarget(String),
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
            Self::InvalidName => write!(f, "manifest field `name` must be {}", paraco::slug::RULES),
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
            Self::DuplicateCapability(capability) => {
                write!(
                    f,
                    "manifest capability `{capability}` may be requested only once"
                )
            }
            Self::UnsupportedSchemaVersion(version) => {
                write!(f, "unsupported manifest schemaVersion `{version}`")
            }
            Self::InvalidTargets => write!(
                f,
                "manifest `targets` must be a non-empty array of unique supported targets (`local`, `cloudflare`)"
            ),
            Self::InvalidOutput(name) => write!(f, "invalid manifest output `{name}`"),
            Self::UnsupportedTarget(target) => {
                write!(f, "application does not support target `{target}`")
            }
            Self::TasksUnsupportedTarget(target) => write!(
                f,
                "the `tasks` capability is not supported on target `{target}`"
            ),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    #[serde(default)]
    schema_version: Option<u32>,
    name: String,
    entrypoint: String,
    capabilities: Vec<String>,
    #[serde(default)]
    targets: Option<Vec<String>>,
    #[serde(default)]
    outputs: BTreeMap<String, ManifestOutput>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManifestOutput {
    media_type: String,
    #[serde(default)]
    schema: Option<Value>,
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

    if let Some(version) = manifest.schema_version
        && version != 1
    {
        return Err(Error::UnsupportedSchemaVersion(version));
    }
    if paraco::slug::validate(&manifest.name).is_err() {
        return Err(Error::InvalidName);
    }
    if let Some(capability) = manifest.capabilities.iter().find(|c| {
        c.as_str() != "ai"
            && c.as_str() != "storage"
            && c.as_str() != "tasks"
            && c.as_str() != "system"
    }) {
        return Err(Error::UnsupportedCapability(capability.clone()));
    }
    for capability in ["ai", "storage", "tasks", "system"] {
        if manifest
            .capabilities
            .iter()
            .filter(|c| c.as_str() == capability)
            .count()
            > 1
        {
            return Err(Error::DuplicateCapability(capability.into()));
        }
    }

    let supported_targets = match manifest.targets {
        Some(targets)
            if !targets.is_empty()
                && targets.iter().all(|t| t == "local" || t == "cloudflare")
                && targets
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    == targets.len() =>
        {
            targets
        }
        Some(_) => return Err(Error::InvalidTargets),
        None if manifest
            .capabilities
            .iter()
            .any(|c| c == "tasks" || c == "system") =>
        {
            vec!["local".into()]
        }
        None => vec!["local".into(), "cloudflare".into()],
    };
    let mut outputs = BTreeMap::new();
    for (name, output) in manifest.outputs {
        if paraco::slug::validate(&name).is_err() || output.media_type.trim().is_empty() {
            return Err(Error::InvalidOutput(name));
        }
        outputs.insert(
            name,
            Output {
                media_type: output.media_type,
                schema: output.schema,
            },
        );
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
        requests_ai: manifest.capabilities.iter().any(|c| c == "ai"),
        requests_storage: manifest.capabilities.iter().any(|c| c == "storage"),
        requests_tasks: manifest.capabilities.iter().any(|c| c == "tasks"),
        requests_system: manifest.capabilities.iter().any(|c| c == "system"),
        name: manifest.name,
        root: app_dir,
        entrypoint,
        supported_targets,
        outputs,
    })
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
        assert_eq!(app.supported_targets, ["local", "cloudflare"]);
        assert!(app.validate_target("local").is_ok());
    }

    #[test]
    fn system_monitoring_is_local_only() {
        let directory = app_dir(
            r#"{"name":"vitals","entrypoint":"main.ts","capabilities":["system"]}"#,
            true,
        );
        let app = load(directory.path()).unwrap();
        assert!(app.requests_system);
        assert_eq!(app.supported_targets, ["local"]);
        assert!(app.validate_target("cloudflare").is_err());
        std::fs::write(
            directory.path().join("paraco.json"),
            r#"{"name":"vitals","entrypoint":"main.ts","capabilities":["system","system"]}"#,
        )
        .unwrap();
        assert!(matches!(
            load(directory.path()),
            Err(Error::DuplicateCapability(_))
        ));
    }

    #[test]
    fn defaults_tasks_to_local_and_checks_explicit_targets() {
        let directory = app_dir(
            r#"{"name":"task-app","entrypoint":"main.ts","capabilities":["tasks"]}"#,
            true,
        );
        let app = load(directory.path()).unwrap();
        assert_eq!(app.supported_targets, ["local"]);
        assert!(matches!(
            app.validate_target("cloudflare"),
            Err(Error::UnsupportedTarget(_))
        ));

        let directory = app_dir(
            r#"{"name":"task-app","entrypoint":"main.ts","capabilities":["tasks"],"targets":["local","cloudflare"]}"#,
            true,
        );
        let app = load(directory.path()).unwrap();
        assert!(matches!(
            app.validate_target("cloudflare"),
            Err(Error::TasksUnsupportedTarget(_))
        ));
    }

    #[test]
    fn validates_targets_and_loads_output_contracts() {
        let directory = app_dir(
            r#"{"name":"hello","entrypoint":"main.ts","capabilities":[],"targets":["cloudflare"],"outputs":{"report":{"mediaType":"application/json","schema":{"type":"object"}}}}"#,
            true,
        );
        let app = load(directory.path()).unwrap();
        assert!(matches!(
            app.validate_target("local"),
            Err(Error::UnsupportedTarget(_))
        ));
        assert_eq!(app.outputs["report"].media_type, "application/json");
        assert_eq!(
            app.outputs["report"].schema,
            Some(serde_json::json!({"type":"object"}))
        );

        for targets in [r#"[]"#, r#"["local","local"]"#, r#"["remote"]"#] {
            std::fs::write(
                directory.path().join("paraco.json"),
                format!(r#"{{"name":"hello","entrypoint":"main.ts","capabilities":[],"targets":{targets}}}"#),
            ).unwrap();
            assert!(matches!(load(directory.path()), Err(Error::InvalidTargets)));
        }
        std::fs::write(
            directory.path().join("paraco.json"),
            r#"{"name":"hello","entrypoint":"main.ts","capabilities":[],"outputs":{"bad name":{"mediaType":"application/json"}}}"#,
        ).unwrap();
        assert!(matches!(
            load(directory.path()),
            Err(Error::InvalidOutput(_))
        ));
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
            r#"{"name":"hello","entrypoint":"main.ts","capabilities":["unknown"]}"#,
        )
        .unwrap();
        assert!(
            matches!(load(directory.path()), Err(Error::UnsupportedCapability(capability)) if capability == "unknown")
        );
        std::fs::write(
            directory.path().join("paraco.json"),
            r#"{"schemaVersion":2,"name":"hello","entrypoint":"main.ts","capabilities":[]}"#,
        )
        .unwrap();
        assert!(matches!(
            load(directory.path()),
            Err(Error::UnsupportedSchemaVersion(2))
        ));
        std::fs::write(
            directory.path().join("paraco.json"),
            r#"{"schemaVersion":1,"name":"hello","entrypoint":"main.ts","capabilities":["ai","ai"]}"#,
        )
        .unwrap();
        assert!(matches!(
            load(directory.path()),
            Err(Error::DuplicateCapability(capability)) if capability == "ai"
        ));
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
