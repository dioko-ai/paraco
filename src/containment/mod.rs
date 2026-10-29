//! Native execution policy. Platform details stay behind a path-based contract.
//! Capability authorization remains in the host bridge, not in these adapters.
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub mod diagnostics;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod macos;

#[derive(Clone, Copy)]
pub enum WorkloadKind {
    Compiler,
    Runtime,
}

pub struct ExecutionSpec<'a> {
    pub executable: &'a Path,
    pub readable: &'a [PathBuf],
    pub writable: &'a [PathBuf],
    pub cwd: &'a Path,
    pub kind: WorkloadKind,
    pub capability_socket: Option<&'a Path>,
}

pub fn backend_name() -> &'static str {
    if std::env::var("PARACO_ALLOW_UNSANDBOXED").as_deref() == Ok("1") {
        "unsafe-development"
    } else if cfg!(target_os = "linux") {
        "linux-bubblewrap"
    } else if cfg!(target_os = "macos") {
        "macos-seatbelt-experimental"
    } else {
        "unsupported"
    }
}

/// No automatic fallback. macOS stays experimental until native release checks pass.
pub fn command(spec: &ExecutionSpec<'_>) -> Result<Command, String> {
    let mut cmd = if backend_name() == "unsafe-development" {
        eprintln!(
            "paraco: explicit unsandboxed development mode; use only trusted application code"
        );
        Command::new(spec.executable)
    } else {
        #[cfg(target_os = "linux")]
        {
            linux::command(spec)?
        }
        #[cfg(target_os = "macos")]
        {
            if std::env::var("PARACO_EXPERIMENTAL_MACOS_SANDBOX").as_deref() != Ok("1") {
                return Err("Native macOS containment is experimental and has not passed release validation. To test it, explicitly set PARACO_EXPERIMENTAL_MACOS_SANDBOX=1. No VM or unsafe fallback is used.".into());
            }
            macos::command(spec)?
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            return Err("No native containment adapter is available for this platform".into());
        }
    };
    cmd.current_dir(spec.cwd)
        .env_clear()
        .env("TZ", "UTC")
        .stdin(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        #[cfg(target_os = "macos")]
        let max_fd = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
        unsafe {
            cmd.pre_exec(move || {
                // Set CLOEXEC rather than closing the std::process error pipe.
                // The runtime listener is explicitly re-enabled afterwards.
                #[cfg(target_os = "linux")]
                if libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 4u32) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                #[cfg(target_os = "macos")]
                {
                    if max_fd < 0 {
                        return Err(std::io::Error::other("cannot determine descriptor limit"));
                    }
                    for fd in 3..max_fd as i32 {
                        if libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) == -1
                            && std::io::Error::last_os_error().raw_os_error() != Some(libc::EBADF)
                        {
                            return Err(std::io::Error::last_os_error());
                        }
                    }
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
    }
    Ok(cmd)
}

/// Preserve explicitly selected host modes across the cleared guardian environment.
pub fn guardian_environment(cmd: &mut Command) {
    for key in [
        "PARACO_ALLOW_UNSANDBOXED",
        "PARACO_EXPERIMENTAL_MACOS_SANDBOX",
    ] {
        if std::env::var(key).as_deref() == Ok("1") {
            cmd.env(key, "1");
        }
    }
}
