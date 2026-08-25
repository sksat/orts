mod cli;
mod commands;
mod config;
mod license;
mod satellite;
mod sim;
mod tle;

use cli::{Cli, Commands};
use commands::CmdError;
use notalawyer_clap::ParseExt;

/// `main` is the only place that ends the process: commands return `CmdError`
/// (which carries the exit code) rather than calling `std::process::exit` from
/// wherever the failure happened to be detected.
fn exit_on_error(result: Result<(), CmdError>) {
    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(e.code);
    }
}

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
            json,
        } => exit_on_error(commands::run::run_simulation_cmd(
            &sim,
            output.as_deref(),
            format,
            json,
        )),
        Commands::Serve {
            sim,
            port,
            stream_stdio,
        } => exit_on_error(commands::serve::run_server(
            &sim,
            port,
            stream_stdio.as_deref(),
        )),
        Commands::Replay { input, port } => commands::replay::run_replay(&input, port),
        Commands::Convert {
            input,
            format,
            output,
        } => commands::convert::run_convert(&input, format, output.as_deref()),
        Commands::Config { command } => commands::config::run_config(command),
    }
}
