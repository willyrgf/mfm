{
  description = "Nixfied vendored wrapper";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    nixfied.url = "path:./nixfied";
  };

  outputs = { self, nixpkgs, flake-utils, nixfied }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        compiled = nixfied.lib.mkNixfied {
          inherit system;
          projectRoot = ./.;
          projectModules = [ ./nixfied/nixfied/project/module.nix ];
          extraModules = [ ];
        };
      in {
        apps = compiled.apps;
        packages = compiled.packages;
        checks = compiled.checks;
        devShells = compiled.devShells;
      });
}

