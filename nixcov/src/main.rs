use anyhow::anyhow;
use clap::{Parser, ValueEnum};
use nixcov_lib::{LcovLineMode, run_coverage};
use std::env;
use std::path::PathBuf;

const INSTRUMENT_BIN_ENV: &str = "NIXCOV_INSTRUMENT_BIN";

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Nix store path to the nixcov-instrument binary used inside the instrumentation derivation.
    #[arg(long)]
    instrument_bin: Option<PathBuf>,
    /// Write LCOV line coverage to this path.
    #[arg(long)]
    lcov: Option<PathBuf>,
    /// How expression hits are projected onto LCOV lines.
    #[arg(long, value_enum, default_value_t = CliLcovLineMode::Strict)]
    lcov_line_mode: CliLcovLineMode,
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

    run_coverage(
        &instrument_bin,
        &cli.flake_ref,
        cli.lcov.as_deref(),
        cli.lcov_line_mode.into(),
    )
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliLcovLineMode {
    /// Mark a line covered if any expression span on it was hit.
    AnyHit,
    /// Mark a line covered only if every expression span on it was hit.
    Strict,
}

impl From<CliLcovLineMode> for LcovLineMode {
    fn from(mode: CliLcovLineMode) -> Self {
        match mode {
            CliLcovLineMode::AnyHit => LcovLineMode::AnyHit,
            CliLcovLineMode::Strict => LcovLineMode::Strict,
        }
    }
}
