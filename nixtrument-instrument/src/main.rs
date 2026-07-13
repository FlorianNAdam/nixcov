use clap::{Parser, Subcommand};
use nixtrument_lib::{instrument_flake, instrument_path};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Instrument Nix files and write a coverage sidecar JSON file.
    Instrument {
        /// Coverage run id used in trace messages.
        #[arg(long, default_value = "manual")]
        run_id: String,
        /// Flake reference to resolve and instrument, for example `.` or `github:owner/repo`.
        flake_ref: String,
        /// Output directory for the instrumented files.
        output_dir: PathBuf,
        /// JSON sidecar path for coverage ID source mappings.
        sidecar: PathBuf,
    },
    #[command(hide = true)]
    InstrumentSource {
        #[arg(long)]
        run_id: String,
        source: PathBuf,
        output_dir: PathBuf,
        sidecar: PathBuf,
    },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Instrument {
            run_id,
            flake_ref,
            output_dir,
            sidecar,
        } => instrument_flake(&flake_ref, &output_dir, &sidecar, &run_id),
        Command::InstrumentSource {
            run_id,
            source,
            output_dir,
            sidecar,
        } => instrument_path(&source, &output_dir, &sidecar, &run_id),
    }
}
