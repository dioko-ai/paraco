//! Offline AI policy core. Configuration and caller identity are supplied by the
//! trusted host, never by the application's request payload.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

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

/// In-memory configuration for this milestone. It is not an app manifest schema.
#[derive(Default)]
pub struct Config {
    /// Credential identity -> owning provider. No real secrets are needed yet.
    pub credentials: BTreeMap<String, String>,
    /// Configuration order breaks ties between credentials for the same pair.
    pub routes: Vec<Route>,
    pub apps: BTreeMap<String, AppPolicy>,
    pub default: Option<Selection>,
    pub provider_default_models: BTreeMap<String, String>,
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
        if config
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

fn fake_complete(route: &Route, _request: &Request) -> Response {
    // Do not echo prompts, credential identifiers, or any host configuration.
    Response {
        selection: route.selection.clone(),
        text: "Fake AI response".into(),
    }
}
