//! Opt-in user-service installation and native-manager operations.
//! Explicit CLI actions invoke systemctl or launchctl; definitions must be
//! owned by Paraco before replacement.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    thread,
    time::{Duration, Instant},
};

pub const SYSTEMD_UNIT: &str = "paraco.service";
pub const LAUNCH_AGENT: &str = "dev.paraco.host.plist";
const MARKER: &str = "# Managed by Paraco; do not edit.\n";
const LAUNCH_MARKER: &str = "<!-- Managed by Paraco; do not edit. -->";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Platform {
    Linux,
    Macos,
}

fn absolute(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!("{label} must be an absolute path"));
    }
    if path.as_os_str().is_empty()
        || path.to_string_lossy().contains('\n')
        || path.to_string_lossy().contains('\0')
    {
        return Err(format!("{label} contains an unsupported control character"));
    }
    Ok(())
}

fn systemd_path(path: &Path) -> Result<String, String> {
    absolute(path, "service path")?;
    let value = path.to_string_lossy();
    if value.contains('"') || value.contains('\\') || value.contains('%') || value.contains('$') {
        return Err("service paths cannot contain quotes or backslashes".into());
    }
    Ok(value.into_owned())
}
fn systemd_quote(path: &Path) -> Result<String, String> {
    // systemd's ExecStart parser treats a quoted argument as one argv item.
    Ok(format!("\"{}\"", systemd_path(path)?))
}
fn xml(value: &Path) -> Result<String, String> {
    absolute(value, "service path")?;
    let value = value.to_string_lossy();
    Ok(value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;"))
}

pub fn render_systemd(entry: &Path, config: &Path, state: &Path) -> Result<String, String> {
    let entry = systemd_quote(entry)?;
    let config = systemd_quote(config)?;
    let logs = systemd_quote(&state.join("logs"))?;
    // WorkingDirectory takes a path, not an ExecStart-style argument list.
    // Quotes would become literal path characters and make it non-absolute.
    let state = systemd_path(state)?;
    Ok(format!(
        "{MARKER}[Unit]\nDescription=Paraco local host\n\n[Service]\nType=simple\nExecStart={entry} serve --config {config} --log-dir {logs}\nWorkingDirectory={state}\nRestart=on-failure\nRestartSec=2\nTimeoutStopSec=10\n\n[Install]\nWantedBy=default.target\n"
    ))
}

pub fn render_launch_agent(entry: &Path, config: &Path, state: &Path) -> Result<String, String> {
    let entry = xml(entry)?;
    let config = xml(config)?;
    let logs = xml(&state.join("logs"))?;
    let temporary = xml(&std::env::temp_dir())?;
    let state = xml(state)?;
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{LAUNCH_MARKER}\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>dev.paraco.host</string>\n<key>ProgramArguments</key><array><string>{entry}</string><string>serve</string><string>--config</string><string>{config}</string><string>--log-dir</string><string>{logs}</string></array>\n<key>EnvironmentVariables</key><dict><key>TMPDIR</key><string>{temporary}</string></dict>\n<key>WorkingDirectory</key><string>{state}</string>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\n<key>ThrottleInterval</key><integer>2</integer>\n<key>ProcessType</key><string>Background</string>\n<key>ExitTimeOut</key><integer>10</integer>\n</dict></plist>\n"
    ))
}

pub fn definition_path(platform: Platform, home: &Path) -> Result<PathBuf, String> {
    absolute(home, "home directory")?;
    Ok(match platform {
        Platform::Linux => home.join(".config/systemd/user").join(SYSTEMD_UNIT),
        Platform::Macos => home.join("Library/LaunchAgents").join(LAUNCH_AGENT),
    })
}

fn is_owned_contents(text: &str) -> bool {
    text.starts_with(MARKER)
        || (text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n")
            && text
                .strip_prefix("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n")
                .is_some_and(|rest| rest.starts_with(LAUNCH_MARKER)))
}

pub fn write_owned(path: &Path, contents: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path)
        && !is_owned_contents(&existing)
    {
        return Err(format!(
            "refusing to replace foreign service registration {}",
            path.display()
        ));
    }
    let parent = path.parent().ok_or("service definition has no parent")?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("cannot create service definition directory: {e}"))?;
    let temp = parent.join(format!(
        ".{}.tmp",
        path.file_name()
            .ok_or("invalid service definition name")?
            .to_string_lossy()
    ));
    fs::write(&temp, contents).map_err(|e| format!("cannot write service definition: {e}"))?;
    fs::rename(&temp, path).map_err(|e| format!("cannot activate service definition: {e}"))
}

pub fn is_owned(path: &Path) -> Result<bool, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(is_owned_contents(&text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("cannot read service definition: {error}")),
    }
}

pub fn remove_owned(path: &Path) -> Result<(), String> {
    match fs::read_to_string(path) {
        Ok(text) if is_owned_contents(&text) => {
            fs::remove_file(path).map_err(|e| format!("cannot remove service definition: {e}"))
        }
        Ok(_) => Err(format!(
            "refusing to remove foreign service registration {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot read service definition: {error}")),
    }
}

pub struct Paths {
    /// A stable, Paraco-owned launcher path. Its target may advance between
    /// activations, but already-running processes retain their executable.
    pub entry: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
    pub definition: PathBuf,
}

pub fn service_entry(home: &Path) -> Result<PathBuf, String> {
    absolute(home, "home directory")?;
    Ok(home.join(".paraco/service/paraco"))
}

/// Derive stable service paths from the owning user's home. No PATH or current
/// working directory is consulted by generated definitions.
pub fn paths(platform: Platform, config: &Path) -> Result<Paths, String> {
    absolute(config, "config path")?;
    let home = std::env::var_os("HOME").ok_or("HOME is required for user-service setup")?;
    let home = PathBuf::from(home);
    absolute(&home, "home directory")?;
    Ok(Paths {
        entry: service_entry(&home)?,
        config: config.to_path_buf(),
        state: home.join(".paraco/state"),
        definition: definition_path(platform, &home)?,
    })
}

/// Atomically install a stable owned launcher for future restarts. Replacing the
/// symlink does not affect a process already running the immutable executable.
fn install_entry(entry: &Path) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|e| format!("cannot resolve paraco executable: {e}"))?
        .canonicalize()
        .map_err(|e| format!("cannot canonicalize paraco executable: {e}"))?;
    let parent = entry.parent().ok_or("service entry has no parent")?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("cannot create service entry directory: {e}"))?;
    if entry.exists()
        && !fs::symlink_metadata(entry)
            .map_err(|e| format!("cannot inspect service entry: {e}"))?
            .file_type()
            .is_symlink()
    {
        return Err(format!(
            "refusing to replace foreign service entry {}",
            entry.display()
        ));
    }
    let temp = parent.join(".paraco-service-entry.tmp");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&executable, &temp)
        .map_err(|e| format!("cannot create service entry: {e}"))?;
    #[cfg(not(unix))]
    return Err("user-service setup is only supported on Unix".into());
    fs::rename(&temp, entry).map_err(|e| format!("cannot activate service entry: {e}"))
}

pub fn render(platform: Platform, paths: &Paths) -> Result<String, String> {
    match platform {
        Platform::Linux => render_systemd(&paths.entry, &paths.config, &paths.state),
        Platform::Macos => render_launch_agent(&paths.entry, &paths.config, &paths.state),
    }
}

fn run_bounded(command: &str, args: &[String]) -> Result<ExitStatus, String> {
    let mut child = Command::new(command).args(args).spawn().map_err(|e| {
        format!("cannot invoke {command}; install/use the native user-service manager: {e}")
    })?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("cannot wait for {command}: {e}"))?
        {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{command} timed out after 10 seconds"));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn manager(platform: Platform, action: &str, definition: &Path) -> Result<(), String> {
    let (program, args): (&str, Vec<String>) = match (platform, action) {
        (Platform::Linux, "setup") => ("systemctl", vec!["--user".into(), "daemon-reload".into()]),
        (Platform::Linux, "remove") => (
            "systemctl",
            vec![
                "--user".into(),
                "disable".into(),
                "--now".into(),
                SYSTEMD_UNIT.into(),
            ],
        ),
        (Platform::Linux, "status") => (
            "systemctl",
            vec![
                "--user".into(),
                "status".into(),
                "--no-pager".into(),
                SYSTEMD_UNIT.into(),
            ],
        ),
        (Platform::Macos, "setup") => (
            "launchctl",
            vec![
                "bootstrap".into(),
                format!("gui/{}", unsafe { libc::geteuid() }),
                definition.display().to_string(),
            ],
        ),
        (Platform::Macos, "remove") => (
            "launchctl",
            vec![
                "bootout".into(),
                format!("gui/{}/dev.paraco.host", unsafe { libc::geteuid() }),
            ],
        ),
        (Platform::Macos, "status") => (
            "launchctl",
            vec![
                "print".into(),
                format!("gui/{}/dev.paraco.host", unsafe { libc::geteuid() }),
            ],
        ),
        _ => return Err("unsupported service operation".into()),
    };
    let status = run_bounded(program, &args)?;
    if !status.success() {
        return Err(format!("{program} {action} failed with {status}"));
    }
    if platform == Platform::Linux && action == "setup" {
        let status = run_bounded(
            "systemctl",
            &[
                "--user".into(),
                "enable".into(),
                "--now".into(),
                SYSTEMD_UNIT.into(),
            ],
        )?;
        if !status.success() {
            return Err(format!("systemctl enable failed with {status}"));
        }
    }
    Ok(())
}

fn setup(
    paths: &Paths,
    platform: Platform,
    activate: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    fs::create_dir_all(&paths.state).map_err(|e| format!("cannot create state directory: {e}"))?;
    if paths.definition.exists() && !is_owned(&paths.definition)? {
        return Err("refusing to replace foreign service registration".into());
    }
    let definition = render(platform, paths)?;
    let previous_definition = fs::read_to_string(&paths.definition).ok();
    let previous_entry = fs::read_link(&paths.entry).ok();
    install_entry(&paths.entry)?;
    let result = write_owned(&paths.definition, &definition).and_then(|()| activate());
    if let Err(error) = result {
        let restore = (|| -> Result<(), String> {
            if let Some(previous) = previous_definition {
                write_owned(&paths.definition, &previous)?;
            } else {
                remove_owned(&paths.definition)?;
            }
            fs::remove_file(&paths.entry).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            if let Some(previous) = previous_entry {
                std::os::unix::fs::symlink(previous, &paths.entry).map_err(|e| e.to_string())?;
            }
            Ok(())
        })();
        return match restore {
            Ok(()) => Err(format!("{error}; previous service files restored")),
            Err(rollback) => Err(format!("{error}; service rollback failed: {rollback}")),
        };
    }
    Ok(())
}

/// Explicit setup/removal/status operations. They never invoke sudo, never delete
/// state/configuration, and reject a foreign registration before manager calls.
pub fn operate(platform: Platform, action: &str, config: &Path) -> Result<(), String> {
    if (platform == Platform::Linux && !cfg!(target_os = "linux"))
        || (platform == Platform::Macos && !cfg!(target_os = "macos"))
    {
        return Err("service operations require the matching native operating system".into());
    }
    let paths = paths(platform, config)?;
    match action {
        "setup" => setup(&paths, platform, || {
            manager(platform, action, &paths.definition)
        }),
        "remove" => {
            if paths.definition.exists() && !is_owned(&paths.definition)? {
                return Err(format!(
                    "refusing to remove foreign service registration {}",
                    paths.definition.display()
                ));
            }
            manager(platform, action, &paths.definition)?;
            remove_owned(&paths.definition)
        }
        "status" => {
            if !is_owned(&paths.definition)? {
                return Err("Paraco user service is not registered by this installation".into());
            }
            manager(platform, action, &paths.definition)
        }
        _ => Err("service action must be setup, remove, or status".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    #[cfg(unix)]
    #[test]
    fn failed_activation_restores_previous_definition_and_launcher() {
        let root = tempdir().unwrap();
        let paths = Paths {
            entry: root.path().join("paraco"),
            config: root.path().join("config"),
            state: root.path().join("state"),
            definition: root.path().join("service"),
        };
        std::os::unix::fs::symlink("/old/paraco", &paths.entry).unwrap();
        let previous = render(Platform::Macos, &paths).unwrap();
        fs::write(&paths.definition, &previous).unwrap();
        assert!(
            setup(&paths, Platform::Macos, || Err("fixture failure".into()))
                .unwrap_err()
                .contains("previous service files restored")
        );
        assert_eq!(
            fs::read_link(&paths.entry).unwrap(),
            Path::new("/old/paraco")
        );
        assert_eq!(fs::read_to_string(&paths.definition).unwrap(), previous);
    }

    #[test]
    fn renders_paths_as_single_arguments() {
        let s = render_systemd(
            Path::new("/opt/paraco/bin/paraco"),
            Path::new("/tmp/a b/config.json"),
            Path::new("/tmp/state dir"),
        )
        .unwrap();
        assert!(s.contains(
            "ExecStart=\"/opt/paraco/bin/paraco\" serve --config \"/tmp/a b/config.json\""
        ));
        assert!(s.contains("TimeoutStopSec=10"));
        assert!(s.contains("\nWorkingDirectory=/tmp/state dir\n"));
    }
    #[test]
    fn escapes_plist_and_rejects_relative_paths() {
        assert!(
            render_launch_agent(Path::new("/a&b"), Path::new("/c<d"), Path::new("/state"))
                .unwrap()
                .contains("/a&amp;b")
        );
        assert!(render_systemd(Path::new("paraco"), Path::new("/c"), Path::new("/s")).is_err());
    }
    #[test]
    fn stable_entry_is_absolute_and_independent_of_release() {
        assert_eq!(
            service_entry(Path::new("/home/a")).unwrap(),
            Path::new("/home/a/.paraco/service/paraco")
        );
    }
    #[test]
    fn launch_agent_has_xml_declaration_before_ownership_marker_and_bounded_shutdown() {
        let definition =
            render_launch_agent(Path::new("/bin/paraco"), Path::new("/c"), Path::new("/s"))
                .unwrap();
        assert!(definition.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"));
        assert!(definition.contains(&format!("\n{LAUNCH_MARKER}\n")));
        assert!(definition.contains("<key>ExitTimeOut</key><integer>10</integer>"));
        assert!(is_owned_contents(&definition));
    }
    #[test]
    fn refuses_foreign_and_removes_only_owned() {
        let d = tempdir().unwrap();
        let p = d.path().join("unit");
        fs::write(&p, "foreign").unwrap();
        assert!(write_owned(&p, "x").is_err());
        assert!(remove_owned(&p).is_err());
        fs::remove_file(&p).unwrap();
        write_owned(
            &p,
            &render_systemd(Path::new("/bin/paraco"), Path::new("/c"), Path::new("/s")).unwrap(),
        )
        .unwrap();
        remove_owned(&p).unwrap();
        assert!(!p.exists());
    }
}
