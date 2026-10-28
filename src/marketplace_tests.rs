use super::*;
use base64::Engine;
use serde_json::json;

fn entry(name: &str, version: &str, sha256: String, capabilities: &[&str]) -> Entry {
    Entry {
        name: name.into(),
        title: format!("{name} app"),
        description: "A marketplace test app".into(),
        repository: "https://github.com/dioko-ai/paraco-hello-world".into(),
        version: version.into(),
        commit: "a".repeat(40),
        package: format!(
            "https://github.com/dioko-ai/paraco-hello-world/releases/download/v{version}/{name}.paraco.json"
        ),
        sha256,
        capabilities: capabilities.iter().map(|value| (*value).into()).collect(),
    }
}

fn package(name: &str, version: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": 1,
        "name": name,
        "version": version,
        "files": files.iter().map(|(path, bytes)| json!({
            "path": path,
            "content": STANDARD.encode(bytes),
        })).collect::<Vec<_>>(),
    }))
    .unwrap()
}

fn valid_package(name: &str, version: &str, capabilities: &[&str]) -> Vec<u8> {
    let manifest = serde_json::to_string(&json!({
        "schemaVersion": 1,
        "name": name,
        "entrypoint": "main.ts",
        "capabilities": capabilities,
    }))
    .unwrap();
    package(
        name,
        version,
        &[
            ("paraco.json", manifest.as_bytes()),
            ("main.ts", b"export {};"),
        ],
    )
}

fn install_fixture(
    bytes: &[u8],
    name: &str,
    version: &str,
    capabilities: &[&str],
) -> (tempfile::TempDir, Entry, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("installed");
    let approved = entry(name, version, digest(bytes), capabilities);
    (directory, approved, output)
}

#[test]
fn installs_approved_package_and_loads_manifest() {
    let bytes = valid_package("hello-world", "0.1.0", &[]);
    let (_directory, approved, output) = install_fixture(&bytes, "hello-world", "0.1.0", &[]);

    install(&approved, &bytes, &output).unwrap();
    let app = crate::manifest::load(&output).unwrap();
    assert_eq!(app.name, "hello-world");
    assert!(app.entrypoint.ends_with("main.ts"));
    assert!(output.join("paraco-install.json").is_file());
    assert_eq!(fs::read(output.join("main.ts")).unwrap(), b"export {};");
}

#[test]
fn rejects_hash_mismatch_without_creating_destination() {
    let bytes = valid_package("hello-world", "0.1.0", &[]);
    let (_directory, mut approved, output) = install_fixture(&bytes, "hello-world", "0.1.0", &[]);
    approved.sha256 = "b".repeat(64);

    assert!(
        install(&approved, &bytes, &output)
            .unwrap_err()
            .contains("SHA-256")
    );
    assert!(!output.exists());
}

#[test]
fn rejects_approved_bytes_with_wrong_identity_or_capabilities() {
    let wrong_identity = valid_package("different-app", "0.1.0", &[]);
    let (_directory, approved, output) =
        install_fixture(&wrong_identity, "hello-world", "0.1.0", &[]);
    assert!(
        install(&approved, &wrong_identity, &output)
            .unwrap_err()
            .contains("identity/version")
    );
    assert!(!output.exists());

    let wrong_capabilities = valid_package("hello-world", "0.1.0", &["ai"]);
    let (_directory, approved, output) =
        install_fixture(&wrong_capabilities, "hello-world", "0.1.0", &[]);
    assert!(
        install(&approved, &wrong_capabilities, &output)
            .unwrap_err()
            .contains("name/capabilities")
    );
    assert!(!output.exists());
}

#[test]
fn rejects_traversal_case_collisions_and_reserved_paths() {
    for paths in [
        vec![("../escape", b"x".as_slice())],
        vec![
            ("Readme.md", b"a".as_slice()),
            ("README.md", b"b".as_slice()),
        ],
        vec![("paraco-install.json", b"fake receipt".as_slice())],
        vec![
            ("Dir/file", b"a".as_slice()),
            ("dir/other", b"b".as_slice()),
        ],
        vec![
            ("parent", b"file".as_slice()),
            ("parent/child", b"nested".as_slice()),
        ],
    ] {
        let bytes = package("hello-world", "0.1.0", &paths);
        let (_directory, approved, output) = install_fixture(&bytes, "hello-world", "0.1.0", &[]);
        assert!(
            install(&approved, &bytes, &output).is_err(),
            "accepted paths: {paths:?}"
        );
        assert!(!output.exists());
    }
}

#[test]
fn existing_destination_is_preserved() {
    let bytes = valid_package("hello-world", "0.1.0", &[]);
    let (_directory, approved, output) = install_fixture(&bytes, "hello-world", "0.1.0", &[]);
    fs::create_dir(&output).unwrap();
    fs::write(output.join("keep.txt"), "keep me").unwrap();

    assert!(
        install(&approved, &bytes, &output)
            .unwrap_err()
            .contains("already exists")
    );
    assert_eq!(
        fs::read_to_string(output.join("keep.txt")).unwrap(),
        "keep me"
    );
    assert!(!output.join("main.ts").exists());
}

#[test]
fn catalog_rejects_duplicate_versions_and_unknown_fields() {
    let sample = entry("hello-world", "0.1.0", "a".repeat(64), &[]);
    let duplicate = Catalog {
        schema_version: 1,
        apps: vec![
            entry(&sample.name, &sample.version, sample.sha256.clone(), &[]),
            entry(&sample.name, &sample.version, sample.sha256.clone(), &[]),
        ],
    };
    let bytes = serde_norway::to_string(&duplicate).unwrap();
    assert!(
        parse_catalog(bytes.as_bytes())
            .err()
            .unwrap()
            .contains("duplicate app version")
    );

    assert!(parse_catalog(b"schemaVersion: 1\napps: []\nextra: true\n").is_err());
}

#[test]
fn selection_requires_approved_single_version() {
    let catalog = Catalog {
        schema_version: 1,
        apps: vec![
            entry("hello-world", "0.1.0", "a".repeat(64), &[]),
            entry("hello-world", "0.2.0", "b".repeat(64), &[]),
        ],
    };
    assert!(select(&catalog, "unknown-app", Some("0.1.0")).is_err());
    assert!(select(&catalog, "hello-world", Some("0.9.0")).is_err());
    assert!(
        select(&catalog, "hello-world", None)
            .err()
            .unwrap()
            .contains("multiple approved versions")
    );
    assert_eq!(
        select(&catalog, "hello-world", Some("0.2.0"))
            .unwrap()
            .version,
        "0.2.0"
    );
}

#[test]
fn rejects_malformed_base64() {
    let bytes = br#"{"schemaVersion":1,"name":"hello-world","version":"0.1.0","files":[{"path":"main.ts","content":"not base64!"}]}"#;
    let (_directory, approved, output) = install_fixture(bytes, "hello-world", "0.1.0", &[]);
    assert!(install(&approved, bytes, &output).is_err());
    assert!(!output.exists());
}

#[test]
fn rejects_unsupported_package_schema() {
    let bytes = br#"{"schemaVersion":2,"name":"hello-world","version":"0.1.0","files":[{"path":"main.ts","content":"ZXhwb3J0IHt9"}]}"#;
    let (_directory, approved, output) = install_fixture(bytes, "hello-world", "0.1.0", &[]);
    assert!(
        install(&approved, bytes, &output)
            .unwrap_err()
            .contains("identity/version")
    );
    assert!(!output.exists());
}

#[test]
fn atomic_publish_refuses_a_destination_created_during_staging() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("staged");
    let destination = root.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("app"), "new").unwrap();
    fs::create_dir(&destination).unwrap();
    assert!(publish_new(&source, &destination).is_err());
    assert!(source.join("app").is_file());
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
}

#[test]
fn rejects_file_directory_conflicts_in_either_order() {
    for files in [
        vec![
            ("parent/child", b"x".as_slice()),
            ("parent", b"x".as_slice()),
        ],
        vec![
            ("parent", b"x".as_slice()),
            ("parent/child", b"x".as_slice()),
        ],
    ] {
        let bytes = package("hello-world", "0.1.0", &files);
        let (_directory, approved, output) = install_fixture(&bytes, "hello-world", "0.1.0", &[]);
        assert!(
            install(&approved, &bytes, &output)
                .unwrap_err()
                .contains("path conflict")
        );
        assert!(!output.exists());
    }
}

#[test]
fn bundled_catalog_approves_all_eight_showcase_packages() {
    let catalog = catalog(None).unwrap();
    for name in [
        "system-vitals",
        "prompt-regression-lab",
        "cron-time-machine",
        "json-yaml-workbench",
        "ai-memory-curator",
        "local-uptime-sentry",
        "github-pulse",
        "scientific-lab-notebook",
    ] {
        let entry = select(&catalog, name, Some("0.1.0")).unwrap();
        assert_eq!(
            entry.repository,
            "https://github.com/dioko-ai/paraco-plugins"
        );
        if name == "system-vitals" {
            assert_eq!(entry.capabilities, ["system"]);
        }
    }
}

#[test]
fn installs_approved_system_capability_without_weakening_capability_checks() {
    let bytes = valid_package("system-vitals", "0.1.0", &["system"]);
    let (_directory, approved, output) =
        install_fixture(&bytes, "system-vitals", "0.1.0", &["system"]);
    install(&approved, &bytes, &output).unwrap();
    assert!(crate::manifest::load(&output).unwrap().requests_system);
    let (_directory, missing_grant, output) =
        install_fixture(&bytes, "system-vitals", "0.1.0", &[]);
    assert!(
        install(&missing_grant, &bytes, &output)
            .unwrap_err()
            .contains("name/capabilities")
    );
    assert!(!output.exists());
}

#[test]
fn abandoned_lock_does_not_block_installation() {
    let bytes = valid_package("hello-world", "0.1.0", &[]);
    let (directory, approved, output) = install_fixture(&bytes, "hello-world", "0.1.0", &[]);
    fs::write(directory.path().join(".paraco-install.lock"), "").unwrap();
    install(&approved, &bytes, &output).unwrap();
    assert!(install(&approved, &bytes, &output).is_err());
    assert_eq!(fs::read(output.join("main.ts")).unwrap(), b"export {};");
}
