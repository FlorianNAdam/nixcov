use anyhow::anyhow;
use clap::Parser;
use nixtrument_lib::run_coverage;
use std::env;
use std::path::PathBuf;

const INSTRUMENT_BIN_ENV: &str = "NIXTRUMENT_INSTRUMENT_BIN";

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Nix store path to the nixtrument-instrument binary used inside the instrumentation derivation.
    #[arg(long)]
    instrument_bin: Option<PathBuf>,
    /// Flake reference to check.
    #[arg(default_value = ".")]
    flake_ref: String,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let instrument_bin = match cli.instrument_bin {
        Some(instrument_bin) => instrument_bin,
        None => env::var_os(INSTRUMENT_BIN_ENV)
            .map(PathBuf::from)
            .ok_or_else(|| {
                anyhow!("missing instrument binary; pass it explicitly or set {INSTRUMENT_BIN_ENV}")
            })?,
    };

    run_coverage(&instrument_bin, &cli.flake_ref)
}
