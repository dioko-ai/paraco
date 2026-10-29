//! Experimental macOS Seatbelt adapter.
//!
//! `sandbox-exec` and SBPL are deprecated/undocumented interfaces, and this
//! policy must be exercised on each supported macOS release before it is
//! relied on. Seatbelt is not a filesystem or network namespace: inherited
//! descriptors remain capabilities. The caller must close unrelated descriptors
//! and pass only the intended listener. App Sandbox inheritance alone does not
//! provide a separate per-plugin boundary. Never fall back on profile failure.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::ExecutionSpec;

// Executable mapping is a separate Seatbelt operation from file-read*.
// dyld support on macOS 13+ also includes the system Cryptex locations; see
// Chromium sandbox/policy/mac/common.sb (dyld-support.sb rules).
// These are OS runtime locations, never application inputs or writable scratch.
const LOADER_PATHS: &[&str] = &[
    "/System/Library",
    "/usr/lib",
    "/System/Cryptexes/App",
    "/System/Cryptexes/OS",
    "/System/Volumes/Preboot/Cryptexes/App/System",
    "/System/Volumes/Preboot/Cryptexes/OS",
];

fn canonical(path: &Path) -> Result<PathBuf, String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("cannot resolve containment path {}: {e}", path.display()))?;
    if path == Path::new("/") {
        return Err("refusing to grant access to the filesystem root".into());
    }
    Ok(path)
}

// Reject control characters rather than relying on undocumented SBPL escape
// forms. Non-UTF-8 paths cannot safely be represented in an inline profile.
fn quoted(path: &Path) -> Result<String, String> {
    let s = path.to_str().ok_or("containment path is not UTF-8")?;
    if s.chars().any(char::is_control) {
        return Err("containment path contains a control character".into());
    }
    Ok(format!(
        "\"{}\"",
        s.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn path_rule(profile: &mut String, operation: &str, path: &Path) -> Result<(), String> {
    let path = canonical(path)?;
    let filter = if path.is_dir() { "subpath" } else { "literal" };
    profile.push_str(&format!(
        "(allow {operation} ({filter} {}))\n",
        quoted(&path)?
    ));
    Ok(())
}

fn profile(spec: &ExecutionSpec<'_>) -> Result<String, String> {
    let executable = canonical(spec.executable)?;
    if !executable.is_file() {
        return Err("containment executable is not a regular file".into());
    }
    // Resolve cwd up front: a missing working directory must fail closed.
    let cwd = canonical(spec.cwd)?;
    if !cwd.is_dir() {
        return Err("containment working directory is not a directory".into());
    }
    let mut p = String::from(
        "(version 1)\n(deny default)\n\
        (allow signal (target self))\n\
        (allow sysctl-read (sysctl-name \"kern.osrelease\") (sysctl-name \"kern.osversion\") (sysctl-name \"hw.ncpu\") (sysctl-name \"hw.activecpu\") (sysctl-name \"hw.memsize\"))\n\
        (allow mach-lookup (global-name \"com.apple.system.logger\"))\n\
        (allow file-read* (subpath \"/System/Library\") (subpath \"/usr/lib\") (literal \"/dev/null\") (literal \"/dev/urandom\") (literal \"/dev/random\"))\n\
        (allow file-write* (literal \"/dev/null\"))\n",
    );
    p.push_str(&format!(
        "(allow process-exec (literal {}))\n",
        quoted(&executable)?
    ));
    p.push_str(&format!(
        "(allow file-read* file-map-executable (literal {}))\n",
        quoted(&executable)?
    ));
    for path in LOADER_PATHS {
        // Do not canonicalize these on the build host: Cryptex locations vary
        // by macOS version and must not resolve to a broader ancestor grant.
        p.push_str(&format!(
            "(allow file-read* file-map-executable (subpath {}))\n",
            quoted(Path::new(path))?
        ));
    }
    for path in spec.readable {
        path_rule(&mut p, "file-read*", path)?;
    }
    for path in spec.writable {
        path_rule(&mut p, "file-read*", path)?;
        path_rule(&mut p, "file-write*", path)?;
    }
    if let Some(socket) = spec.capability_socket {
        let socket = canonical(socket)?;
        p.push_str("(allow system-socket (socket-domain AF_UNIX))\n");
        p.push_str(&format!(
            "(allow file-read* (literal {}))\n",
            quoted(&socket)?
        ));
        p.push_str(&format!(
            "(allow network-outbound (remote unix-socket (literal {})))\n",
            quoted(&socket)?
        ));
    }
    // Accept on the caller-supplied listener. No IP socket creation or bind is
    // granted; the inherited descriptor is the only intended IP capability.
    if matches!(spec.kind, super::WorkloadKind::Runtime) {
        p.push_str("(allow network-inbound (local ip \"localhost:*\"))\n");
    }
    Ok(p)
}

pub(super) fn command(spec: &ExecutionSpec<'_>) -> Result<Command, String> {
    let policy = profile(spec)?;
    let executable = canonical(spec.executable)?;
    let mut cmd = Command::new("/usr/bin/sandbox-exec");
    cmd.arg("-p").arg(policy).arg(executable);
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_cannot_escape_the_profile() {
        assert_eq!(quoted(Path::new("/a/\\\"b")).unwrap(), "\"/a/\\\\\\\"b\"");
        assert!(quoted(Path::new("/a/\nb")).is_err());
    }

    #[test]
    fn refuses_root_grants() {
        assert!(canonical(Path::new("/")).is_err());
    }

    #[test]
    fn executable_mapping_is_limited_to_binary_and_system_loader() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let executable = root.join("runtime");
        std::fs::write(&executable, b"profile fixture").unwrap();
        let source = root.join("source");
        let scratch = root.join("scratch");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&scratch).unwrap();
        let policy = profile(&ExecutionSpec {
            executable: &executable,
            readable: std::slice::from_ref(&source),
            writable: std::slice::from_ref(&scratch),
            cwd: &scratch,
            kind: super::super::WorkloadKind::Compiler,
            capability_socket: None,
        })
        .unwrap();
        let mapping: Vec<_> = policy
            .lines()
            .filter(|line| line.contains("file-map-executable"))
            .collect();
        assert_eq!(mapping.len(), LOADER_PATHS.len() + 1);
        assert!(
            mapping.contains(
                &format!(
                    "(allow file-read* file-map-executable (literal {}))",
                    quoted(&executable).unwrap()
                )
                .as_str()
            )
        );
        for path in LOADER_PATHS {
            assert!(
                mapping.contains(
                    &format!(
                        "(allow file-read* file-map-executable (subpath {}))",
                        quoted(Path::new(path)).unwrap()
                    )
                    .as_str()
                )
            );
        }
        assert!(policy.contains("(deny default)"));
        assert!(!policy.contains("(allow network-outbound)"));
        assert!(!policy.contains("(allow process-fork)"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_utf8_paths() {
        use std::os::unix::ffi::OsStrExt;
        assert!(quoted(Path::new(std::ffi::OsStr::from_bytes(b"/a/\xff"))).is_err());
    }
}
