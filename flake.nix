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
          devTools = mkDevTools system;
          projectApps = nixfied.lib.${system}.projectApps ./nixfied.nix;
          dependencyArtifactPilot = import ./nix/dependency-artifact.nix {
            lib = pkgs.lib;
            inherit pkgs rustPlatform;
            src = ./.;
          };
        in
        {
          default = self.packages.${system}.model;
          model = nixfied.lib.${system}.compileModel ./nixfied.nix;
          sqlx-cli = mkSqlxCli system;
          dependency-artifact = dependencyArtifactPilot.dependencyArtifact;
          dependency-artifact-source-tests = dependencyArtifactPilot.sourceInputTests;
          quick = pkgs.writeShellApplication {
            name = "mfm-quick";
            runtimeInputs = devTools;
            text = ''
              unset CARGO_TARGET_DIR
              cargo fmt --all -- --check
              cargo check --workspace --lib --bins
            '';
          };
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
          mfm-start = pkgs.writeShellApplication {
            name = "mfm-start";
            runtimeInputs = [
              self.packages.${system}.mfm
              pkgs.jq
            ];
            text = ''
              if [[ $# -eq 0 ]]; then
                exec mfm run start --help
              fi
              case "''${1:-}" in
                -h|--help)
                  exec mfm run start "$@"
                  ;;
              esac

              slot=9
              nixfied_run="${projectApps.run.program}"
              nixfied_down="${projectApps.down.program}"

              store_output="$("$nixfied_run" --task mfm-start-store --slot "$slot" --json)"
              host="$(
                jq -r '.services[] | select(.serviceId == "postgres") | .selectedEndpoint.host' \
                  <<<"$store_output" \
                  | tail -n 1
              )"
              port="$(
                jq -r '.services[] | select(.serviceId == "postgres") | .selectedEndpoint.port' \
                  <<<"$store_output" \
                  | tail -n 1
              )"
              if [[ -z "$host" || -z "$port" || "$host" == "null" || "$port" == "null" ]]; then
                printf '%s\n' "mfm-start could not resolve the managed Postgres endpoint" >&2
                printf '%s\n' "$store_output" >&2
                exit 1
              fi

              # shellcheck disable=SC2329
              cleanup() {
                "$nixfied_down" --slot "$slot" >/dev/null || true
              }
              trap cleanup EXIT

              export DATABASE_URL="postgresql://postgres@$host:$port/postgres"

              set +e
              mfm run start "$@"
              status=$?
              set -e
              exit "$status"
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
        # The verification surface is generated: MFM's own task names become
        # the verbs (`.#check`/`.#test`/`.#ci` via nixfied.surface.verbs),
        # model admission lives at `.#model-check`. The only override is MFM's own binary.
        (nixfied.lib.${system}.projectApps ./nixfied.nix)
        // {
          mfm = {
            type = "app";
            program = "${self.packages.${system}.mfm}/bin/mfm";
          };
          mfm-start = {
            type = "app";
            program = "${self.packages.${system}.mfm-start}/bin/mfm-start";
          };
          quick = {
            type = "app";
            program = "${self.packages.${system}.quick}/bin/mfm-quick";
          };
        }
      );
    };
}
