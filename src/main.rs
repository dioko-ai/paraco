use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod artifact;
mod bridge;
mod capability;
mod containment;
mod control;
mod data;
mod guardian;
mod logs;
mod manifest;
mod marketplace;
mod platform;
mod reference;
mod runner;
mod runtime;
mod sandbox_probe;
mod server;
mod service;
mod state;
mod storage;
mod task_api;
mod tasks;
mod telemetry;
mod worker;

#[derive(Parser)]
#[command(name = "paraco", version, about = "Run local Paraco applications")]
struct Cli {
    /// Persistent JSONL log directory (otherwise PARACO_LOG_DIR or ~/.paraco/logs).
    #[arg(long, global = true)]
    log_dir: Option<PathBuf>,
    /// Application data directory (otherwise PARACO_DATA_DIR or ~/.paraco); independent of logs.
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run native containment canaries and print JSON; exit nonzero unless all pass.
    SandboxCheck,
    /// Serve the bundled escape-probe plugin with independent native test evidence.
    SandboxProbe {
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },
    /// Discover host stores or print the identity to preserve when relocating configuration.
    HostState {
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Validate a package's declared capabilities for an execution target without deploying it.
    Check {
        app: PathBuf,
        #[arg(long, value_parser=["local", "cloudflare"], default_value="local")]
        target: String,
    },
    /// Explicitly approve an installed source's current capability manifest (runtime stopped).
    ApproveCapabilities {
        app: PathBuf,
        #[arg(long)]
        state: PathBuf,
    },
    /// Browse and install versions approved by a host-owned marketplace catalog.
    Marketplace {
        #[command(subcommand)]
        command: marketplace::Command,
    },
    /// Run the bundled cron app; no checkout or separate app download required.
    Cron {
        #[arg(long, default_value_t = 3000)]
        port: u16,
        /// Host-owned AI configuration; AI drafts remain optional.
        #[arg(long)]
        ai_config: Option<PathBuf>,
        /// Maximum task network grant. Each published revision requests a subset.
        #[arg(long)]
        allow_task_network: Vec<String>,
    },
    /// Open the main dashboard in your browser.
    Open {
        #[arg(long, default_value_t = 3000)]
        port: u16,
        #[arg(long)]
        print: bool,
    },
    /// Export a stopped runtime's state as a consistent SQLite database.
    Backup {
        #[arg(long)]
        state: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Compile an application and pin its runtime in an immutable offline artifact.
    Prepare {
        /// Source application directory.
        app: PathBuf,
        /// New host-owned artifact directory to create.
        #[arg(long)]
        output: PathBuf,
        /// Explicit workerd executable used to run this artifact.
        #[arg(long)]
        workerd: PathBuf,
    },
    /// Read retained JSONL records, even when the runtime is stopped.
    Logs {
        app: Option<String>,
        #[arg(long, default_value_t = 100)]
        tail: usize,
        #[arg(long)]
        port: Option<u16>,
    },
    /// Inspect applications in a running local server.
    Status {
        app: Option<String>,
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },
    /// Print the stable host-owned ID used by standalone run for an app source.
    Identity {
        /// Application directory.
        app: PathBuf,
    },
    /// Start a stopped or failed application in a running local server.
    Start {
        app: String,
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },
    /// Stop one application without stopping the server.
    Stop {
        app: String,
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },
    /// Stop and then start one application in a running local server.
    Restart {
        app: String,
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },
    /// Render, install, or remove an explicitly opt-in user-service definition.
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    /// Host configured applications and a dashboard on loopback.
    Serve {
        /// Local server configuration; app paths are relative to this file.
        #[arg(long)]
        config: PathBuf,
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },
    /// Run an application directory containing paraco.json.
    Run {
        /// Application directory.
        app: PathBuf,
        /// Loopback TCP port for the application.
        #[arg(long, default_value_t = 3000)]
        port: u16,
        /// Host-owned AI routing, grants, and optional provider configuration.
        #[arg(long)]
        ai_config: Option<PathBuf>,
    },
    /// Run a verified prepared artifact without dependency downloads.
    RunPrepared {
        /// Prepared artifact directory created by `paraco prepare`.
        artifact: PathBuf,
        #[arg(long, default_value_t = 3000)]
        port: u16,
        #[arg(long)]
        ai_config: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ServiceCommand {
    /// Print a private user-service definition.
    Render {
        #[arg(long)]
        platform: ServicePlatform,
        #[arg(long)]
        entry: PathBuf,
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        state: PathBuf,
    },
    /// Explicitly register and start an owned user service. Never uses sudo.
    Setup {
        #[arg(long)]
        platform: ServicePlatform,
        #[arg(long)]
        config: PathBuf,
    },
    /// Stop and unregister an owned user service; preserves config and data.
    Remove {
        #[arg(long)]
        platform: ServicePlatform,
        #[arg(long)]
        config: PathBuf,
    },
    /// Query an owned user service through its native user-service manager.
    Status {
        #[arg(long)]
        platform: ServicePlatform,
        #[arg(long)]
        config: PathBuf,
    },
}

#[derive(Clone, clap::ValueEnum)]
enum ServicePlatform {
    Linux,
    Macos,
}

impl From<ServicePlatform> for service::Platform {
    fn from(value: ServicePlatform) -> Self {
        match value {
            ServicePlatform::Linux => service::Platform::Linux,
            ServicePlatform::Macos => service::Platform::Macos,
        }
    }
}

fn main() {
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("__paraco_containment_probe"))
    {
        std::process::exit(containment::diagnostics::child());
    }
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("__paraco_containment_check"))
    {
        println!("{}", containment::diagnostics::verify());
        return;
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("__paraco_task_guardian"))
    {
        std::process::exit(tasks::guardian());
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("__paraco_guardian")) {
        std::process::exit(guardian::run());
    }
    let cli = Cli::parse();
    let result = execute(cli);
    if let Err(error) = result {
        eprintln!("paraco: {error}");
        std::process::exit(1);
    }
}

fn service_log_dir(path: Option<&std::path::Path>) -> Result<PathBuf, String> {
    let path = logs::directory(path)?;
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path))
    }
}

fn execute(cli: Cli) -> Result<(), String> {
    paraco::ai_settings::initialize(&data::Directory::resolve(cli.data_dir.as_deref())?.root);
    match cli.command {
        Command::SandboxCheck => {
            let report = containment::diagnostics::verify_with_canary();
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
            );
            if report["status"] == "PASS" {
                Ok(())
            } else {
                Err("Containment checks did not all pass; see the JSON report".into())
            }
        }
        Command::SandboxProbe { port } => sandbox_probe::run(port),
        Command::HostState { config } => {
            data::Directory::resolve(cli.data_dir.as_deref())?.discover(config.as_deref())
        }
        Command::Check { app, target } => {
            let app = manifest::load(&app).map_err(|e| e.to_string())?;
            app.validate_target(&target).map_err(|e| e.to_string())?;
            println!("{} supports {target}", app.name);
            Ok(())
        }
        Command::ApproveCapabilities { app, state } => {
            let app = manifest::load(&app).map_err(|e| e.to_string())?;
            let mut store = state::Store::open(&state)?;
            let deployment = store.install(&app.root)?;
            store.check_capabilities(&deployment.id, &app, true)?;
            println!(
                "Approved current capabilities for {} ({})",
                app.name, deployment.id
            );
            Ok(())
        }
        Command::Marketplace { command } => marketplace::execute(command),
        Command::Cron {
            port,
            ai_config,
            allow_task_network,
        } => reference::serve(
            port,
            &logs::directory(cli.log_dir.as_deref())?,
            &data::Directory::resolve(cli.data_dir.as_deref())?,
            ai_config.as_deref(),
            allow_task_network,
        ),
        Command::Open { port, print } => control::open(port, print),
        Command::Backup { state, output } => state::Store::open(&state)?.backup(&output),
        Command::Prepare {
            app,
            output,
            workerd,
        } => artifact::prepare(&app, &output, &workerd),
        Command::Service {
            command:
                ServiceCommand::Render {
                    platform,
                    entry,
                    config,
                    state,
                },
        } => {
            let log_dir = service_log_dir(cli.log_dir.as_deref())?;
            let rendered = match platform {
                ServicePlatform::Linux => {
                    service::render_systemd(&entry, &config, &state, &log_dir)
                }
                ServicePlatform::Macos => {
                    service::render_launch_agent(&entry, &config, &state, &log_dir)
                }
            }?;
            print!("{rendered}");
            Ok(())
        }
        Command::Service {
            command: ServiceCommand::Setup { platform, config },
        } => service::operate(
            platform.into(),
            "setup",
            &config,
            &data::Directory::resolve(cli.data_dir.as_deref())?,
            &service_log_dir(cli.log_dir.as_deref())?,
        ),
        Command::Service {
            command: ServiceCommand::Remove { platform, config },
        } => service::operate(
            platform.into(),
            "remove",
            &config,
            &data::Directory::resolve(cli.data_dir.as_deref())?,
            &service_log_dir(cli.log_dir.as_deref())?,
        ),
        Command::Service {
            command: ServiceCommand::Status { platform, config },
        } => service::operate(
            platform.into(),
            "status",
            &config,
            &data::Directory::resolve(cli.data_dir.as_deref())?,
            &service_log_dir(cli.log_dir.as_deref())?,
        ),
        Command::Logs { app, tail, port } => logs::print(
            &logs::directory(cli.log_dir.as_deref())?,
            app.as_deref(),
            port,
            tail,
        ),
        Command::Identity { app } => {
            println!(
                "{}",
                runner::standalone_identity(
                    &app,
                    &logs::directory(cli.log_dir.as_deref())?,
                    &data::Directory::resolve(cli.data_dir.as_deref())?
                )?
            );
            Ok(())
        }
        Command::Status { app, port } => control::execute(port, control::Action::Status, app),
        Command::Start { app, port } => control::execute(port, control::Action::Start, Some(app)),
        Command::Stop { app, port } => control::execute(port, control::Action::Stop, Some(app)),
        Command::Restart { app, port } => {
            control::execute(port, control::Action::Restart, Some(app))
        }
        Command::Serve { config, port } => server::serve(
            &config,
            port,
            &logs::directory(cli.log_dir.as_deref())?,
            &data::Directory::resolve(cli.data_dir.as_deref())?,
        ),
        Command::Run {
            app,
            port,
            ai_config,
        } => runner::run(
            &app,
            port,
            ai_config.as_deref(),
            &logs::directory(cli.log_dir.as_deref())?,
            &data::Directory::resolve(cli.data_dir.as_deref())?,
        ),
        Command::RunPrepared {
            artifact,
            port,
            ai_config,
        } => runner::run_prepared(
            &artifact,
            port,
            ai_config.as_deref(),
            &logs::directory(cli.log_dir.as_deref())?,
            &data::Directory::resolve(cli.data_dir.as_deref())?,
        ),
    }
}
