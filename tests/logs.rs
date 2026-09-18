//! CLI directory resolution does not require Deno or a running server.
use std::{fs, process::Command};

#[test]
fn log_directory_precedence_and_offline_reading() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let env_path = dir.path().join("environment");
    let explicit = dir.path().join("explicit");
    let invoke = |override_env: bool, override_flag: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_paraco"));
        command
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env_remove("PARACO_LOG_DIR");
        if override_env {
            command.env("PARACO_LOG_DIR", &env_path);
        }
        command.args(["logs", "hello", "--tail", "100"]);
        if override_flag {
            command.arg("--log-dir").arg(&explicit);
        }
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stdout.is_empty());
    };
    invoke(true, true);
    assert!(explicit.join("current.jsonl").exists());
    assert!(!env_path.exists());
    assert!(!home.exists());
    invoke(true, false);
    assert!(env_path.join("current.jsonl").exists());
    assert!(!home.exists());
    invoke(false, false);
    assert!(home.join(".paraco/logs/current.jsonl").exists());
    let bytes = fs::read(explicit.join("current.jsonl")).unwrap();
    assert!(bytes.is_empty());
    let invalid = Command::new(env!("CARGO_BIN_EXE_paraco"))
        .args(["logs", "--tail", "10001", "--log-dir"])
        .arg(&explicit)
        .output()
        .unwrap();
    assert!(!invalid.status.success());
}
