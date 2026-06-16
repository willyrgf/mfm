{
  description = "MFM";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    nixfied = {
      url = "github:willyrgf/nixfied";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixfied,
      nixpkgs,
    }:
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
      mkSqlxCli =
        system:
        let
          pkgs = mkPkgs system;
        in
        assert pkgs.sqlx-cli.version == "0.9.0";
        pkgs.sqlx-cli;
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
          sqlx-cli = mkSqlxCli system;
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
