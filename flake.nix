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
          mkWorkflowApp =
            name: workflow:
            let
              app = pkgs.writeShellApplication {
                inherit name;
                text = ''
                  "${runtimeBin}" check --model "${modelJson}"
                  exec "${runtimeBin}" run --model "${modelJson}" --workflow "${workflow}" "$@"
                '';
              };
            in
            {
              type = "app";
              program = "${app}/bin/${name}";
            };
        in
        (nixfied.lib.${system}.projectApps ./nixfied.nix)
        // {
          check = mkWorkflowApp "mfm-check" "check";
          test = mkWorkflowApp "mfm-test" "test";
          ci = mkWorkflowApp "mfm-ci" "ci";
        }
      );
    };
}
