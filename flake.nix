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
        nixfiedLib = import ./nixfied/lib/default.nix {
          inherit
            pkgs
            system
            ;
        };
        compiled = nixfiedLib.mkNixfied {
          projectRoot = ./.;
          projectModules = [ ./nixfied/project/module.nix ];
          extraModules = [ ];
        };
      in {
        apps = compiled.apps;
        packages = compiled.packages;
        checks = compiled.checks;
        devShells = compiled.devShells;
      });
}

