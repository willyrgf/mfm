{
  frameworkInput ? "github:willyrgf/nixfied",
  vendorPath ? null,
}:
if vendorPath == null then
  ''
    {
      description = "Nixfied thin wrapper";

      inputs = {
        nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
        flake-utils.url = "github:numtide/flake-utils";
        nixfied.url = "${frameworkInput}";
      };

      outputs = { self, nixpkgs, flake-utils, nixfied }:
        flake-utils.lib.eachDefaultSystem (system:
          let
            compiled = nixfied.lib.mkNixfied {
              inherit system;
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
  ''
else
  ''
    {
      description = "Nixfied vendored wrapper";

      inputs = {
        nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
        flake-utils.url = "github:numtide/flake-utils";
      };

      outputs = { self, nixpkgs, flake-utils }:
        flake-utils.lib.eachDefaultSystem (system:
          let
            pkgs = import nixpkgs { inherit system; };
            nixfiedLib = import ${vendorPath}/lib {
              inherit
                pkgs
                system
                ;
            };
            compiled = nixfiedLib.mkNixfied {
              projectRoot = ./.;
              projectModules = [ ${vendorPath}/project/module.nix ];
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
  ''
