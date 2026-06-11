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
      mkPkgs =
        system:
        import nixpkgs {
          inherit system;
          overlays = [ nixfied.inputs.rust-overlay.overlays.default ];
        };
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = mkPkgs system;
          rustToolchain = pkgs.rust-bin.stable."1.96.0".minimal;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rustToolchain;
            rustc = rustToolchain;
          };
        in
        {
          default = self.packages.${system}.model;
          model = nixfied.lib.${system}.compileModel ./nixfied.nix;
          mfm = rustPlatform.buildRustPackage {
            pname = "mfm";
            version = "0.1.29";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [
              "-p"
              "mfm"
              "--bin"
              "mfm_cli"
            ];
            doCheck = false;
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
              pkgs.libiconv
            ];
            postInstall = ''
              ln -s "$out/bin/mfm_cli" "$out/bin/mfm"
            '';
          };
        }
      );

      apps = forAllSystems (
        system:
        let
          pkgs = mkPkgs system;
          runtimeBin = "${nixfied.packages.${system}.nixfied-runtime}/bin/nixfied-runtime";
          modelJson = "${self.packages.${system}.model}/model.json";
          # MFM's verification surface selects project workflows: `check` is the
          # source gate (fmt/clippy/contracts), `ci` the full service-backed
          # gate. Everything else (run/test/ps/down/clean and default state
          # placement) is the generated nixfied surface.
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
          mfmApp =
            {
              type = "app";
              program = "${self.packages.${system}.mfm}/bin/mfm";
            };
        in
        (nixfied.lib.${system}.projectApps ./nixfied.nix)
        // {
          check = mkWorkflowApp "mfm-check" "check";
          ci = mkWorkflowApp "mfm-ci" "ci";
          mfm = mfmApp;
        }
      );
    };
}
