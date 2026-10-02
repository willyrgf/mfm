{
  description = "MFM";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    nixfied = {
      url = "github:willyrgf/nixfied/fd33e0b1aa90abe5afaf91f0a337a2bf0c531f01";
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
      mkDevTools =
        system:
        let
          pkgs = mkPkgs system;
          rustToolchain = import ./nix/rust-toolchain.nix { inherit pkgs; };
        in
        [
          rustToolchain.package
          pkgs.git
          pkgs.jq
          pkgs.pkg-config
          pkgs.ripgrep
          pkgs.stdenv.cc
          (mkSqlxCli system)
        ]
        ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
          pkgs.bubblewrap
          pkgs.glibc.bin
        ]
        ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = mkPkgs system;
          rustToolchain = import ./nix/rust-toolchain.nix { inherit pkgs; };
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rustToolchain.package;
            rustc = rustToolchain.package;
          };
        in
        {
          default = self.packages.${system}.manifest;
          manifest = nixfied.lib.${system}.compileManifest ./nixfied.nix;
          sqlx-cli = mkSqlxCli system;
          mfm = rustPlatform.buildRustPackage (
            rustToolchain.env
            // {
              pname = "mfm";
              version = "0.1.0";
              src = ./.;
              cargoLock.lockFile = ./Cargo.lock;
              SQLX_OFFLINE = "true";
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
            }
          );
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = mkPkgs system;
          rustToolchain = import ./nix/rust-toolchain.nix { inherit pkgs; };
        in
        {
          default = pkgs.mkShell (
            rustToolchain.env
            // {
              packages = mkDevTools system;
              shellHook = ''
                unset CARGO_TARGET_DIR
              '';
            }
          );
        }
      );

      apps = forAllSystems (
        system:
        let
          projectApps = nixfied.lib.${system}.projectApps ./nixfied.nix;
        in
        # Keep the generated project apps and add the CLI.
        projectApps
        // {
          mfm = {
            type = "app";
            program = "${self.packages.${system}.mfm}/bin/mfm_cli";
            meta.description = "Run the MFM CLI";
          };
        }
      );
    };
}
