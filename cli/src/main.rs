mod cli;
mod commands;
mod config;
mod license;
mod satellite;
mod sim;
mod tle;

use cli::{Cli, Commands};
use notalawyer_clap::ParseExt;

fn main() {
    // Concatenate the Rust and viewer (npm) NOTICE strings so a single
    // `--license-notice` invocation prints everything that is redistributed
    // in the binary. Built at runtime so the viewer notice can come from the
    // rust-embed asset store (feature = "viewer").
    let notice = license::combined_notice();
    let cli = Cli::parse_with_license_notice(&notice);
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
