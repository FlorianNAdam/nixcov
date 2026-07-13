{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    git-hooks-nix.url = "github:cachix/git-hooks.nix";
    git-hooks-nix.inputs.nixpkgs.follows = "nixpkgs";
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    naersk.url = "github:nix-community/naersk";
  };

  outputs =
    inputs@{
      flake-parts,
      nixpkgs,
      naersk,
      ...
    }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [
        inputs.git-hooks-nix.flakeModule
      ];

      systems = nixpkgs.lib.systems.flakeExposed;

      perSystem =
        { config, pkgs, ... }:
        let
          cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
          packageName = cargoToml.package.name;
          package = import ./default.nix { inherit pkgs naersk; };
          cargo-lock = pkgs.writeShellApplication {
            name = "cargo-lock";
            runtimeInputs = with pkgs; [
              cargo
              rustc
            ];
            text = ''
              exec cargo generate-lockfile
            '';
          };
          flake-lock = pkgs.writeShellApplication {
            name = "flake-lock";
            runtimeInputs = [ pkgs.nix ];
            text = ''
              exec nix flake lock
            '';
          };
        in
        {
          packages = {
            "${packageName}" = package;
            default = package;
          };

          pre-commit.settings.hooks = {
            "cargo-lock" = {
              enable = true;
              name = "Update Cargo.lock";
              package = cargo-lock;
              entry = "cargo-lock";
              files = "^(Cargo\\.toml|Cargo\\.lock)$";
              pass_filenames = false;
            };
            "flake-lock" = {
              enable = true;
              name = "Update flake.lock";
              package = flake-lock;
              entry = "flake-lock";
              files = "^(flake\\.nix|flake\\.lock)$";
              pass_filenames = false;
            };
            rustfmt.enable = true;
          };

          devShells.default = pkgs.mkShell {
            shellHook = ''
              ${config.pre-commit.shellHook}
            '';

            buildInputs = with pkgs; [
              cargo
              rustc
              rustfmt
            ];

            packages =
              config.pre-commit.settings.enabledPackages
              ++ (with pkgs; [
                rust-analyzer
              ]);
          };
        };
    };
}
