mod cli;
mod commands;
mod config;
mod satellite;
mod sim;
mod tle;

use clap::Parser;
use cli::{Cli, Commands};

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run {
            sim,
            output,
            format,
        } => commands::run::run_simulation_cmd(&sim, &output, format),
        Commands::Serve { sim, port } => commands::serve::run_server(&sim, port),
        Commands::Replay { input, port } => commands::replay::run_replay(&input, port),
        Commands::Convert {
            input,
            format,
            output,
        } => commands::convert::run_convert(&input, format, output.as_deref()),
    }
}
