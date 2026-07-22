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
          rustToolchain
          pkgs.cargo-nextest
          pkgs.git
          pkgs.pkg-config
          pkgs.stdenv.cc
          (mkSqlxCli system)
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

      devShells = forAllSystems (
        system:
        let
          pkgs = mkPkgs system;
        in
        {
          default = pkgs.mkShell {
            packages = mkDevTools system;
            shellHook = ''
              unset CARGO_TARGET_DIR
            '';
          };
        }
      );

      apps = forAllSystems (
        system:
        let
          pkgs = mkPkgs system;
          projectApps = nixfied.lib.${system}.projectApps ./nixfied.nix;
          managedMfm = pkgs.writeShellApplication {
            name = "mfm";
            runtimeInputs = [
              self.packages.${system}.mfm
              pkgs.jq
            ];
            text = ''
              slot=9
              nixfied_run="${projectApps.run.program}"
              nixfied_down="${projectApps.down.program}"

              # shellcheck disable=SC2329
              cleanup() {
                "$nixfied_down" --slot "$slot" >/dev/null || true
              }
              trap cleanup EXIT

              store_output="$("$nixfied_run" --task mfm-store --slot "$slot" --json)"
              endpoint="$(
                jq -r \
                  '[.services[] | select(.serviceId == "postgres")][-1].selectedEndpoint | [.host, .port] | @tsv' \
                  <<<"$store_output"
              )"
              IFS=$'\t' read -r host port <<<"$endpoint"
              if [[ -z "$host" || -z "$port" || "$host" == "null" || "$port" == "null" ]]; then
                printf '%s\n' "mfm could not resolve the managed Postgres endpoint" >&2
                printf '%s\n' "$store_output" >&2
                exit 1
              fi

              export DATABASE_URL="postgresql://postgres@$host:$port/postgres"
              mfm "$@"
            '';
          };
        in
        # Keep the generated project apps and add the managed CLI.
        projectApps
        // {
          check = projectApps.check // {
            meta.description = "Run formatting, Clippy, metadata, and offline SQLx checks";
          };
          test = projectApps.test // {
            meta.description = "Run service-free workspace Nextest and doctests";
          };
          test-db = projectApps.test-db // {
            meta.description = "Run managed PostgreSQL SQLx and parity verification";
          };
          ci = projectApps.ci // {
            meta.description = "Run the complete MFM verification graph";
          };
          mfm = {
            type = "app";
            program = "${managedMfm}/bin/mfm";
            meta.description = "Run the MFM CLI with managed PostgreSQL";
          };
        }
      );
    };
}
