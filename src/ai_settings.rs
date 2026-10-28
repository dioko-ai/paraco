//! Device defaults and installation overrides. Only the host reads this store.
use crate::ai::{AppPolicy, Config, ProviderKind, Route, Selection};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

static ROOT: OnceLock<PathBuf> = OnceLock::new();
static WRITER: Mutex<()> = Mutex::new(());
pub fn initialize(data: &Path) {
    let _ = ROOT.set(data.join("ai"));
}
pub fn root() -> Option<&'static Path> {
    ROOT.get().map(PathBuf::as_path)
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub transport: ProviderKind,
    pub credential_file: Option<PathBuf>,
    pub endpoint: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Override {
    Disabled,
    Custom { profile: Profile },
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub default: Option<Profile>,
    #[serde(default)]
    pub apps: BTreeMap<String, Override>,
}
impl Settings {
    pub fn selected(&self, deployment: &str) -> Option<&Profile> {
        match self.apps.get(deployment) {
            Some(Override::Disabled) => None,
            Some(Override::Custom { profile }) => Some(profile),
            None => self.default.as_ref(),
        }
    }
}

pub fn read(root: &Path) -> Result<Settings, String> {
    match std::fs::read(root.join("settings.json")) {
        Ok(bytes) if bytes.len() <= 1024 * 1024 => {
            serde_json::from_slice(&bytes).map_err(|_| "Invalid AI settings".into())
        }
        Ok(_) => Err("AI settings are too large".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(_) => Err("Cannot read AI settings".into()),
    }
}

impl Profile {
    pub fn endpoint(&self) -> Result<String, String> {
        let preset = match self.provider.as_str() {
            "openai" => "https://api.openai.com/v1/chat/completions",
            "anthropic" => "https://api.anthropic.com/v1/messages",
            "xai" => "https://api.x.ai/v1/chat/completions",
            _ => "",
        };
        let value = self
            .endpoint
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(preset);
        let url = url::Url::parse(value).map_err(|_| "Set an HTTPS completion endpoint")?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err("AI endpoint must be HTTPS without credentials or fragment".into());
        }
        Ok(value.into())
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.provider.trim().is_empty()
            || self.model.trim().is_empty()
            || self.model.len() > 200
            || self.provider.len() > 100
        {
            return Err("Choose a provider and enter a model".into());
        }
        match &self.transport {
            ProviderKind::CodexSdk { node, module } | ProviderKind::ClaudeSdk { node, module } => {
                if !matches!(
                    (&self.transport, self.provider.as_str()),
                    (ProviderKind::CodexSdk { .. }, "openai")
                        | (ProviderKind::ClaudeSdk { .. }, "anthropic")
                ) {
                    return Err("Choose the matching provider for this SDK".into());
                }
                crate::ai_sdk::validate_path(node)?;
                crate::ai_sdk::validate_path(module)?;
            }
            _ => {
                if (self.provider == "anthropic")
                    != matches!(self.transport, ProviderKind::Anthropic)
                {
                    return Err("Claude API requires the Anthropic transport".into());
                }
                self.endpoint()?;
                if !self
                    .credential_file
                    .as_ref()
                    .is_some_and(|p| p.is_absolute())
                {
                    return Err("An API key is required".into());
                }
            }
        }
        Ok(())
    }
    pub fn config(&self, deployment: &str, app_root: &Path) -> Result<Config, String> {
        self.validate()?;
        let mut config = Config::default();
        let selection = Selection {
            provider: self.provider.clone(),
            model: self.model.clone(),
        };
        config
            .credentials
            .insert("selected".into(), self.provider.clone());
        config.routes.push(Route {
            selection: selection.clone(),
            credential: "selected".into(),
        });
        config.apps.insert(
            deployment.into(),
            AppPolicy {
                requests_ai: true,
                credential_grants: ["selected".into()].into(),
                default: Some(selection),
            },
        );
        config
            .provider_default_models
            .insert(self.provider.clone(), self.model.clone());
        config
            .provider_kinds
            .insert(self.provider.clone(), self.transport.clone());
        match &self.transport {
            ProviderKind::CodexSdk { node, module } | ProviderKind::ClaudeSdk { node, module } => {
                let node = outside(node, app_root)?;
                let module = outside(module, app_root)?;
                config.provider_kinds.insert(
                    self.provider.clone(),
                    if matches!(self.transport, ProviderKind::CodexSdk { .. }) {
                        ProviderKind::CodexSdk { node, module }
                    } else {
                        ProviderKind::ClaudeSdk { node, module }
                    },
                );
                config.limits.timeout = std::time::Duration::from_secs(120);
            }
            _ => {
                config
                    .provider_endpoints
                    .insert(self.provider.clone(), self.endpoint()?);
                let path = outside(
                    self.credential_file.as_ref().ok_or("Missing AI key")?,
                    app_root,
                )?;
                config.credential_files.insert("selected".into(), path);
            }
        }
        Ok(config)
    }
}
fn outside(path: &Path, app_root: &Path) -> Result<PathBuf, String> {
    let path = path
        .canonicalize()
        .map_err(|_| "Cannot read host AI file")?;
    let app_root = app_root
        .canonicalize()
        .map_err(|_| "Cannot read application directory")?;
    if path.starts_with(app_root) {
        return Err("AI keys and SDK files must be outside the application directory".into());
    }
    Ok(path)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    pub app: Option<String>,
    pub mode: String,
    pub profile: Option<Profile>,
    pub api_key: Option<String>,
}

pub fn update(root: &Path, deployment: Option<&str>, mut update: Update) -> Result<(), String> {
    let _lock = WRITER.lock().map_err(|_| "AI settings unavailable")?;
    std::fs::create_dir_all(root).map_err(|_| "Cannot create AI settings directory")?;
    let mut lock_options = std::fs::OpenOptions::new();
    lock_options.create(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        lock_options.mode(0o600);
    }
    let lock = lock_options
        .open(root.join(".lock"))
        .map_err(|_| "Cannot lock AI settings")?;
    lock.lock().map_err(|_| "Cannot lock AI settings")?;
    let mut settings = read(root)?;
    let old = match deployment {
        Some(id) => match settings.apps.get(id) {
            Some(Override::Custom { profile }) => Some(profile),
            _ => None,
        },
        None => settings.default.as_ref(),
    }
    .cloned();
    std::fs::create_dir_all(root).map_err(|_| "Cannot create AI settings directory")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot protect AI settings")?;
    }
    let mut new_key = None;
    let result = (|| {
        let selected = match update.mode.as_str() {
            "inherit" if deployment.is_some() => None,
            "disabled" => Some(Override::Disabled),
            "custom" => {
                let mut profile = update.profile.take().ok_or("Choose an AI provider")?;
                // Browser-supplied paths are not a way to read arbitrary host secrets.
                profile.credential_file = old
                    .as_ref()
                    .filter(|p| p.provider == profile.provider && p.endpoint == profile.endpoint)
                    .and_then(|p| p.credential_file.clone());
                if matches!(
                    profile.transport,
                    ProviderKind::Openai | ProviderKind::Anthropic
                ) {
                    if let Some(key) = update.api_key.as_deref().filter(|k| !k.is_empty()) {
                        if key.trim().is_empty() || key.len() > 4096 || key.contains(['\r', '\n']) {
                            return Err("Invalid API key".into());
                        }
                        let file = tempfile::Builder::new()
                            .prefix("key-")
                            .tempfile_in(root)
                            .map_err(|_| "Cannot save AI key")?;
                        std::fs::write(file.path(), key.trim())
                            .map_err(|_| "Cannot save AI key")?;
                        let (_, path) = file.keep().map_err(|_| "Cannot save AI key")?;
                        new_key = Some(path.clone());
                        profile.credential_file = Some(path);
                    }
                } else {
                    profile.credential_file = None;
                }
                profile.validate()?;
                Some(Override::Custom { profile })
            }
            _ => return Err("Invalid AI settings mode".into()),
        };
        if let Some(id) = deployment {
            if let Some(selected) = selected {
                settings.apps.insert(id.into(), selected);
            } else {
                settings.apps.remove(id);
            }
        } else {
            settings.default = match selected {
                Some(Override::Custom { profile }) => Some(profile),
                _ => None,
            };
        }
        let bytes = serde_json::to_vec_pretty(&settings).map_err(|_| "Cannot save AI settings")?;
        let file = tempfile::NamedTempFile::new_in(root).map_err(|_| "Cannot save AI settings")?;
        std::fs::write(file.path(), bytes).map_err(|_| "Cannot save AI settings")?;
        file.persist(root.join("settings.json"))
            .map_err(|_| "Cannot save AI settings")?;
        Ok(())
    })();
    if result.is_err() {
        if let Some(path) = new_key {
            let _ = std::fs::remove_file(path);
        }
    } else if let Some(path) = old.and_then(|p| p.credential_file) {
        let retained = settings
            .default
            .iter()
            .chain(settings.apps.values().filter_map(|v| match v {
                Override::Custom { profile } => Some(profile),
                _ => None,
            }))
            .any(|p| p.credential_file.as_ref() == Some(&path));
        if !retained
            && path.parent() == Some(root)
            && path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("key-"))
        {
            let _ = std::fs::remove_file(path);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile(provider: &str) -> Profile {
        Profile {
            provider: provider.into(),
            model: "test-model".into(),
            transport: if provider == "anthropic" {
                ProviderKind::Anthropic
            } else {
                ProviderKind::Openai
            },
            credential_file: None,
            endpoint: None,
        }
    }
    fn change(provider: &str, key: Option<&str>) -> Update {
        Update {
            app: None,
            mode: "custom".into(),
            profile: Some(profile(provider)),
            api_key: key.map(str::to_owned),
        }
    }
    fn mode(value: &str) -> Update {
        Update {
            app: None,
            mode: value.into(),
            profile: None,
            api_key: None,
        }
    }
    #[test]
    fn persisted_global_override_disable_and_inherit() {
        let dir = tempfile::tempdir().unwrap();
        update(dir.path(), None, change("openai", Some("global-secret"))).unwrap();
        update(
            dir.path(),
            Some("app-a"),
            change("anthropic", Some("app-secret")),
        )
        .unwrap();
        let settings = read(dir.path()).unwrap();
        assert_eq!(settings.selected("app-a").unwrap().provider, "anthropic");
        assert_eq!(settings.selected("app-b").unwrap().provider, "openai");
        assert!(
            !std::fs::read_to_string(dir.path().join("settings.json"))
                .unwrap()
                .contains("global-secret")
        );
        update(dir.path(), Some("app-a"), mode("disabled")).unwrap();
        assert!(read(dir.path()).unwrap().selected("app-a").is_none());
        update(dir.path(), Some("app-a"), mode("inherit")).unwrap();
        assert_eq!(
            read(dir.path())
                .unwrap()
                .selected("app-a")
                .unwrap()
                .provider,
            "openai"
        );
        update(dir.path(), None, mode("disabled")).unwrap();
        assert!(read(dir.path()).unwrap().selected("app-b").is_none());
    }
    #[test]
    fn keeps_key_but_requires_new_key_when_provider_or_endpoint_changes() {
        let dir = tempfile::tempdir().unwrap();
        update(dir.path(), None, change("openai", Some("saved-key"))).unwrap();
        let old = read(dir.path())
            .unwrap()
            .default
            .unwrap()
            .credential_file
            .unwrap();
        update(dir.path(), None, change("openai", None)).unwrap();
        assert_eq!(
            read(dir.path())
                .unwrap()
                .default
                .unwrap()
                .credential_file
                .unwrap(),
            old
        );
        assert!(update(dir.path(), None, change("xai", None)).is_err());
        let mut other = change("openai", None);
        other.profile.as_mut().unwrap().endpoint = Some("https://other.invalid/completions".into());
        assert!(update(dir.path(), None, other).is_err());
        assert_eq!(
            read(dir.path()).unwrap().default.unwrap().provider,
            "openai"
        );
        update(dir.path(), None, change("xai", Some("replacement"))).unwrap();
        assert!(!old.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let key = read(dir.path())
                .unwrap()
                .default
                .unwrap()
                .credential_file
                .unwrap();
            assert_eq!(
                std::fs::metadata(key).unwrap().permissions().mode() & 0o077,
                0
            );
        }
    }
    #[test]
    fn invalid_save_rolls_back_and_does_not_leave_key_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut invalid = change("openai", Some("secret"));
        invalid.profile.as_mut().unwrap().model.clear();
        assert!(update(dir.path(), None, invalid).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(read(dir.path()).unwrap().default.is_none());
    }
    #[test]
    fn configured_app_cannot_select_another_profile_or_access_its_key() {
        let dir = tempfile::tempdir().unwrap();
        update(dir.path(), None, change("openai", Some("key"))).unwrap();
        let settings = read(dir.path()).unwrap();
        let app = dir.path().join("app");
        std::fs::create_dir(&app).unwrap();
        let config = settings
            .selected("app-a")
            .unwrap()
            .config("app-a", &app)
            .unwrap();
        let proxy = crate::ai::FakeProxy::new(config).unwrap();
        let request = crate::ai::Request::default();
        assert!(proxy.complete("app-b", &request).is_err());
        assert!(
            proxy
                .complete(
                    "app-a",
                    &crate::ai::Request {
                        provider: Some("xai".into()),
                        ..request
                    }
                )
                .is_err()
        );
        assert!(
            settings
                .selected("app-a")
                .unwrap()
                .config("app-a", dir.path())
                .is_err()
        );
    }
    #[cfg(unix)]
    #[test]
    fn rejects_host_files_inside_symlinked_application_directory() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("app");
        std::fs::create_dir(&app).unwrap();
        let key = app.join("key");
        std::fs::write(&key, "secret").unwrap();
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(&app, &alias).unwrap();
        assert!(outside(&key, &alias).is_err());
        assert!(outside(&alias.join("key"), &app).is_err());
        let host_key = dir.path().join("host-key");
        std::fs::write(&host_key, "secret").unwrap();
        assert_eq!(
            outside(&host_key, &alias).unwrap(),
            host_key.canonicalize().unwrap()
        );
        let host_alias = dir.path().join("host-alias");
        std::os::unix::fs::symlink(&host_key, &host_alias).unwrap();
        assert_eq!(
            outside(&host_alias, &alias).unwrap(),
            host_key.canonicalize().unwrap()
        );
        assert!(outside(&host_key, &dir.path().join("missing-app")).is_err());
    }
}
