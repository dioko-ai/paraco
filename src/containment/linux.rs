use super::ExecutionSpec;
use std::{path::Path, process::Command};

pub(super) fn command(spec: &ExecutionSpec<'_>) -> Result<Command, String> {
    let wrapper = Path::new("/usr/bin/bwrap");
    if !wrapper.is_file() {
        return Err(
            "Linux containment requires /usr/bin/bwrap and usable unprivileged namespaces".into(),
        );
    }
    let executable = spec.executable.canonicalize().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(wrapper);
    cmd.args([
        "--unshare-all",
        "--die-with-parent",
        "--new-session",
        "--cap-drop",
        "ALL",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
    ]);
    // Libraries, not all of /usr (which exposed unrelated executable tools).
    for path in ["/usr/lib", "/usr/lib64", "/lib", "/lib64", "/etc/ssl/certs"] {
        if Path::new(path).exists() {
            cmd.args(["--ro-bind", path, path]);
        }
    }
    cmd.arg("--ro-bind").arg(&executable).arg(&executable);
    for path in spec.readable {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        if path == Path::new("/") {
            return Err("refusing filesystem root grant".into());
        }
        cmd.arg("--ro-bind").arg(&path).arg(&path);
    }
    for path in spec.writable {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        if path == Path::new("/") {
            return Err("refusing filesystem root grant".into());
        }
        cmd.arg("--bind").arg(&path).arg(&path);
    }
    // The socket is normally already in the read-only launch directory.
    if let Some(socket) = spec.capability_socket
        && !spec.readable.iter().any(|p| socket.starts_with(p))
    {
        cmd.arg("--ro-bind").arg(socket).arg(socket);
    }
    let _ = spec.kind;
    cmd.arg("--chdir")
        .arg(spec.cwd.canonicalize().map_err(|e| e.to_string())?)
        .arg(executable);
    Ok(cmd)
}
