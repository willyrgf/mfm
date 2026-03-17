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
        ciAppName = "ci";
        ciAppNameRaw = "mfm-ci-internal";
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

            "${ciAppNameRaw}" = compiled.apps."${ciAppName}";
            "${ciAppName}" = mkShellApp {
              appName = ciAppName;
              body = ''
                set -euo pipefail

                mode_arg_mainnet=0
                saw_mode=0

                for arg in "$@"; do
                  if [ "$saw_mode" = "1" ]; then
                    saw_mode=0
                    if [ "$arg" = "mainnet" ]; then
                      mode_arg_mainnet=1
                    fi
                    continue
                  fi

                  case "$arg" in
                    --mode)
                      saw_mode=1
                      ;;
                    --mode=mainnet)
                      mode_arg_mainnet=1
                      ;;
                  esac
                done

                if [ "$mode_arg_mainnet" = "0" ] && [ -z "''${SKIP_HELIOS+x}" ]; then
                  export SKIP_HELIOS=1
                fi

                exec ${pkgs.nix}/bin/nix \
                  run --impure --accept-flake-config ${toString ./.}#${ciAppNameRaw} -- "$@"
              '';
            };
          };
        packages = compiled.packages;
        checks = compiled.checks;
        devShells = compiled.devShells;
      });
}
