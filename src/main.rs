use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod manifest;
mod runner;

#[derive(Parser)]
#[command(name = "paraco", version, about = "Run local Paraco applications")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run an application directory containing paraco.json.
    Run {
        /// Application directory.
        app: PathBuf,
        /// Loopback TCP port for the application.
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Run { app, port } => runner::run(&app, port),
    };

    if let Err(error) = result {
        eprintln!("paraco: {error}");
        std::process::exit(1);
    }
}
