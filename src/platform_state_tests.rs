use super::*;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

fn write_app(root: &Path, name: &str, capabilities: &[&str]) -> crate::manifest::App {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("main.ts"), "export default {};").unwrap();
    fs::write(
        root.join("paraco.json"),
        json!({"name":name,"entrypoint":"main.ts","capabilities":capabilities}).to_string(),
    )
    .unwrap();
    crate::manifest::load(root).unwrap()
}

#[test]
fn capability_expansion_requires_approval_and_baseline_persists() {
    let root = tempfile::tempdir().unwrap();
    let app_root = root.path().join("app");
    let state_root = root.path().join("state");
    let initial = write_app(&app_root, "sample", &[]);
    let installation_id;

    {
        let mut store = Store::open(&state_root).unwrap();
        installation_id = store.install(&app_root).unwrap().id;
        assert!(
            store
                .check_capabilities(&installation_id, &initial, false)
                .is_ok()
        );

        let expanded = write_app(&app_root, "sample", &["storage"]);
        assert!(
            store
                .check_capabilities(&installation_id, &expanded, false)
                .unwrap_err()
                .contains("additional capabilities")
        );
        store
            .check_capabilities(&installation_id, &expanded, true)
            .unwrap();

        let renamed = write_app(&app_root, "renamed", &["storage"]);
        assert!(
            store
                .check_capabilities(&installation_id, &renamed, true)
                .unwrap_err()
                .contains("identity changed")
        );
    }

    let mut reopened = Store::open(&state_root).unwrap();
    let approved = write_app(&app_root, "sample", &["storage"]);
    reopened
        .check_capabilities(&installation_id, &approved, false)
        .unwrap();
    let reduced = write_app(&app_root, "sample", &[]);
    reopened
        .check_capabilities(&installation_id, &reduced, false)
        .unwrap();
}

#[test]
fn schema_v2_migration_preserves_deployments_and_legacy_storage() {
    let root = tempfile::tempdir().unwrap();
    let app_root = root.path().join("app");
    fs::create_dir(&app_root).unwrap();
    let source = app_root
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let id = "old-installation";
    let state_root = root.path().join("state");
    fs::create_dir(&state_root).unwrap();
    let db_path = state_root.join("state.sqlite3");
    let db = rusqlite::Connection::open(&db_path).unwrap();
    db.execute_batch(
        "CREATE TABLE deployments(source TEXT PRIMARY KEY, id TEXT UNIQUE NOT NULL, desired INTEGER NOT NULL CHECK(desired IN (0,1)));
         CREATE TABLE storage(deployment TEXT NOT NULL, namespace TEXT NOT NULL CHECK(namespace IN ('config','data')), key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(deployment,namespace,key));
         PRAGMA user_version=2;",
    )
    .unwrap();
    db.execute(
        "INSERT INTO deployments(source,id,desired) VALUES(?1,?2,1)",
        rusqlite::params![source, id],
    )
    .unwrap();
    db.execute(
        "INSERT INTO storage(deployment,namespace,key,value) VALUES(?1,'data','kept','\"old-value\"')",
        [id],
    )
    .unwrap();
    drop(db);

    let mut store = Store::open(&state_root).unwrap();
    assert_eq!(store.install(&app_root).unwrap().id, id);
    assert_eq!(
        store.storage(id, "data", "kept", None, false).unwrap(),
        json!("old-value")
    );
    assert_eq!(
        store
            .db
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        3
    );
}

#[test]
fn platform_private_records_and_artifacts_survive_store_backup_restore() {
    let root = tempfile::tempdir().unwrap();
    let app_root = root.path().join("app");
    let _app = write_app(&app_root, "sample", &[]);
    let state_root = root.path().join("state");
    let backup = root.path().join("state-backup.sqlite3");
    let installation_id;
    let artifact_id;
    {
        let mut store = Store::open(&state_root).unwrap();
        installation_id = store.install(&app_root).unwrap().id;
        let record = store
            .platform(
                &installation_id,
                "sample",
                &Value::Null,
                &json!({"dataset":"private","action":"record.put","key":"private-key","expectedVersion":null,"value":{"preserved":true}}),
            )
            .unwrap();
        assert_eq!(record["version"], 1);
        let artifact = store
            .platform(
                &installation_id,
                "sample",
                &Value::Null,
                &json!({"dataset":"private","action":"artifact.publish","draft":{"output":"report","mediaType":"application/json","content":{"preserved":true}}}),
            )
            .unwrap();
        artifact_id = artifact["id"].as_str().unwrap().to_owned();
        store.backup(&backup).unwrap();
    }

    let restored_root = root.path().join("restored");
    fs::create_dir(&restored_root).unwrap();
    fs::copy(&backup, restored_root.join("state.sqlite3")).unwrap();
    let mut restored = Store::open(&restored_root).unwrap();
    let record = restored
        .platform(
            &installation_id,
            "sample",
            &Value::Null,
            &json!({"dataset":"private","action":"record.get","key":"private-key"}),
        )
        .unwrap();
    assert_eq!(record["value"]["preserved"], true);
    let artifact = restored
        .platform(
            &installation_id,
            "sample",
            &Value::Null,
            &json!({"dataset":"private","action":"artifact.get","id":artifact_id}),
        )
        .unwrap();
    assert_eq!(artifact["content"]["preserved"], true);
}

#[test]
fn platform_config_requires_private_file_permissions_and_reserves_private_alias() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("platform.json");
    fs::write(
        &config,
        r#"{"installations":{"install-1":{"datasets":{"private":{"id":"shared"}}}}}"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(
        crate::platform::Config::load(root.path())
            .err()
            .unwrap()
            .contains("private dataset cannot be rebound")
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::write(&config, r#"{"installations":{}}"#).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            crate::platform::Config::load(root.path())
                .err()
                .unwrap()
                .contains("owner-only")
        );
    }
}

#[cfg(unix)]
#[test]
fn platform_config_rejects_unencrypted_remote_urls() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("platform.json");
    fs::write(
        &config,
        r#"{"installations":{"install-1":{"remote":{"url":"http://dataset.example/api","token":"scoped"}}}}"#,
    )
    .unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        crate::platform::Config::load(root.path())
            .err()
            .unwrap()
            .contains("requires HTTPS")
    );
}
