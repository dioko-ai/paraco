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
    if !cfg!(target_os = "linux") && std::env::var("PARACO_ALLOW_UNSANDBOXED").as_deref() != Ok("1")
    {
        return Err("Application compilation requires a supported containment adapter; trusted development may explicitly set PARACO_ALLOW_UNSANDBOXED=1".into());
    }
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
    let mut command;
    if cfg!(target_os = "linux") && std::env::var("PARACO_ALLOW_UNSANDBOXED").as_deref() != Ok("1")
    {
        command = Command::new("/usr/bin/bwrap");
        command
            .args([
                "--unshare-all",
                "--die-with-parent",
                "--new-session",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "--tmpfs",
                "/tmp",
            ])
            .arg("--ro-bind")
            .arg(&compiler)
            .arg("/compiler")
            .arg("--ro-bind")
            .arg(&source)
            .arg(&source)
            .arg("--bind")
            .arg(output.parent().ok_or("output needs parent")?)
            .arg(output.parent().unwrap())
            .arg("--chdir")
            .arg(&source)
            .arg("/compiler");
    } else {
        command = Command::new(&compiler);
    }
    let result = command
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
        .env_clear()
        .output()
        .map_err(|e| format!("cannot compile application: {e}"))?;
    if !result.status.success() {
        return Err(format!(
            "application compilation failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
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
    _dir: &Path,
    config: &Path,
    listener: &std::net::TcpListener,
) -> Result<Command, String> {
    use std::os::{fd::AsRawFd, unix::process::CommandExt};
    let unsafe_dev = std::env::var("PARACO_ALLOW_UNSANDBOXED").as_deref() == Ok("1");
    let mut command;
    if unsafe_dev {
        eprintln!(
            "paraco: explicit unsandboxed development mode; use only trusted application code"
        );
        command = Command::new(runtime);
        command.arg("serve").arg(config);
    } else {
        #[cfg(target_os = "linux")]
        {
            let sandbox = Path::new("/usr/bin/bwrap");
            if !sandbox.is_file() {
                return Err("Linux execution requires bubblewrap; install it or explicitly opt into trusted development with PARACO_ALLOW_UNSANDBOXED=1".into());
            }
            command = Command::new(sandbox);
            command.args([
                "--unshare-all",
                "--die-with-parent",
                "--new-session",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                "--tmpfs",
                "/tmp",
            ]);
            for library in ["/usr", "/lib", "/lib64", "/etc/ssl/certs"] {
                if Path::new(library).exists() {
                    command.arg("--ro-bind").arg(library).arg(library);
                }
            }
            command
                .arg("--ro-bind")
                .arg(runtime)
                .arg("/runtime")
                .arg("--ro-bind")
                .arg(_dir)
                .arg(_dir)
                .args(["--chdir", "/", "/runtime", "serve"])
                .arg(config);
        }
        #[cfg(not(target_os = "linux"))]
        {
            return Err("This platform requires an external VM containment adapter; native untrusted execution is not supported. Trusted development may explicitly set PARACO_ALLOW_UNSANDBOXED=1".into());
        }
    }
    let fd = listener.as_raw_fd();
    command
        .arg("--socket-fd=http=3")
        .env_clear()
        .env("TZ", "UTC")
        .stdin(std::process::Stdio::null());
    unsafe {
        command.pre_exec(move || {
            if fd != 3 && libc::dup2(fd, 3) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::fcntl(3, libc::F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            let limit = libc::rlimit {
                rlim_cur: 256,
                rlim_max: 256,
            };
            if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(command)
}
