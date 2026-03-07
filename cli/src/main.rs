mod cli;
mod config;
mod satellite;
mod tle;
mod sim;
mod commands;

use clap::Parser;
use cli::{Cli, Commands};

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run { sim, output, format } => commands::run::run_simulation_cmd(&sim, &output, format),
        Commands::Serve { sim, port } => commands::serve::run_server(&sim, port),
        Commands::Convert { input, format, output } => commands::convert::run_convert(&input, format, output.as_deref()),
    }
}
