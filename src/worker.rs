//! Compilation and explicit workerd service wiring. No ambient internet binding.
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub fn compiler() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("PARACO_ESBUILD") {
        return Ok(path.into());
    }
    let exe = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if let Some(root) = exe.parent().and_then(Path::parent) {
        let bundled = root.join("libexec/paraco/esbuild");
        if root.join("bundle.json").is_file() {
            if !bundled.is_file() {
                return Err(
                    "private bundled esbuild compiler is missing; reinstall the release bundle"
                        .into(),
                );
            }
            return Ok(bundled);
        }
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let path = dir.join("esbuild");
        if path.is_file() {
            return Ok(path);
        }
    }
    Err("TypeScript preparation requires the bundled esbuild compiler (or PARACO_ESBUILD)".into())
}

pub fn compile(entry: &Path, output: &Path, root: &Path) -> Result<(), String> {
    let snapshot = tempfile::tempdir().map_err(|e| e.to_string())?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let relative = entry
        .strip_prefix(&root)
        .map_err(|_| "entrypoint outside application")?;
    let source = snapshot.path().join("source");
    crate::artifact::copy_tree(&root, &source)?;
    let entry = source.join(relative);
    let meta = output.with_extension("meta.json");
    let compiler = compiler()?.canonicalize().map_err(|e| e.to_string())?;
    let mut command = compiler_command(
        &compiler,
        &source,
        output.parent().ok_or("output needs parent")?,
    )?;
    command
        .arg(&entry)
        .args([
            "--bundle",
            "--format=esm",
            "--platform=browser",
            "--target=es2022",
            "--log-level=error",
        ])
        .arg(format!("--outfile={}", output.display()))
        .arg(format!("--metafile={}", meta.display()))
        .current_dir(&source)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let (success, diagnostic) =
        crate::tasks::bounded_command(command, std::time::Duration::from_secs(30), || false)
            .map_err(|e| format!("cannot compile application: {e}"))?;
    if !success {
        return Err(format!("application compilation failed: {diagnostic}"));
    }
    let graph: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&meta).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let root = source.canonicalize().map_err(|e| e.to_string())?;
    for name in graph["inputs"]
        .as_object()
        .ok_or("compiler did not report inputs")?
        .keys()
    {
        let path = root.join(name).canonicalize().map_err(|e| e.to_string())?;
        if !path.starts_with(&root) {
            return Err("application dependency escapes its package".into());
        }
    }
    std::fs::remove_file(meta).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn config(
    dir: &Path,
    port: u16,
    bridge: Option<&str>,
    settings: &serde_json::Value,
) -> Result<PathBuf, String> {
    std::fs::write(
        dir.join("platform-client.js"),
        include_str!("../runtime/platform-client.js"),
    )
    .map_err(|e| e.to_string())?;
    let q = |s: &str| serde_json::to_string(s).unwrap();
    let binding = bridge
        .map(|_| ", (name = \"HOST\", service = \"host\")")
        .unwrap_or_default();
    let external = bridge
        .map(|address| {
            format!(
                ", (name = \"host\", external = (address = {}, http = ()))",
                q(address)
            )
        })
        .unwrap_or_default();
    let source = format!(
        r#"using Workerd = import "/workerd/workerd.capnp";
const config :Workerd.Config = (
 services = [
 (name = "app", worker = (compatibilityDate = "2026-08-03", compatibilityFlags = ["disallow_importable_env"], modules = [
 (name = "adapter.js", esModule = embed "adapter.js"),
 (name = "app.js", esModule = embed "app.js"),
 (name = "platform-client.js", esModule = embed "platform-client.js")],
 bindings = [(name = "SETTINGS", json = {}){}], globalOutbound = "blocked")),
 (name = "blocked", network = (allow = [])){}
 ], sockets = [(name = "http", address = "127.0.0.1:{}", http = (), service = "app")]
);
"#,
        q(&settings.to_string()),
        binding,
        external,
        port
    );
    let path = dir.join("worker.capnp");
    std::fs::write(&path, source).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn task_network(config: &Path, dir: &Path, address: &str, token: &str) -> Result<(), String> {
    let script = format!(
        r#"const token = {};
export default {{ async fetch(request, env) {{
 const bytes=new Uint8Array(await request.arrayBuffer());
 if(bytes.length>65536) throw new Error('Task network body exceeds 64 KiB');
 const body=btoa(Array.from(bytes, b=>String.fromCharCode(b)).join(''));
 const response=await env.NET.fetch('http://host/network',{{method:'POST',headers:{{authorization:`Bearer ${{token}}`}},body:JSON.stringify({{url:request.url,method:request.method,headers:[...request.headers],body}})}});
 const value=await response.json();
 if(value.error) throw new Error(value.error);
 return new Response([204,304].includes(value.status)?null:Uint8Array.from(atob(value.body),c=>c.charCodeAt(0)),{{status:value.status,headers:value.headers}});
}} }};"#,
        serde_json::to_string(token).map_err(|e| e.to_string())?
    );
    std::fs::write(dir.join("egress.js"), script).map_err(|e| e.to_string())?;
    let extra = format!(
        r#"(name = "egress", worker = (compatibilityDate = "2026-08-03", compatibilityFlags = ["disallow_importable_env"], modules = [(name = "egress.js", esModule = embed "egress.js")], bindings = [(name = "NET", service = "network")], globalOutbound = "blocked")),
(name = "network", external = (address = {}, http = ())),
(name = "blocked", network"#,
        serde_json::to_string(address).map_err(|e| e.to_string())?
    );
    let text = std::fs::read_to_string(config)
        .map_err(|e| e.to_string())?
        .replace(
            "globalOutbound = \"blocked\"",
            "globalOutbound = \"egress\"",
        )
        .replace("(name = \"blocked\", network", &extra);
    std::fs::write(config, text).map_err(|e| e.to_string())
}

/// Separate OS containment from isolate capability policy. Never silently fall back.
pub fn launch_command(
    runtime: &Path,
    dir: &Path,
    config: &Path,
    listener: &std::net::TcpListener,
) -> Result<Command, String> {
    use std::os::{fd::AsRawFd, unix::process::CommandExt};
    let runtime = runtime.canonicalize().map_err(|e| e.to_string())?;
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    let socket = dir.join("host.sock");
    let mut command = crate::containment::command(&crate::containment::ExecutionSpec {
        executable: &runtime,
        readable: std::slice::from_ref(&dir),
        writable: &[],
        cwd: &dir,
        kind: crate::containment::WorkloadKind::Runtime,
        capability_socket: socket.exists().then_some(socket.as_path()),
    })?;
    command.arg("serve").arg(config);
    let fd = listener.as_raw_fd();
    command
        .arg(format!("--socket-fd=http={fd}"))
        .env_clear()
        .env("TZ", "UTC")
        .stdin(std::process::Stdio::null());
    unsafe {
        command.pre_exec(move || {
            // Preserve the existing descriptor number: dup2 onto fd 3 could
            // overwrite std::process's private spawn-error pipe.
            if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(command)
}

/// Both application bundling and task transforms use the same native boundary.
pub fn compiler_command(compiler: &Path, source: &Path, output: &Path) -> Result<Command, String> {
    let compiler = compiler.canonicalize().map_err(|e| e.to_string())?;
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    let output = output.canonicalize().map_err(|e| e.to_string())?;
    crate::containment::command(&crate::containment::ExecutionSpec {
        executable: &compiler,
        readable: std::slice::from_ref(&source),
        writable: std::slice::from_ref(&output),
        cwd: if source.is_dir() { &source } else { &output },
        kind: crate::containment::WorkloadKind::Compiler,
        capability_socket: None,
    })
}
