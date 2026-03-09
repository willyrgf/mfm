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
        conf = import ./nixfied/project/conf.nix { inherit pkgs; };
        publishDocsRuntimePath = pkgs.lib.makeBinPath conf.tooling.runtimePackages;
        publishDocsAppScript = ''
          set -euo pipefail

          export PATH="${publishDocsRuntimePath}:$PATH"
          workspace_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
          export MFM_WORKSPACE_ROOT="''${MFM_WORKSPACE_ROOT:-$workspace_root}"
          cd "$workspace_root"

          export CARGO_TARGET_DIR="''${CARGO_TARGET_DIR:-''${TMPDIR:-/tmp}/mfm-task-artifacts/publish-docs-target}"
          export RUSTC_WRAPPER="''${RUSTC_WRAPPER:-sccache}"
          mkdir -p "$CARGO_TARGET_DIR"
        ''
        + pkgs.lib.optionalString pkgs.stdenv.isDarwin ''
          export LIBRARY_PATH="${pkgs.libiconv}/lib"
          export CC="/usr/bin/cc"
          export CXX="/usr/bin/c++"
        ''
        + ''
          cargo build -q -p mfm-publish-docs
          exec "$CARGO_TARGET_DIR/debug/mfm-publish-docs" "$@"
        '';
        publishDocsAppWrapper = pkgs.writeShellScriptBin "mfm-publish-docs-app" publishDocsAppScript;
        publishDocsApp = {
          type = "app";
          program = "${publishDocsAppWrapper}/bin/mfm-publish-docs-app";
        };
      in
      {
        apps = compiled.apps // {
          # Use the fast direct app for the public publish-docs entrypoint.
          publish-docs = publishDocsApp;
        };
        packages = compiled.packages;
        checks = compiled.checks;
        devShells = compiled.devShells;
      }
    );
}
