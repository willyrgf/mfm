{
  description = "MFM Nixfied wrapper";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        nixfiedLib = import ./nixfied/lib {
          inherit
            pkgs
            system
            ;
        };
        compiled = nixfiedLib.mkNixfied {
          projectRoot = ./.;
          projectModules = [ ./nixfied/project/module.nix ];
          extraModules = [ ];
          localOverrides = [ ];
        };
      in {
        apps = compiled.apps;
        packages = compiled.packages;
        checks = compiled.checks;
        devShells = compiled.devShells;
      });
}
