//! Offline AI policy core. Configuration and caller identity are supplied by the
//! trusted host, never by the application's request payload.

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::OnceLock;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Strict incremental decoder for the small OpenAI SSE subset we can safely
/// forward. It deliberately accepts bytes (rather than UTF-8 fragments), so a
/// UTF-8 character split across transport chunks is not corrupted. Callers
/// must treat EOF before `Done` as failure.
#[derive(Default)]
struct SseDecoder {
    pending: Vec<u8>,
    emitted: usize,
    finished: bool,
}

enum SseEvent {
    Text(String),
    Done,
}

impl SseDecoder {
    fn push(&mut self, bytes: &[u8], maximum: usize) -> Result<Vec<SseEvent>, Error> {
        if self.finished || self.pending.len().saturating_add(bytes.len()) > maximum {
            return Err(Error::ProviderFailed);
        }
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = self.pending.windows(2).position(|v| v == b"\n\n") {
            let record: Vec<u8> = self.pending.drain(..end + 2).collect();
            let record = std::str::from_utf8(&record[..end]).map_err(|_| Error::ProviderFailed)?;
            let mut data = None;
            for line in record.lines() {
                if let Some(value) = line.strip_prefix("data:") {
                    if data.replace(value.trim_start()).is_some() {
                        return Err(Error::ProviderFailed);
                    }
                } else if !line.is_empty() && !line.starts_with(':') {
                    return Err(Error::ProviderFailed);
                }
            }
            let data = data.ok_or(Error::ProviderFailed)?;
            if data == "[DONE]" {
                self.finished = true;
                events.push(SseEvent::Done);
                continue;
            }
            let value: serde_json::Value =
                serde_json::from_str(data).map_err(|_| Error::ProviderFailed)?;
            // OpenAI sends a final empty-delta record carrying finish_reason
            // before [DONE]. It is structural terminal metadata, not output.
            let Some(text) = value
                .pointer("/choices/0/delta/content")
                .and_then(serde_json::Value::as_str)
            else {
                if value
                    .pointer("/choices/0/finish_reason")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
                {
                    continue;
                }
                return Err(Error::ProviderFailed);
            };
            self.emitted = self.emitted.saturating_add(text.len());
            if self.emitted > maximum {
                return Err(Error::ResponseTooLarge);
            }
            events.push(SseEvent::Text(text.to_owned()));
        }
        Ok(events)
    }
    fn finish(self) -> Result<(), Error> {
        if self.finished && self.pending.is_empty() {
            Ok(())
        } else {
            Err(Error::ProviderFailed)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub provider: String,
    pub model: String,
}

/// A configured route references a host-owned credential, not a credential value.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub selection: Selection,
    pub credential: String,
}

#[derive(Clone, Debug, Default)]
pub struct AppPolicy {
    pub requests_ai: bool,
    pub credential_grants: BTreeSet<String>,
    pub default: Option<Selection>,
}

/// Host-owned configuration. Credential values are deliberately absent: only
/// paths to separately stored secret files may be configured here.
#[derive(Clone, Default)]
pub struct Config {
    /// Credential identity -> owning provider.
    pub credentials: BTreeMap<String, String>,
    /// Credential identity -> host-owned file containing its bearer secret.
    pub credential_files: BTreeMap<String, std::path::PathBuf>,
    /// Provider name -> OpenAI-compatible HTTPS endpoint.
    pub provider_endpoints: BTreeMap<String, String>,
    /// Configuration order breaks ties between credentials for the same pair.
    pub routes: Vec<Route>,
    pub apps: BTreeMap<String, AppPolicy>,
    pub default: Option<Selection>,
    pub provider_default_models: BTreeMap<String, String>,
    pub limits: Limits,
}

/// Resource limits are host policy, never supplied by an application request.
#[derive(Clone, Debug)]
pub struct Limits {
    pub global_concurrency: usize,
    pub per_deployment_concurrency: usize,
    pub queued_requests: usize,
    pub timeout: std::time::Duration,
    pub response_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            global_concurrency: 8,
            per_deployment_concurrency: 2,
            queued_requests: 16,
            timeout: std::time::Duration::from_secs(30),
            response_bytes: 1024 * 1024,
        }
    }
}

/// Only selection and prompt are app-controlled. There is no caller or credential
/// field. Prompt content deliberately has no derived Debug implementation.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub prompt: String,
}

#[derive(PartialEq, Eq, Serialize)]
pub struct Response {
    pub selection: Selection,
    pub text: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidConfiguration,
    UnknownCaller,
    CapabilityNotRequested,
    InvalidSelection,
    NoRoute,
    CredentialNotGranted,
    NoDefault,
    NoProviderDefault,
    AmbiguousModel,
    InvalidProvider,
    CredentialUnavailable,
    Busy,
    ProviderFailed,
    ResponseTooLarge,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidConfiguration => "invalid AI routing configuration",
            Self::UnknownCaller => "unknown AI caller",
            Self::CapabilityNotRequested => "application did not request the AI capability",
            Self::InvalidSelection => "provider and model must be non-empty when specified",
            Self::NoRoute => "no configured AI route matches the selection",
            Self::CredentialNotGranted => "no credential grant permits this AI selection",
            Self::NoDefault => "no permitted application or runtime AI default is configured",
            Self::NoProviderDefault => "no default model is configured for the selected provider",
            Self::AmbiguousModel => {
                "model matches multiple permitted providers; specify a provider"
            }
            Self::InvalidProvider => "invalid AI provider configuration",
            Self::CredentialUnavailable => "AI credential is unavailable",
            Self::Busy => "AI capacity is currently unavailable",
            Self::ProviderFailed => "AI provider request failed",
            Self::ResponseTooLarge => "AI provider response exceeded the configured limit",
        })
    }
}

impl std::error::Error for Error {}

/// The fake backend is intentional: this type never makes network requests.
/// A transport must authenticate callers before supplying `caller` to complete.
pub struct FakeProxy {
    config: Config,
}

impl FakeProxy {
    pub fn new(config: Config) -> Result<Self, Error> {
        let valid_selection =
            |s: &Selection| !s.provider.trim().is_empty() && !s.model.trim().is_empty();
        if config.limits.global_concurrency == 0
            || config.limits.per_deployment_concurrency == 0
            || config.limits.timeout.is_zero()
            || config.limits.response_bytes == 0
            || config
                .credentials
                .iter()
                .any(|(id, provider)| id.trim().is_empty() || provider.trim().is_empty())
            || config.routes.iter().any(|route| {
                !valid_selection(&route.selection)
                    || config.credentials.get(&route.credential) != Some(&route.selection.provider)
            })
            || config.apps.iter().any(|(id, policy)| {
                id.trim().is_empty()
                    || policy
                        .credential_grants
                        .iter()
                        .any(|id| !config.credentials.contains_key(id))
                    || policy.default.as_ref().is_some_and(|s| !valid_selection(s))
            })
            || config.default.as_ref().is_some_and(|s| !valid_selection(s))
            || config
                .provider_default_models
                .iter()
                .any(|(provider, model)| provider.trim().is_empty() || model.trim().is_empty())
        {
            return Err(Error::InvalidConfiguration);
        }
        Ok(Self { config })
    }

    pub fn complete(&self, caller: &str, request: &Request) -> Result<Response, Error> {
        let policy = self.config.apps.get(caller).ok_or(Error::UnknownCaller)?;
        if !policy.requests_ai {
            return Err(Error::CapabilityNotRequested);
        }
        if request
            .provider
            .as_ref()
            .is_some_and(|s| s.trim().is_empty())
            || request.model.as_ref().is_some_and(|s| s.trim().is_empty())
        {
            return Err(Error::InvalidSelection);
        }
        let route = match (&request.provider, &request.model) {
            (Some(provider), Some(model)) => self.resolve(
                policy,
                &Selection {
                    provider: provider.clone(),
                    model: model.clone(),
                },
            )?,
            (Some(provider), None) => {
                let model = self
                    .config
                    .provider_default_models
                    .get(provider)
                    .ok_or(Error::NoProviderDefault)?;
                self.resolve(
                    policy,
                    &Selection {
                        provider: provider.clone(),
                        model: model.clone(),
                    },
                )?
            }
            (None, Some(model)) => {
                let candidates: BTreeSet<_> = self
                    .config
                    .routes
                    .iter()
                    .filter(|route| {
                        &route.selection.model == model
                            && policy.credential_grants.contains(&route.credential)
                    })
                    .map(|route| &route.selection)
                    .collect();
                match candidates.len() {
                    0 => {
                        return Err(
                            if self
                                .config
                                .routes
                                .iter()
                                .any(|r| &r.selection.model == model)
                            {
                                Error::CredentialNotGranted
                            } else {
                                Error::NoRoute
                            },
                        );
                    }
                    1 => self.resolve(policy, candidates.into_iter().next().unwrap())?,
                    _ => return Err(Error::AmbiguousModel),
                }
            }
            (None, None) => policy
                .default
                .iter()
                .chain(self.config.default.iter())
                .find_map(|selection| self.resolve(policy, selection).ok())
                .ok_or(Error::NoDefault)?,
        };
        Ok(fake_complete(route, request))
    }

    fn resolve(&self, policy: &AppPolicy, selection: &Selection) -> Result<&Route, Error> {
        let mut matches = self
            .config
            .routes
            .iter()
            .filter(|route| &route.selection == selection)
            .peekable();
        if matches.peek().is_none() {
            return Err(Error::NoRoute);
        }
        matches
            .find(|route| policy.credential_grants.contains(&route.credential))
            .ok_or(Error::CredentialNotGranted)
    }
}

/// A real, narrowly scoped OpenAI-compatible provider. It shares FakeProxy's
/// authorization path, so a rejected caller cannot cause credential reads or
/// network traffic. Completion and normalized SSE streaming share its limits.
pub struct OpenAiProxy {
    policy: FakeProxy,
    config: Config,
    client: reqwest::Client,
    global: Arc<Semaphore>,
    deployments: Mutex<BTreeMap<String, Arc<Semaphore>>>,
    queued: AtomicUsize,
}

// This is an explicit host operating bound, shared even when an administrator
// uses more than one AI configuration file. Per-service `Limits` can narrow it
// further but cannot mint a second host-wide provider budget.
static HOST_PROVIDER_SEMAPHORE: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn host_provider_semaphore() -> Arc<Semaphore> {
    HOST_PROVIDER_SEMAPHORE
        .get_or_init(|| Arc::new(Semaphore::new(Limits::default().global_concurrency)))
        .clone()
}

impl OpenAiProxy {
    pub fn new(config: Config) -> Result<Self, Error> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| Error::InvalidProvider)?;
        Self::with_client(config, client)
    }

    /// Construct the provider with a host-supplied HTTP client. Application
    /// configuration never controls the client, proxy policy, or trust roots.
    pub fn with_client(config: Config, client: reqwest::Client) -> Result<Self, Error> {
        for (provider, endpoint) in &config.provider_endpoints {
            if provider.trim().is_empty() || !valid_endpoint(endpoint) {
                return Err(Error::InvalidProvider);
            }
        }
        for (credential, path) in &config.credential_files {
            if !config.credentials.contains_key(credential) || !path.is_absolute() {
                return Err(Error::InvalidConfiguration);
            }
        }
        let policy = FakeProxy::new(config.clone())?;
        Ok(Self {
            global: Arc::new(Semaphore::new(config.limits.global_concurrency)),
            policy,
            config,
            client,
            deployments: Mutex::new(BTreeMap::new()),
            queued: AtomicUsize::new(0),
        })
    }

    /// The caller is a host-assigned deployment ID, not an app-supplied name.
    pub async fn complete(&self, deployment: &str, request: &Request) -> Result<Response, Error> {
        let deadline = std::time::Instant::now() + self.config.limits.timeout;
        // Authorization is deliberately first; this preserves the no-network
        // guarantee for denied routes.
        let selected = self.policy.complete(deployment, request)?;
        let route =
            self.config
                .routes
                .iter()
                .find(|route| {
                    route.selection == selected.selection
                        && self.config.apps.get(deployment).is_some_and(|policy| {
                            policy.credential_grants.contains(&route.credential)
                        })
                })
                .ok_or(Error::NoRoute)?;
        let endpoint = self
            .config
            .provider_endpoints
            .get(&route.selection.provider)
            .ok_or(Error::InvalidProvider)?;
        let secret_path = self
            .config
            .credential_files
            .get(&route.credential)
            .ok_or(Error::CredentialUnavailable)?;
        let secret = read_secret(secret_path)?;
        let _queue = QueueSlot::acquire(&self.queued, self.config.limits.queued_requests)?;
        let host_global = acquire(host_provider_semaphore(), deadline).await?;
        let global = acquire(self.global.clone(), deadline).await?;
        let deployment_semaphore = {
            let mut deployments = self.deployments.lock().map_err(|_| Error::Busy)?;
            deployments
                .entry(deployment.to_owned())
                .or_insert_with(|| {
                    Arc::new(Semaphore::new(
                        self.config.limits.per_deployment_concurrency,
                    ))
                })
                .clone()
        };
        let per_deployment = acquire(deployment_semaphore, deadline).await?;
        let text = self
            .request(
                endpoint,
                &route.selection.model,
                &request.prompt,
                &secret,
                (host_global, global, per_deployment),
                deadline,
            )
            .await?;
        Ok(Response {
            selection: selected.selection,
            text,
        })
    }

    /// Forward only normalized text deltas. The sink is called as each SSE
    /// record arrives; a sink error (including a disconnected client) aborts
    /// the upstream future and drops its admission permits.
    pub async fn stream<F>(
        &self,
        deployment: &str,
        request: &Request,
        mut sink: F,
    ) -> Result<Selection, Error>
    where
        F: FnMut(&str) -> Result<(), Error>,
    {
        let deadline = std::time::Instant::now() + self.config.limits.timeout;
        let selected = self.policy.complete(deployment, request)?;
        let route =
            self.config
                .routes
                .iter()
                .find(|route| {
                    route.selection == selected.selection
                        && self.config.apps.get(deployment).is_some_and(|policy| {
                            policy.credential_grants.contains(&route.credential)
                        })
                })
                .ok_or(Error::NoRoute)?;
        let endpoint = self
            .config
            .provider_endpoints
            .get(&route.selection.provider)
            .ok_or(Error::InvalidProvider)?;
        let secret = read_secret(
            self.config
                .credential_files
                .get(&route.credential)
                .ok_or(Error::CredentialUnavailable)?,
        )?;
        let _queue = QueueSlot::acquire(&self.queued, self.config.limits.queued_requests)?;
        let _host_global = acquire(host_provider_semaphore(), deadline).await?;
        let _global = acquire(self.global.clone(), deadline).await?;
        let deployment_semaphore = self
            .deployments
            .lock()
            .map_err(|_| Error::Busy)?
            .entry(deployment.to_owned())
            .or_insert_with(|| {
                Arc::new(Semaphore::new(
                    self.config.limits.per_deployment_concurrency,
                ))
            })
            .clone();
        let _deployment = acquire(deployment_semaphore, deadline).await?;
        let maximum = self.config.limits.response_bytes;
        tokio::time::timeout(remaining(deadline)?, async {
            let response = self.client.post(endpoint).bearer_auth(secret).json(&serde_json::json!({
                "model": route.selection.model, "messages": [{"role":"user", "content": request.prompt}], "stream": true
            })).send().await.map_err(|_| Error::ProviderFailed)?;
            if !response.status().is_success() { return Err(Error::ProviderFailed); }
            let mut decoder = SseDecoder::default();
            let mut body = response.bytes_stream();
            while let Some(chunk) = body.next().await {
                for event in decoder.push(&chunk.map_err(|_| Error::ProviderFailed)?, maximum)? {
                    match event { SseEvent::Text(text) => sink(&text)?, SseEvent::Done => {} }
                }
            }
            decoder.finish()
        }).await.map_err(|_| Error::ProviderFailed)??;
        Ok(selected.selection)
    }

    async fn request(
        &self,
        endpoint: &str,
        model: &str,
        prompt: &str,
        secret: &str,
        _permits: (
            OwnedSemaphorePermit,
            OwnedSemaphorePermit,
            OwnedSemaphorePermit,
        ),
        deadline: std::time::Instant,
    ) -> Result<String, Error> {
        let body = tokio::time::timeout(remaining(deadline)?, async {
            let response = self
                .client
                .post(endpoint)
                .bearer_auth(secret)
                .json(&serde_json::json!({
                    "model": model,
                    "messages": [{"role": "user", "content": prompt}],
                    "stream": false
                }))
                .send()
                .await
                .map_err(|_| Error::ProviderFailed)?;
            if !response.status().is_success() {
                return Err(Error::ProviderFailed);
            }
            let mut body = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| Error::ProviderFailed)?;
                if body.len().saturating_add(chunk.len()) > self.config.limits.response_bytes {
                    return Err(Error::ResponseTooLarge);
                }
                body.extend_from_slice(&chunk);
            }
            Ok::<_, Error>(body)
        })
        .await
        .map_err(|_| Error::ProviderFailed)?
        .map_err(|_| Error::ProviderFailed)?;
        let body: serde_json::Value =
            serde_json::from_slice(&body).map_err(|_| Error::ProviderFailed)?;
        body.pointer("/choices/0/message/content")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or(Error::ProviderFailed)
    }
}

// Kept separate so endpoint validation is unit-testable without any network.
fn valid_endpoint(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
}

fn read_secret(path: &std::path::Path) -> Result<String, Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(path).map_err(|_| Error::CredentialUnavailable)?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(Error::CredentialUnavailable);
        }
    }
    let value = std::fs::read_to_string(path).map_err(|_| Error::CredentialUnavailable)?;
    let value = value.trim().to_owned();
    if value.is_empty() || value.len() > 4096 || value.contains(['\r', '\n']) {
        return Err(Error::CredentialUnavailable);
    }
    Ok(value)
}

struct QueueSlot<'a>(&'a AtomicUsize);
impl<'a> QueueSlot<'a> {
    fn acquire(queued: &'a AtomicUsize, maximum: usize) -> Result<Self, Error> {
        loop {
            let current = queued.load(Ordering::Acquire);
            if current >= maximum {
                return Err(Error::Busy);
            }
            if queued
                .compare_exchange(current, current + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(Self(queued));
            }
        }
    }
}
impl Drop for QueueSlot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

async fn acquire(
    semaphore: Arc<Semaphore>,
    deadline: std::time::Instant,
) -> Result<OwnedSemaphorePermit, Error> {
    tokio::time::timeout(
        remaining(deadline).map_err(|_| Error::Busy)?,
        semaphore.acquire_owned(),
    )
    .await
    .map_err(|_| Error::Busy)?
    .map_err(|_| Error::Busy)
}

fn remaining(deadline: std::time::Instant) -> Result<std::time::Duration, Error> {
    deadline
        .checked_duration_since(std::time::Instant::now())
        .ok_or(Error::ProviderFailed)
}

fn fake_complete(route: &Route, _request: &Request) -> Response {
    // Do not echo prompts, credential identifiers, or any host configuration.
    Response {
        selection: route.selection.clone(),
        text: "Fake AI response".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real_config(secret: std::path::PathBuf) -> Config {
        let selection = Selection {
            provider: "fixture".into(),
            model: "test".into(),
        };
        let mut config = Config::default();
        config
            .credentials
            .insert("fixture-key".into(), "fixture".into());
        config.credential_files.insert("fixture-key".into(), secret);
        config.provider_endpoints.insert(
            "fixture".into(),
            "https://fixture.invalid/v1/chat/completions".into(),
        );
        config.routes.push(Route {
            selection: selection.clone(),
            credential: "fixture-key".into(),
        });
        config.apps.insert(
            "deployment-a".into(),
            AppPolicy {
                requests_ai: true,
                credential_grants: ["fixture-key".into()].into_iter().collect(),
                default: Some(selection),
            },
        );
        config
    }

    #[test]
    fn provider_rejects_downgrade_and_credential_leaking_endpoints() {
        for endpoint in [
            "http://fixture.invalid/v1",
            "https://key@fixture.invalid/v1",
            "https://fixture.invalid/v1#fragment",
            "not a URL",
        ] {
            assert!(!valid_endpoint(endpoint), "{endpoint}");
        }
        assert!(valid_endpoint(
            "https://fixture.invalid/v1/chat/completions"
        ));
    }

    #[tokio::test]
    async fn denied_request_does_not_read_credential_or_contact_provider() {
        let missing_secret = std::path::PathBuf::from("/definitely/not/a/paraco-secret");
        let mut config = real_config(missing_secret);
        config.apps.get_mut("deployment-a").unwrap().requests_ai = false;
        let provider = OpenAiProxy::new(config).unwrap();
        assert!(matches!(
            provider
                .complete(
                    "deployment-a",
                    &Request {
                        prompt: "private".into(),
                        ..Request::default()
                    }
                )
                .await,
            Err(Error::CapabilityNotRequested)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_world_readable_host_secret() {
        use std::os::unix::fs::PermissionsExt;
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "secret").unwrap();
        let mut permissions = std::fs::metadata(file.path()).unwrap().permissions();
        permissions.set_mode(0o644);
        std::fs::set_permissions(file.path(), permissions).unwrap();
        assert!(matches!(
            read_secret(file.path()),
            Err(Error::CredentialUnavailable)
        ));
    }

    #[tokio::test]
    async fn missing_host_secret_is_redacted_before_transport() {
        let provider =
            OpenAiProxy::new(real_config("/definitely/not/a/paraco-secret".into())).unwrap();
        assert!(matches!(
            provider
                .complete(
                    "deployment-a",
                    &Request {
                        prompt: "private".into(),
                        ..Request::default()
                    }
                )
                .await,
            Err(Error::CredentialUnavailable)
        ));
    }

    #[test]
    fn sse_decoder_handles_split_unicode_and_requires_terminal_event() {
        let mut decoder = SseDecoder::default();
        let event = b"data: {\"choices\":[{\"delta\":{\"content\":\"h\xC3\xA9\"}}]}\n\n";
        assert!(decoder.push(&event[..45], 1024).unwrap().is_empty());
        let chunks = decoder.push(&event[45..], 1024).unwrap();
        assert!(matches!(&chunks[..], [SseEvent::Text(value)] if value == "h\u{e9}"));
        assert!(decoder.push(b"data: [DONE]\n\n", 1024).is_ok());
        assert!(decoder.finish().is_ok());
    }

    #[test]
    fn sse_decoder_rejects_malformed_and_truncated_records() {
        assert!(
            SseDecoder::default()
                .push(b"data: {secret}\n\n", 1024)
                .is_err()
        );
        let mut decoder = SseDecoder::default();
        decoder
            .push(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n",
                1024,
            )
            .unwrap();
        assert!(decoder.finish().is_err());
    }
}
