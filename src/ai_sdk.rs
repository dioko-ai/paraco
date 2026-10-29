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
    let (provider, node, module) = match kind {
        ProviderKind::CodexSdk { node, module } => ("codex", node, module),
        ProviderKind::ClaudeSdk { node, module } => ("claude", node, module),
        _ => return Err(Error::InvalidProvider),
    };
    if !node.is_absolute() || !module.is_absolute() {
        return Err(Error::InvalidProvider);
    }
    let directory = tempfile::tempdir().map_err(|_| Error::ProviderFailed)?;
    let script = directory.path().join("sdk.mjs");
    std::fs::write(&script, include_str!("../runtime/ai-sdk.mjs"))
        .map_err(|_| Error::ProviderFailed)?;
    let mut command = tokio::process::Command::new(node);
    command
        .arg(&script)
        .arg(provider)
        .arg(module)
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
    let input = serde_json::to_vec(&serde_json::json!({"model": model, "prompt": prompt}))
        .map_err(|_| Error::ProviderFailed)?;
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
