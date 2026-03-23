{
  description = "Nixfied vendored wrapper";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        nixfiedLib = import ./nixfied/framework/core/default.nix {
          inherit
            pkgs
            system
            ;
        };
        frameworkSourceRevision = import ./nixfied/framework/core/framework-revision.nix {
          sourcePath = ./nixfied;
          metadataPath = ./nixfied/VENDORED.txt;
        };
        frameworkOutputs = nixfiedLib.mkFlakeOutputs {
          projectRoot = ./.;
          projectModules = [ ./nixfied/project/module.nix ];
          extraModules = [ ];
          # Module overrides only. ./nixfied/local/default.nix is a legacy
          # extension file and is not loaded by the default flake outputs.
          localOverrides = [ ];
          inherit frameworkSourceRevision;
        };
      in {
        apps = frameworkOutputs.apps;
        packages = frameworkOutputs.packages;
        legacyPackages = frameworkOutputs.legacyPackages;
        checks = frameworkOutputs.checks;
        devShells = frameworkOutputs.devShells;
      });
}

