use anyhow::anyhow;
use clap::{Parser, Subcommand, ValueEnum};
use nixcov_lib::{CoverageCommand, LcovLineMode, run_coverage};
use std::env;
use std::path::PathBuf;

const INSTRUMENT_BIN_ENV: &str = "NIXCOV_INSTRUMENT_BIN";

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Nix store path to the nixcov-instrument binary used inside the instrumentation derivation.
    #[arg(long, global = true)]
    instrument_bin: Option<PathBuf>,
    /// Write LCOV line coverage to this path.
    #[arg(long, global = true)]
    lcov: Option<PathBuf>,
    /// How expression hits are projected onto LCOV lines.
    #[arg(long, value_enum, default_value_t = CliLcovLineMode::Strict, global = true)]
    lcov_line_mode: CliLcovLineMode,
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Evaluate all flake checks with nix build --dry-run.
    Check {
        /// Flake reference to check.
        #[arg(default_value = ".")]
        flake_ref: String,
    },
    /// Run nix flake check on an instrumented flake source.
    FlakeCheck {
        /// Evaluate checks without building them.
        #[arg(long)]
        no_build: bool,
        /// Flake reference to check.
        #[arg(default_value = ".")]
        flake_ref: String,
    },
    /// Run nix build on an instrumented flake installable.
    FlakeBuild {
        /// Show what would be built without realizing build outputs.
        #[arg(long)]
        dry_run: bool,
        /// Flake installable to build.
        installable: String,
    },
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

    let command = match cli.command {
        Some(CliCommand::Check { flake_ref }) => CoverageCommand::check(flake_ref),
        Some(CliCommand::FlakeCheck {
            no_build,
            flake_ref,
        }) => CoverageCommand::flake_check(flake_ref, no_build),
        Some(CliCommand::FlakeBuild {
            dry_run,
            installable,
        }) => CoverageCommand::flake_build(&installable, dry_run),
        None => CoverageCommand::check("."),
    };

    run_coverage(
        &instrument_bin,
        command,
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
