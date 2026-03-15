{
  description = "Nixfied vendored wrapper";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
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
        compiled = nixfiedLib.mkNixfied {
          projectRoot = ./.;
          projectModules = [ ./nixfied/project/module.nix ];
          extraModules = [ ];
          inherit frameworkSourceRevision;
        };
      in
      {
        apps = compiled.apps;
        packages = compiled.packages;
        checks = compiled.checks;
        devShells = compiled.devShells;
      }
    );
}
