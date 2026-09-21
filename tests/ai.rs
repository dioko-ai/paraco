use paraco::ai::{AppPolicy, Config, Error, FakeProxy, Request, Route, Selection};

fn pair(provider: &str, model: &str) -> Selection {
    Selection {
        provider: provider.into(),
        model: model.into(),
    }
}

fn request(provider: Option<&str>, model: Option<&str>) -> Request {
    Request {
        provider: provider.map(String::from),
        model: model.map(String::from),
        prompt: "private prompt".into(),
    }
}

fn config() -> Config {
    Config {
        credentials: [
            ("alice-key", "alpha"),
            ("bob-key", "beta"),
            ("second-alpha-key", "alpha"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect(),
        routes: [
            ("alpha", "small", "alice-key"),
            ("alpha", "shared", "alice-key"),
            ("beta", "large", "bob-key"),
            ("beta", "shared", "bob-key"),
            ("alpha", "small", "second-alpha-key"),
        ]
        .into_iter()
        .map(|(p, m, c)| Route {
            selection: pair(p, m),
            credential: c.into(),
        })
        .collect(),
        apps: [
            (
                "alice".into(),
                AppPolicy {
                    requests_ai: true,
                    credential_grants: ["alice-key".into()].into(),
                    default: Some(pair("alpha", "small")),
                },
            ),
            (
                "bob".into(),
                AppPolicy {
                    requests_ai: true,
                    credential_grants: ["bob-key".into()].into(),
                    default: None,
                },
            ),
        ]
        .into(),
        default: Some(pair("beta", "large")),
        provider_default_models: [
            ("alpha".into(), "small".into()),
            ("beta".into(), "large".into()),
        ]
        .into(),
        ..Config::default()
    }
}

fn selection(config: Config, caller: &str, request: Request) -> Result<Selection, Error> {
    FakeProxy::new(config)?
        .complete(caller, &request)
        .map(|response| response.selection)
}

#[test]
fn app_default_precedes_runtime_default_and_runtime_fills_absence() {
    assert_eq!(
        selection(config(), "alice", Request::default()),
        Ok(pair("alpha", "small"))
    );
    assert_eq!(
        selection(config(), "bob", Request::default()),
        Ok(pair("beta", "large"))
    );
}

#[test]
fn automatic_defaults_only_use_permitted_routes() {
    let mut c = config();
    c.apps.get_mut("bob").unwrap().default = Some(pair("alpha", "small"));
    assert_eq!(
        selection(c, "bob", Request::default()),
        Ok(pair("beta", "large"))
    );
    let mut c = config();
    c.apps.get_mut("alice").unwrap().default = None;
    assert_eq!(
        selection(c, "alice", Request::default()),
        Err(Error::NoDefault)
    );
}

#[test]
fn missing_default_route_falls_back_but_no_defaults_fail() {
    let mut c = config();
    c.apps.get_mut("bob").unwrap().default = Some(pair("beta", "missing"));
    assert_eq!(
        selection(c, "bob", Request::default()),
        Ok(pair("beta", "large"))
    );
    let mut c = config();
    c.default = None;
    assert_eq!(
        selection(c, "bob", Request::default()),
        Err(Error::NoDefault)
    );
}

#[test]
fn provider_only_uses_its_configured_model() {
    assert_eq!(
        selection(config(), "alice", request(Some("alpha"), None)),
        Ok(pair("alpha", "small"))
    );
    let mut c = config();
    c.provider_default_models.clear();
    assert_eq!(
        selection(c, "alice", request(Some("alpha"), None)),
        Err(Error::NoProviderDefault)
    );
}

#[test]
fn explicit_pair_overrides_defaults_without_substitution() {
    assert_eq!(
        selection(config(), "alice", request(Some("alpha"), Some("shared"))),
        Ok(pair("alpha", "shared"))
    );
    assert_eq!(
        selection(config(), "alice", request(Some("alpha"), Some("missing"))),
        Err(Error::NoRoute)
    );
    assert_eq!(
        selection(config(), "alice", request(Some("beta"), Some("large"))),
        Err(Error::CredentialNotGranted)
    );
    assert_eq!(
        selection(config(), "alice", request(Some("beta"), None)),
        Err(Error::CredentialNotGranted)
    );
}

#[test]
fn model_only_resolves_within_callers_grants() {
    assert_eq!(
        selection(config(), "alice", request(None, Some("shared"))),
        Ok(pair("alpha", "shared"))
    );
    assert_eq!(
        selection(config(), "bob", request(None, Some("shared"))),
        Ok(pair("beta", "shared"))
    );
    assert_eq!(
        selection(config(), "alice", request(None, Some("large"))),
        Err(Error::CredentialNotGranted)
    );
    assert_eq!(
        selection(config(), "alice", request(None, Some("missing"))),
        Err(Error::NoRoute)
    );
}

#[test]
fn ambiguous_model_requires_provider_even_when_app_has_default() {
    let mut c = config();
    c.apps
        .get_mut("alice")
        .unwrap()
        .credential_grants
        .insert("bob-key".into());
    assert_eq!(
        selection(c, "alice", request(None, Some("shared"))),
        Err(Error::AmbiguousModel)
    );
}

#[test]
fn multiple_credentials_for_one_pair_are_not_ambiguous() {
    let mut c = config();
    c.apps
        .get_mut("alice")
        .unwrap()
        .credential_grants
        .insert("second-alpha-key".into());
    assert_eq!(
        selection(c, "alice", request(None, Some("small"))),
        Ok(pair("alpha", "small"))
    );
}

#[test]
fn granted_credential_can_follow_an_ungranted_route_for_same_pair() {
    let mut c = config();
    c.apps.get_mut("alice").unwrap().credential_grants = ["second-alpha-key".into()].into();
    assert_eq!(
        selection(c, "alice", request(Some("alpha"), Some("small"))),
        Ok(pair("alpha", "small"))
    );
}

#[test]
fn capability_request_does_not_grant_credentials() {
    for r in [
        Request::default(),
        request(Some("alpha"), None),
        request(Some("alpha"), Some("small")),
        request(None, Some("small")),
    ] {
        let mut c = config();
        c.apps.get_mut("alice").unwrap().credential_grants.clear();
        assert!(selection(c, "alice", r).is_err());
    }
}

#[test]
fn grants_do_not_replace_capability_request_and_unknown_callers_fail() {
    let mut c = config();
    c.apps.get_mut("alice").unwrap().requests_ai = false;
    assert_eq!(
        selection(c, "alice", Request::default()),
        Err(Error::CapabilityNotRequested)
    );
    assert_eq!(
        selection(config(), "unknown", Request::default()),
        Err(Error::UnknownCaller)
    );
}

#[test]
fn blank_explicit_selection_is_not_treated_as_omission() {
    for r in [
        request(Some(""), None),
        request(None, Some("  ")),
        request(Some("alpha"), Some("")),
    ] {
        assert_eq!(
            selection(config(), "alice", r),
            Err(Error::InvalidSelection)
        );
    }
}

#[test]
fn invalid_credential_references_and_provider_mismatches_fail_at_configuration() {
    for credential in ["missing", "bob-key"] {
        let mut c = config();
        c.routes[0].credential = credential.into();
        assert!(matches!(
            FakeProxy::new(c),
            Err(Error::InvalidConfiguration)
        ));
    }
    let mut c = config();
    c.apps
        .get_mut("alice")
        .unwrap()
        .credential_grants
        .insert("missing".into());
    assert!(matches!(
        FakeProxy::new(c),
        Err(Error::InvalidConfiguration)
    ));
}

#[test]
fn fake_response_is_deterministic_and_excludes_private_inputs() {
    let proxy = FakeProxy::new(config()).unwrap();
    let request = request(Some("alpha"), Some("small"));
    let first = proxy.complete("alice", &request).unwrap();
    let second = proxy.complete("alice", &request).unwrap();
    assert!(first == second);
    assert_eq!(first.selection, pair("alpha", "small"));
    assert_eq!(first.text, "Fake AI response");
    for private in ["private prompt", "alice-key", "bob-key"] {
        assert!(!first.text.contains(private));
        assert!(!Error::CredentialNotGranted.to_string().contains(private));
    }
}
