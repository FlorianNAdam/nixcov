{
  pkgs ? import <nixpkgs> { },
  naersk,
}:
let
  cargoToml = builtins.fromTOML (builtins.readFile ./nixcov/Cargo.toml);
  naersk-lib = pkgs.callPackage naersk { };
in
naersk-lib.buildPackage {
  pname = cargoToml.package.name;
  version = cargoToml.package.version;
  src = ./.;

  nativeBuildInputs = [ pkgs.makeWrapper ];

  postInstall = ''
    wrapProgram "$out/bin/${cargoToml.package.name}" \
      --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.nix ]} \
      --set NIXCOV_INSTRUMENT_BIN "$out/bin/.nixcov-instrument-wrapped"
    wrapProgram "$out/bin/nixcov-instrument" \
      --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.nix ]}
  '';
}
