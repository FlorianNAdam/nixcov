# nixcov

A small nix instrumentation framework, mainly for determining coverage.

## Usage

```sh
cargo run -p nixcov-instrument -- instrument <flake-ref> <output-dir> <sidecar-json>
cargo run -p nixcov -- --instrument-bin <nixcov-instrument-bin-store-path> [flake-ref]
nix run .# -- [flake-ref]
```

The instrumenter resolves the flake with `nix flake metadata --json`, uses the
source path reported by Nix, parses each `.nix` file with `rnix`, wraps original
expression ranges with `builtins.trace "NIXCOV:<run-id>:<id>" (...)`, and writes a
JSON sidecar that maps coverage IDs back to file, byte, line/column, and expression
kind. Non-Nix files from the resolved flake source are copied unchanged so
relative paths keep working in the instrumented tree.

`nixcov` resolves the flake source, builds one derivation that runs the given
store-path `nixcov-instrument` binary to produce an instrumented source tree
plus `coverage-map.json`, then runs `nix flake check` on that instrumented
source. The flake reference defaults to `.`. The packaged `nixcov` binary is
wrapped with `NIXCOV_INSTRUMENT_BIN`, so `nix run .# -- [flake-ref]` uses the
matching packaged `nixcov-instrument` automatically.

License: MIT
