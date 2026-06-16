{
  description = "MFM";

  inputs = {
    nixfied.url = "github:willyrgf/nixfied";
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
          rustToolchain = import ./nix/rust-toolchain.nix { inherit pkgs; };
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
        # The verification surface is generated: MFM's own task names become
        # the verbs (`.#check`/`.#test`/`.#ci` via nixfied.surface.verbs),
        # model admission lives at `.#model-check`. The only override is MFM's own binary.
        (nixfied.lib.${system}.projectApps ./nixfied.nix)
        // {
          mfm = {
            type = "app";
            program = "${self.packages.${system}.mfm}/bin/mfm";
          };
        }
      );
    };
}
