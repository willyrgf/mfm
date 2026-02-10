{
  description = "MFM distributed by Nix.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        # Use rust-overlay toolchains so Nix builds don't get stuck on an older
        # nixpkgs rustc that can't compile newer transitive deps.
        stableToolchain = pkgs.rust-bin.stable.latest.default;

        nightlyToolchain = pkgs.rust-bin.nightly.latest.default.override {
          extensions = [
            "rustfmt"
            "clippy"
            "rust-src"
          ];
        };

        rustPlatform = pkgs.makeRustPlatform {
          cargo = stableToolchain;
          rustc = stableToolchain;
        };

        # Define the Rust package
        mfm_cli = rustPlatform.buildRustPackage {
          pname = "mfm_cli";
          version = "0.0.2";
          src = ./.;

          # Specify the cargo workspace root if needed
          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = with pkgs; [ pkg-config ];

          buildInputs = with pkgs; [
            git
            openssl
          ];

          # Skip tests if you don't want them
          doCheck = false;
        };
      in
      {
        packages.mfm_cli = mfm_cli;

        defaultPackage = self.packages.${system}.mfm_cli;

        apps.default = {
          type = "app";
          program = "${self.packages.${system}.mfm_cli}/bin/mfm_cli";
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            nightlyToolchain
            cargo-nextest
            git
            pkg-config
            openssl
          ];
        };
      }
    );
}
