{
  frameworkInput ? "github:willyrgf/nixfied/dev",
  vendorPath ? null,
}:
if vendorPath == null then
  ''
    {
      description = "Nixfied thin wrapper";

      inputs = {
        nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
        flake-utils.url = "github:numtide/flake-utils";
        nixfied.url = "${frameworkInput}";
      };

      outputs = { self, nixpkgs, flake-utils, nixfied }:
        flake-utils.lib.eachDefaultSystem (system:
          let
            frameworkSourceRevision =
              let
                dirtyRev = if nixfied ? dirtyRev then nixfied.dirtyRev else null;
                rev = if nixfied ? rev then nixfied.rev else null;
                fallbackRevision = builtins.substring 0 12 (
                  builtins.hashString "sha256" (builtins.toString nixfied.outPath)
                );
              in
              if dirtyRev != null then
                dirtyRev
              else if rev != null then
                rev
              else
                fallbackRevision;
            frameworkOutputs = nixfied.lib.mkFlakeOutputs {
              inherit system;
              projectRoot = ./.;
              projectModules = [ ./nixfied/project/module.nix ];
              extraModules = [ ];
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
  ''
else
  ''
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
            nixfiedLib = import ${vendorPath}/framework/core/default.nix {
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
  ''
