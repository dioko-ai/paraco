use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod artifact;
mod capability;
mod control;
mod guardian;
mod logs;
mod manifest;
mod runner;
mod runtime;
mod server;
mod service;
mod state;
mod storage;

#[derive(Parser)]
#[command(name = "paraco", version, about = "Run local Paraco applications")]
struct Cli {
    /// Persistent JSONL log directory (otherwise PARACO_LOG_DIR or ~/.paraco/logs).
    #[arg(long, global = true)]
    log_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Open the authenticated dashboard through the private local control socket.
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
    /// Copy, lock, and cache an application into an immutable offline artifact.
    Prepare {
        /// Source application directory.
        app: PathBuf,
        /// New host-owned artifact directory to create.
        #[arg(long)]
        output: PathBuf,
        /// Explicit Deno executable used to cache and later run this artifact.
        #[arg(long)]
        deno: PathBuf,
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

fn execute(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Open { port, print } => control::open(port, print),
        Command::Backup { state, output } => state::Store::open(&state)?.backup(&output),
        Command::Prepare { app, output, deno } => artifact::prepare(&app, &output, &deno),
        Command::Service {
            command:
                ServiceCommand::Render {
                    platform,
                    entry,
                    config,
                    state,
                },
        } => {
            let rendered = match platform {
                ServicePlatform::Linux => service::render_systemd(&entry, &config, &state),
                ServicePlatform::Macos => service::render_launch_agent(&entry, &config, &state),
            }?;
            print!("{rendered}");
            Ok(())
        }
        Command::Service {
            command: ServiceCommand::Setup { platform, config },
        } => service::operate(platform.into(), "setup", &config),
        Command::Service {
            command: ServiceCommand::Remove { platform, config },
        } => service::operate(platform.into(), "remove", &config),
        Command::Service {
            command: ServiceCommand::Status { platform, config },
        } => service::operate(platform.into(), "status", &config),
        Command::Logs { app, tail, port } => logs::print(
            &logs::directory(cli.log_dir.as_deref())?,
            app.as_deref(),
            port,
            tail,
        ),
        Command::Identity { app } => {
            println!(
                "{}",
                runner::standalone_identity(&app, &logs::directory(cli.log_dir.as_deref())?)?
            );
            Ok(())
        }
        Command::Status { app, port } => control::execute(port, control::Action::Status, app),
        Command::Start { app, port } => control::execute(port, control::Action::Start, Some(app)),
        Command::Stop { app, port } => control::execute(port, control::Action::Stop, Some(app)),
        Command::Restart { app, port } => {
            control::execute(port, control::Action::Restart, Some(app))
        }
        Command::Serve { config, port } => {
            server::serve(&config, port, &logs::directory(cli.log_dir.as_deref())?)
        }
        Command::Run {
            app,
            port,
            ai_config,
        } => runner::run(
            &app,
            port,
            ai_config.as_deref(),
            &logs::directory(cli.log_dir.as_deref())?,
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
        ),
    }
}
