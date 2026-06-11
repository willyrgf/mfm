{
  description = "Nixfied project";

  inputs = {
    nixfied.url = "github:willyrgf/nixfied?ref=v2";
    nixpkgs.follows = "nixfied/nixpkgs";
  };

  outputs =
    { self, nixfied, nixpkgs }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-darwin"
        "x86_64-linux"
      ];
      forAllSystems =
        f:
        builtins.listToAttrs (
          map (system: {
            name = system;
            value = f system;
          }) systems
        );
    in
    {
      packages = forAllSystems (system: {
        default = self.packages.${system}.model;
        model = nixfied.lib.${system}.compileModel ./nixfied.nix;
      });

      apps = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          runtimeBin = "${nixfied.packages.${system}.nixfied-runtime}/bin/nixfied-runtime";
          modelJson = "${self.packages.${system}.model}/model.json";
          mkRuntimeApp =
            name: text:
            let
              app = pkgs.writeShellApplication {
                inherit name;
                text = ''
                  if [ -z "''${NIXFIED_STATE_DIR:-}" ]; then
                    model_store="${modelJson}"
                    model_store="''${model_store%/model.json}"
                    model_key="''${model_store##*/}"
                    model_key="''${model_key%-nixfied-model}"

                    state_home="''${XDG_STATE_HOME:-}"
                    if [ -z "$state_home" ]; then
                      if [ -z "''${HOME:-}" ]; then
                        echo "HOME or NIXFIED_STATE_DIR is required" >&2
                        exit 64
                      fi
                      state_home="$HOME/.local/state"
                    fi

                    export NIXFIED_STATE_DIR="$state_home/nixfied/mfm-models/$model_key"
                  fi

                  ${text}
                '';
              };
            in
            {
              type = "app";
              program = "${app}/bin/${name}";
            };
          mkWorkflowApp =
            name: workflow:
            mkRuntimeApp name ''
              "${runtimeBin}" check --model "${modelJson}"
              exec "${runtimeBin}" run --model "${modelJson}" --workflow "${workflow}" "$@"
            '';
        in
        (nixfied.lib.${system}.projectApps ./nixfied.nix)
        // {
          run = mkRuntimeApp "mfm-run" ''exec "${runtimeBin}" run --model "${modelJson}" "$@"'';
          check = mkWorkflowApp "mfm-check" "check";
          test = mkWorkflowApp "mfm-test" "test";
          ci = mkWorkflowApp "mfm-ci" "ci";
        }
      );
    };
}
