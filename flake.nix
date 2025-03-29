{
  description = "MFM distributed by Nix.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        
        # Define the Rust package
        mfm_cli = pkgs.rustPlatform.buildRustPackage {
          pname = "mfm_cli";
          version = "0.0.2";
          src = ./.;
          
          # Specify the cargo workspace root if needed
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
          
          nativeBuildInputs = with pkgs; [
            cargo
            rustc
            rustfmt
            pkg-config
          ];
          
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
            cargo
            rustc
            rustfmt
            git
          ];
        };
      });
}
