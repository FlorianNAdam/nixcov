# nixcov

A small nix instrumentation framework, mainly for determining coverage.

## Usage

```sh
cargo run -p nixcov-instrument -- instrument <flake-ref> <output-dir> <sidecar-json>
cargo run -p nixcov -- --instrument-bin <nixcov-instrument-bin-store-path> check [flake-ref]
cargo run -p nixcov -- --instrument-bin <nixcov-instrument-bin-store-path> flake-check [--no-build] [flake-ref]
cargo run -p nixcov -- --instrument-bin <nixcov-instrument-bin-store-path> flake-build [--dry-run] <installable>
nix run .# -- check [flake-ref]
nix run .# -- flake-check [--no-build] [flake-ref]
nix run .# -- flake-build [--dry-run] <installable>
```

Use `--summary none`, `--summary totals`, or `--summary files` to control
terminal coverage output. The default is `--summary totals`.
Use `--line-mode strict` or `--line-mode any-hit` to control how expression hits
are projected onto line coverage for both terminal summaries and LCOV output.

The instrumenter resolves the flake with `nix flake metadata --json`, uses the
source path reported by Nix, parses each `.nix` file with `rnix`, wraps original
expression ranges with `builtins.trace "NIXCOV:<run-id>:<id>" (...)`, and writes a
JSON sidecar that maps coverage IDs back to file, byte, line/column, and expression
kind. Non-Nix files from the resolved flake source are copied unchanged so
relative paths keep working in the instrumented tree.

`nixcov` resolves the flake source, builds one derivation that runs the given
store-path `nixcov-instrument` binary to produce an instrumented source tree
plus `coverage-map.json`. `check` enumerates flake `checks`, `packages`,
`devShells`, and `apps` for the current system. It runs `nix build --dry-run --no-link` for
checks/packages/devShells and evaluates app `program` paths from the instrumented source.
Use `flake-check` to run `nix flake check` directly, or `flake-build` to run `nix build --no-link` for one installable. For example,
`nix run .#nixcov -- flake-build --dry-run ~/dev/nirion#checks.x86_64-linux.module-sops`
evaluates a single check from the instrumented source. The packaged `nixcov` binary is
wrapped with `NIXCOV_INSTRUMENT_BIN`, so `nix run .# -- ...` uses the matching
packaged `nixcov-instrument` automatically.

License: MIT
