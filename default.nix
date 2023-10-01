{ pkgs }:
let
  common = {
    version = "0.1.29";
    src = ./.;

    cargoLock = {
      lockFile = ./Cargo.lock;
    };

    buildInputs = with pkgs;[
      darwin.apple_sdk.frameworks.Security
      pkg-config
      openssl
    ];
  };
in {
  mfm = pkgs.rustPlatform.buildRustPackage (common // {
    pname = "mfm";
    meta = {
          description = "An Nix module for MFM CLI";
          license = pkgs.lib.licenses.mit;
          homepage = "https://github.com/willyrgf/mfm";
    };
  });
}
