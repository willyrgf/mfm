{
  lib,
  nixfiedLib,
  pkgs,
  adapters,
  ...
}:
let
  rustToolchain = import ./nix/rust-toolchain.nix { inherit pkgs; };
  cargoTools = [
    "rust-toolchain"
    pkgs.bash
    pkgs.git
    pkgs.pkg-config
    "cc"
  ]
  ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [
    pkgs.bubblewrap
    "ldd"
  ]
  ++ lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];
  ccEnvSuffix = lib.replaceStrings [ "-" ] [ "_" ] pkgs.stdenv.hostPlatform.config;
  cargoEnv = {
    CARGO_TARGET_DIR = "target/verification";
    CARGO_INCREMENTAL = "0";
    CARGO_PROFILE_DEV_DEBUG = "1";
    CARGO_PROFILE_TEST_DEBUG = "1";
    CARGO_PROFILE_DEV_SPLIT_DEBUGINFO = "off";
    CARGO_PROFILE_TEST_SPLIT_DEBUGINFO = "off";
    CARGO_BUILD_JOBS = "2";
    RUST_BACKTRACE = "1";
    TMPDIR = "\${stateDir}";
  }
  // lib.optionalAttrs pkgs.stdenv.hostPlatform.isDarwin {
    "NIX_LDFLAGS_${ccEnvSuffix}" = "-L${pkgs.libiconv}/lib";
    CPATH = "${pkgs.libiconv}/include";
  };
  cargoLeaf =
    {
      run,
      env ? { },
      tools ? [ ],
    }:
    {
      invocation = {
        tools = cargoTools ++ tools;
        run = [
          "bash"
          "-c"
          ''
            set -euo pipefail
            export CARGO_TARGET_DIR="$(pwd -P)/$CARGO_TARGET_DIR"
            exec "$@"
          ''
          "mfm-cargo"
        ]
        ++ run;
        env = cargoEnv // env;
        timeoutMs = 7200000;
      };
    };
  pgLocalPrepare = pkgs.writeShellApplication {
    name = "mfm-pg-local-prepare";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.gnugrep
      pkgs.postgresql
    ];
    text = ''
      state_dir=""
      while [[ $# -gt 0 ]]; do
        case "$1" in
          --state-dir)
            state_dir="''${2:?missing --state-dir value}"
            shift 2
            ;;
          *)
            echo "unknown postgres prepare argument" >&2
            exit 64
            ;;
        esac
      done
      if [[ -z "$state_dir" || "$state_dir" == "/" ]]; then
        echo "missing or unsafe --state-dir argument" >&2
        exit 64
      fi

      pgdata="$state_dir/pgdata"
      if [[ ! -s "$pgdata/PG_VERSION" ]]; then
        rm -rf "$pgdata"
        mkdir -p "$pgdata"
        initdb -D "$pgdata" -U postgres -A trust --no-locale --encoding=UTF8
      fi

      if ! grep -q '^# mfm local postgres fixture$' "$pgdata/postgresql.conf"; then
        {
          printf '%s\n' '# mfm local postgres fixture'
          printf '%s\n' 'ssl = off'
          printf '%s\n' "password_encryption = 'scram-sha-256'"
        } >> "$pgdata/postgresql.conf"
      fi
      {
        printf '%s\n' 'hostnossl all postgres 127.0.0.1/32 trust'
        printf '%s\n' 'hostnossl all mfm_runtime 127.0.0.1/32 scram-sha-256'
        printf '%s\n' 'host all all 127.0.0.1/32 reject'
        printf '%s\n' 'hostnossl all postgres ::1/128 trust'
        printf '%s\n' 'hostnossl all mfm_runtime ::1/128 scram-sha-256'
        printf '%s\n' 'host all all ::1/128 reject'
      } > "$pgdata/pg_hba.conf"
    '';
  };
  localPostgresRun = cargoArgs: ''
    set -euo pipefail
    runtime_password="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
    ambient_password="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
    admin_dsn="host=''${host:postgres} port=''${port:postgres} user=postgres dbname=postgres sslmode=disable"
    psql "$admin_dsn" -v ON_ERROR_STOP=1 >/dev/null <<SQL
    SELECT 'CREATE ROLE mfm_runtime' WHERE NOT EXISTS (
      SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'mfm_runtime'
    ) \gexec
    ALTER ROLE mfm_runtime NOSUPERUSER NOINHERIT NOCREATEROLE NOCREATEDB LOGIN NOREPLICATION NOBYPASSRLS;
    ALTER ROLE mfm_runtime PASSWORD '$runtime_password';
    SQL

    runtime_url="postgresql://mfm_runtime:$runtime_password@''${host:postgres}:''${port:postgres}/postgres?sslmode=disable"
    admin_url="postgresql://postgres:$ambient_password@''${host:postgres}:''${port:postgres}/postgres?sslmode=disable"
    export MFM_TEST_ADMIN_POSTGRES_LOCATOR="$admin_url"
    export MFM_TEST_RUNTIME_POSTGRES_LOCATOR="$runtime_url"
    export PGHOST=192.0.2.1 PGPORT=1 PGUSER=ambient PGDATABASE=ambient
    export PGPASSWORD="$ambient_password" PGPASSFILE="''${stateDir}/absent-pgpass"
    export PGSERVICE=ambient PGSSLMODE=verify-full
    ${cargoArgs}
  '';
  localEvmRun = cargoArgs: ''
    set -euo pipefail
    rpc_url="http://''${host:reth}:''${port:reth}"
    export MFM_TEST_EVM_ADAPTER_LOCATOR="$rpc_url"
    export HTTP_PROXY="http://127.0.0.1:1" HTTPS_PROXY="http://127.0.0.1:1"
    export ALL_PROXY="http://127.0.0.1:1" http_proxy="http://127.0.0.1:1"
    export https_proxy="http://127.0.0.1:1" all_proxy="http://127.0.0.1:1"
    export NO_PROXY="" no_proxy=""
    ${cargoArgs}
  '';
in
{
  imports = [
    adapters.postgres
    adapters.reth
  ];

  nixfied.project.projectId = "mfm";
  nixfied.project.name = "MFM";
  nixfied.codebases.main.logicalRoot = ".";

  nixfied.slotPolicy = {
    min = 0;
    default = 0;
    max = 9;
  };

  nixfied.closures.rust-toolchain = {
    package = rustToolchain;
    executable = "bin/cargo";
    effects = [
      "process"
      "source-read"
      "file-write"
    ];
  };
  nixfied.closures.cc = {
    package = pkgs.stdenv.cc;
    executable = "bin/cc";
    effects = [ "process" ];
  };
  nixfied.closures.pg-local-prepare = {
    package = pgLocalPrepare;
    executable = "bin/mfm-pg-local-prepare";
    effects = [
      "process"
      "file-write"
    ];
  };
  nixfied.tasks.pg-init = lib.mkForce {
    invocation = {
      tools = [ "pg-local-prepare" ];
      run = [
        "mfm-pg-local-prepare"
        "--state-dir"
        "\${stateDir}"
      ];
      timeoutMs = 60000;
    };
  };
  nixfied.closures.ldd = lib.mkIf pkgs.stdenv.hostPlatform.isLinux {
    package = pkgs.glibc.bin;
    executable = "bin/ldd";
    effects = [ "process" ];
  };

  nixfied.tasks = {
    fmt = cargoLeaf {
      run = [
        "cargo"
        "fmt"
        "--all"
        "--"
        "--check"
      ];
    };
    clippy = cargoLeaf {
      run = [
        "cargo"
        "clippy"
        "--workspace"
        "--all-targets"
        "--all-features"
        "--"
        "-D"
        "warnings"
      ];
    };
    cargo-check = cargoLeaf {
      run = [
        "cargo"
        "check"
        "--workspace"
        "--all-targets"
      ];
    };
    cargo-test = cargoLeaf {
      run = [
        "cargo"
        "test"
        "--workspace"
        "--all-targets"
      ];
    };
    postgres-test =
      (cargoLeaf {
        # Every ignored test owns the whole managed database, so they must not overlap.
        run = [
          "bash"
          "-c"
          (localPostgresRun ''
            env MFM_TEST_BLOCKED_POSTGRES_ENV=PGOPTIONS PGOPTIONS=mfm-rejected \
              cargo test -p mfm-storage-postgres --lib \
                tests::managed_postgres_rejects_pgoptions -- \
                --include-ignored --exact --test-threads=1
            exec cargo test -p mfm-storage-postgres --lib -- --include-ignored --test-threads=1
          '')
        ];
        tools = [
          "pg-psql"
          pkgs.coreutils
        ];
      })
      // {
        requires = [ "postgres" ];
      };
    test-db = {
      kind = "composite";
      steps = nixfiedLib.seq [ "postgres-test" ];
    };
    client-e2e =
      (cargoLeaf {
        run = [
          "bash"
          "-c"
          (localPostgresRun (localEvmRun ''
            env -u PGSERVICE -u PGHOST -u PGPORT -u PGUSER -u PGDATABASE \
              -u PGPASSWORD -u PGPASSFILE \
              psql "$admin_dsn" -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
            DROP SCHEMA IF EXISTS mfm_config CASCADE;
            DROP SCHEMA IF EXISTS public CASCADE;
            CREATE SCHEMA public AUTHORIZATION CURRENT_USER;
            SQL
            export MFM_E2E_ADMIN_POSTGRES_LOCATOR="$MFM_TEST_ADMIN_POSTGRES_LOCATOR"
            export MFM_E2E_RUNTIME_POSTGRES_LOCATOR="$MFM_TEST_RUNTIME_POSTGRES_LOCATOR"
            export MFM_E2E_EVM_ADAPTER_LOCATOR="$MFM_TEST_EVM_ADAPTER_LOCATOR"
            cargo build -p mfm -p mfm-rest-api --bins
            export MFM_E2E_CLI_BIN="$CARGO_TARGET_DIR/debug/mfm_cli"
            export MFM_E2E_REST_BIN="$CARGO_TARGET_DIR/debug/mfm_rest_api"
            test -x "$MFM_E2E_CLI_BIN" -a -x "$MFM_E2E_REST_BIN"
            exec cargo test -p mfm-rest-api --test client_execution_e2e -- \
              --include-ignored --test-threads=1
          ''))
        ];
        tools = [
          "pg-psql"
          pkgs.coreutils
        ];
      })
      // {
        requires = [
          "postgres"
          "reth"
        ];
      };
    doc-tests = cargoLeaf {
      run = [
        "cargo"
        "test"
        "--workspace"
        "--doc"
      ];
    };
    capacity-app = cargoLeaf {
      run = [
        "cargo"
        "test"
        "-p"
        "mfm-evm"
        "-p"
        "mfm-portfolio"
        "-p"
        "mfm-app"
        "--all-targets"
        "--"
        "--nocapture"
      ];
    };
    capacity-runtime = cargoLeaf {
      run = [
        "cargo"
        "test"
        "-p"
        "mfm-runtime"
        "--test"
        "runtime_contract"
        "--"
        "--nocapture"
      ];
    };
    capacity-store = cargoLeaf {
      run = [
        "cargo"
        "test"
        "-p"
        "mfm-store"
        "-p"
        "mfm-journal"
        "--all-targets"
        "--"
        "--nocapture"
      ];
    };
    capacity-envelope = {
      kind = "composite";
      steps = nixfiedLib.seq [
        "capacity-app"
        "capacity-runtime"
        "capacity-store"
      ];
    };
    ci = {
      kind = "composite";
      steps = nixfiedLib.seq [
        "fmt"
        "clippy"
        "cargo-check"
        "cargo-test"
        "doc-tests"
        "capacity-envelope"
        "test-db"
        "client-e2e"
      ];
    };
  };

  nixfied.surface.verbs = [ "ci" ];
}
