# nixtrument

A small nix instrumentation framework, mainly for determining coverage.

## Usage

```sh
cargo run -- instrument <flake-ref> <output-dir> <sidecar-json>
```

The instrumenter resolves the flake with `nix flake metadata --json`, uses the
source path reported by Nix, parses each `.nix` file with `rnix`, wraps original
expression ranges with `builtins.trace "NIXCOV:<id>" (...)`, and writes a JSON
sidecar that maps coverage IDs back to file, byte, line/column, and expression
kind. Non-Nix files from the resolved flake source are copied unchanged so
relative paths keep working in the instrumented tree.

License: MIT
