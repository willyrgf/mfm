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
        mkShellApp = import ./nixfied/framework/core/mk-shell-app.nix {
          inherit pkgs;
        };
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
        snapshotAppName = "mfm::portfolio::snapshot";
        snapshotAppNameRaw = "mfm-portfolio-snapshot-internal";
      in {
        apps =
          compiled.apps
          // {
            "${snapshotAppNameRaw}" = compiled.apps."${snapshotAppName}";
            "${snapshotAppName}" = mkShellApp {
              appName = snapshotAppName;
              body = ''
                exec ${pkgs.nix}/bin/nix \
                  run --impure --accept-flake-config ${toString ./.}#${snapshotAppNameRaw} -- "$@"
              '';
            };
          };
        packages = compiled.packages;
        checks = compiled.checks;
        devShells = compiled.devShells;
      });
}
