//! Bounded, host-owned SDK subprocess. No application-supplied command or environment.
use crate::ai::{Error, ProviderKind};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn complete(
    kind: &ProviderKind,
    model: &str,
    prompt: &str,
    maximum: usize,
    timeout: Duration,
) -> Result<String, Error> {
    let directory = tempfile::tempdir().map_err(|_| Error::ProviderFailed)?;
    let cli = matches!(kind, ProviderKind::CodexCli { .. });
    let mut command = match kind {
        ProviderKind::CodexCli { executable } => {
            validate_executable(executable).map_err(|_| Error::InvalidProvider)?;
            let mut command = tokio::process::Command::new(executable);
            command.args([
                "exec",
                "--skip-git-repo-check",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--color",
                "never",
            ]);
            // Preserve the user's model selection, but disable every external tool surface.
            for setting in [
                "approval_policy=\"never\"",
                "forced_login_method=\"chatgpt\"",
                "features.shell_tool=false",
                "features.unified_exec=false",
                "features.shell_snapshot=false",
                "features.multi_agent=false",
                "features.apps=false",
                "features.remote_plugin=false",
                "features.hooks=false",
                "features.memories=false",
                "features.goals=false",
                "features.code_mode.enabled=false",
                "tools.view_image=false",
                "mcp_servers={}",
                "plugins={}",
                "apps={_default={enabled=false}}",
                "web_search=\"disabled\"",
                "sandbox_workspace_write.network_access=false",
            ] {
                command.args(["-c", setting]);
            }
            if model != "default" {
                command.args(["--model", model]);
            }
            command.arg("-");
            command
        }
        ProviderKind::CodexSdk { node, module } | ProviderKind::ClaudeSdk { node, module } => {
            if !node.is_absolute() || !module.is_absolute() {
                return Err(Error::InvalidProvider);
            }
            let script = directory.path().join("sdk.mjs");
            std::fs::write(&script, include_str!("../runtime/ai-sdk.mjs"))
                .map_err(|_| Error::ProviderFailed)?;
            let mut command = tokio::process::Command::new(node);
            command
                .arg(script)
                .arg(if matches!(kind, ProviderKind::CodexSdk { .. }) {
                    "codex"
                } else {
                    "claude"
                })
                .arg(module);
            command
        }
        _ => return Err(Error::InvalidProvider),
    };
    command
        .current_dir(directory.path())
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // SDKs own their sign-in stores. Never pass ambient API keys or Node injection options.
    for key in [
        "HOME",
        "PATH",
        "USER",
        "LANG",
        "CODEX_HOME",
        "CLAUDE_CONFIG_DIR",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().map_err(|_| Error::CredentialUnavailable)?;
    let _group = ProcessGroup(child.id());
    let input = if cli {
        prompt.as_bytes().to_vec()
    } else {
        serde_json::to_vec(&serde_json::json!({"model": model, "prompt": prompt}))
            .map_err(|_| Error::ProviderFailed)?
    };
    tokio::time::timeout(timeout, async {
        let mut stdin = child.stdin.take().ok_or(Error::ProviderFailed)?;
        stdin
            .write_all(&input)
            .await
            .map_err(|_| Error::ProviderFailed)?;
        drop(stdin);
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .ok_or(Error::ProviderFailed)?
            .take(maximum as u64 + 1)
            .read_to_end(&mut output)
            .await
            .map_err(|_| Error::ProviderFailed)?;
        if output.len() > maximum {
            return Err(Error::ResponseTooLarge);
        }
        if !child
            .wait()
            .await
            .map_err(|_| Error::ProviderFailed)?
            .success()
        {
            return Err(Error::ProviderFailed);
        }
        if cli {
            return String::from_utf8(output).map_err(|_| Error::ProviderFailed);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&output).map_err(|_| Error::ProviderFailed)?;
        value["text"]
            .as_str()
            .map(str::to_owned)
            .ok_or(Error::ProviderFailed)
    })
    .await
    .map_err(|_| Error::ProviderFailed)?
}

struct ProcessGroup(Option<u32>);
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(id) = self.0 {
            unsafe {
                libc::kill(-(id as i32), libc::SIGKILL);
            }
        }
    }
}

/// Resolve only host search paths, never an application's working directory.
pub fn find_codex() -> Result<std::path::PathBuf, String> {
    let mut directories: Vec<_> = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        directories.extend([home.join(".local/bin"), home.join(".npm-global/bin")]);
    }
    directories.into_iter().filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("codex"))
        .find(|path| validate_executable(path).is_ok())
        .ok_or_else(|| "Codex CLI was not found. Install Codex on this computer, run codex login, then restart Paraco and try again.".into())
}

pub async fn check_codex_login(executable: &Path) -> Result<(), String> {
    validate_executable(executable)?;
    let directory = tempfile::tempdir().map_err(|_| "Cannot check Codex sign-in")?;
    let mut command = tokio::process::Command::new(executable);
    command
        .args(["login", "status"])
        .current_dir(directory.path())
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for key in ["HOME", "PATH", "USER", "LANG", "CODEX_HOME"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|_| "Cannot start Codex CLI. Check its installation and restart Paraco.")?;
    let _group = ProcessGroup(child.id());
    match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(_)) => Err("Codex needs sign-in. Run codex login on this computer, then save again.".into()),
        _ => Err("Could not check Codex sign-in. Check that codex login status works on this computer, then try again.".into()),
    }
}

pub fn validate_executable(path: &Path) -> Result<(), String> {
    validate_path(path).map_err(|_| "Codex CLI must be an existing absolute executable file")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(path)
            .map_err(|_| "Cannot read Codex CLI")?
            .permissions()
            .mode()
            & 0o111
            == 0
        {
            return Err("Codex CLI is not executable".into());
        }
    }
    Ok(())
}

pub fn validate_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || !path.is_file() {
        return Err("SDK Node and module paths must name existing absolute files".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(source: &str) -> (tempfile::TempDir, ProviderKind) {
        let dir = tempfile::tempdir().unwrap();
        let module = dir.path().join("sdk.mjs");
        std::fs::write(&module, source).unwrap();
        // Use the same Node installation as the test environment, including setup-node.
        let node = std::process::Command::new("node")
            .args(["--print", "process.execPath"])
            .output()
            .expect("Node must be available on PATH for SDK tests");
        assert!(node.status.success(), "Node executable lookup failed");
        let node = std::path::PathBuf::from(String::from_utf8(node.stdout).unwrap().trim());
        assert!(
            node.is_absolute(),
            "Node must report its absolute executable path"
        );
        let kind = ProviderKind::ClaudeSdk { node, module };
        (dir, kind)
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn codex_cli_uses_stdin_default_model_and_bounded_output() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("codex");
        std::fs::write(
            &executable,
            r#"#!/usr/bin/env node
const assert = require('node:assert/strict');
const args = process.argv.slice(2);
assert.equal(args[0], 'exec');
assert.ok(args.includes('read-only'));
assert.ok(args.includes('approval_policy="never"'));
assert.ok(args.includes('features.shell_tool=false'));
assert.ok(args.includes('mcp_servers={}'));
assert.equal(args.at(-1), '-');
assert.equal(process.env.NODE_OPTIONS, undefined);
assert.equal(process.env.OPENAI_API_KEY, undefined);
let prompt=''; process.stdin.on('data', c=>prompt+=c);
process.stdin.on('end', ()=> {
 if (prompt === 'fail') process.exit(1);
 if (prompt === 'hang') return setTimeout(()=>{}, 60000);
 const index = args.indexOf('--model');
 process.stdout.write((index < 0 ? 'default' : args[index+1])+':'+prompt);
});
"#,
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let kind = ProviderKind::CodexCli { executable };
        assert_eq!(
            complete(&kind, "default", "héllo", 1024, Duration::from_secs(5))
                .await
                .unwrap(),
            "default:héllo"
        );
        assert_eq!(
            complete(&kind, "chosen", "hello", 1024, Duration::from_secs(5))
                .await
                .unwrap(),
            "chosen:hello"
        );
        assert_eq!(
            complete(&kind, "default", "fail", 1024, Duration::from_secs(5)).await,
            Err(Error::ProviderFailed)
        );
        assert_eq!(
            complete(&kind, "default", "large", 2, Duration::from_secs(5)).await,
            Err(Error::ResponseTooLarge)
        );
        assert_eq!(
            complete(&kind, "default", "hang", 1024, Duration::from_millis(200)).await,
            Err(Error::ProviderFailed)
        );
    }

    #[tokio::test]
    async fn subprocess_normalizes_success_and_redacts_failure() {
        let (_dir, kind) = fixture(
            "export async function* query({prompt}) { yield {type:'result', subtype:'success', result:prompt}; }",
        );
        assert_eq!(
            complete(&kind, "test", "héllo", 1024, Duration::from_secs(5))
                .await
                .unwrap(),
            "héllo"
        );
        let (_dir, kind) =
            fixture("export async function* query() { throw Error('private-key'); }");
        assert!(matches!(
            complete(&kind, "test", "prompt", 1024, Duration::from_secs(5)).await,
            Err(Error::ProviderFailed)
        ));
    }
    #[tokio::test]
    async fn subprocess_enforces_deadline_and_output_budget() {
        let (_dir, kind) = fixture(
            "export async function* query() { await new Promise(r=>setTimeout(r,60000)); }",
        );
        let start = std::time::Instant::now();
        assert!(matches!(
            complete(&kind, "test", "prompt", 1024, Duration::from_millis(200)).await,
            Err(Error::ProviderFailed)
        ));
        assert!(start.elapsed() < Duration::from_secs(3));
        let (_dir, kind) = fixture(
            "export async function* query() { yield {type:'result', subtype:'success', result:'x'.repeat(10000)}; }",
        );
        assert!(matches!(
            complete(&kind, "test", "prompt", 100, Duration::from_secs(5)).await,
            Err(Error::ResponseTooLarge)
        ));
    }
}
