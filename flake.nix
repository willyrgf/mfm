{
  description = "A flake for the MFM CLI";

  inputs = {
    rust-overlay.url = "github:oxalica/rust-overlay";
    nixpkgs.url      = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url  = "github:numtide/flake-utils";
  };

  outputs = { nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        }; 

        packages = pkgs.callPackage ./. {};
      in
      {
        packages = {          
          default = packages.mfm;
        };
      }
    );
}
