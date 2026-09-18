use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod capability;
mod control;
mod logs;
mod manifest;
mod runner;
mod server;

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
        /// Host-owned fake AI routing and grants configuration.
        #[arg(long)]
        ai_config: Option<PathBuf>,
    },
}

fn main() {
    let cli = Cli::parse();
    let result = execute(cli);
    if let Err(error) = result {
        eprintln!("paraco: {error}");
        std::process::exit(1);
    }
}

fn execute(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Logs { app, tail, port } => logs::print(
            &logs::directory(cli.log_dir.as_deref())?,
            app.as_deref(),
            port,
            tail,
        ),
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
    }
}
