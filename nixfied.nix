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
  pgSecurePrepare = pkgs.writeShellApplication {
    name = "mfm-pg-secure-prepare";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.gnugrep
      pkgs.openssl
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

      tls_dir="$state_dir/postgres-tls"
      if [[ ! -s "$tls_dir/ca.pem" || ! -s "$tls_dir/server.pem" || ! -s "$tls_dir/server.key" || ! -s "$tls_dir/alternate-ca.pem" ]]; then
        rm -rf "$tls_dir"
        mkdir -p "$tls_dir"
        chmod 700 "$tls_dir"
        openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
          -subj "/CN=mfm-postgres-test-ca" \
          -keyout "$tls_dir/ca.key" -out "$tls_dir/ca.pem" >/dev/null 2>&1
        openssl req -newkey rsa:2048 -nodes \
          -subj "/CN=127.0.0.1" \
          -keyout "$tls_dir/server.key" -out "$tls_dir/server.csr" >/dev/null 2>&1
        printf '%s\n' 'subjectAltName=IP:127.0.0.1' > "$tls_dir/server.ext"
        openssl x509 -req -days 3650 -sha256 \
          -in "$tls_dir/server.csr" -CA "$tls_dir/ca.pem" -CAkey "$tls_dir/ca.key" \
          -CAcreateserial -extfile "$tls_dir/server.ext" -out "$tls_dir/server.pem" \
          >/dev/null 2>&1
        openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
          -subj "/CN=mfm-postgres-alternate-ca" \
          -keyout "$tls_dir/alternate-ca.key" -out "$tls_dir/alternate-ca.pem" \
          >/dev/null 2>&1
        chmod 600 "$tls_dir/server.key" "$tls_dir/ca.key" "$tls_dir/alternate-ca.key"
      fi

      if ! grep -q '^# mfm secure postgres fixture$' "$pgdata/postgresql.conf"; then
        {
          printf '%s\n' '# mfm secure postgres fixture'
          printf "ssl = on\nssl_cert_file = '%s'\nssl_key_file = '%s'\n" \
            "$tls_dir/server.pem" "$tls_dir/server.key"
          printf '%s\n' "password_encryption = 'scram-sha-256'"
        } >> "$pgdata/postgresql.conf"
      fi
      {
        printf '%s\n' 'hostssl all postgres 127.0.0.1/32 trust'
        printf '%s\n' 'hostssl all mfm_runtime 127.0.0.1/32 scram-sha-256'
        printf '%s\n' 'host all all 127.0.0.1/32 reject'
      } > "$pgdata/pg_hba.conf"
    '';
  };
  securePostgresRun = cargoArgs: ''
    set -euo pipefail
    tls_dir="''${stateDir}/postgres-tls"
    ca_path="$tls_dir/ca.pem"
    alternate_ca_path="$tls_dir/alternate-ca.pem"
    ca_hex="$(sha256sum "$ca_path" | cut -d ' ' -f 1)"
    alternate_ca_hex="$(sha256sum "$alternate_ca_path" | cut -d ' ' -f 1)"
    rm -f "$tls_dir/ca.key" "$tls_dir/ca.srl" "$tls_dir/server.key" \
      "$tls_dir/server.csr" "$tls_dir/alternate-ca.key"
    runtime_password="$(openssl rand -hex 32)"
    ambient_password="$(openssl rand -hex 32)"
    admin_dsn="host=''${host:postgres} port=''${port:postgres} user=postgres dbname=postgres sslmode=verify-full sslrootcert=$ca_path"
    psql "$admin_dsn" -v ON_ERROR_STOP=1 >/dev/null <<SQL
    SELECT 'CREATE ROLE mfm_runtime' WHERE NOT EXISTS (
      SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'mfm_runtime'
    ) \gexec
    ALTER ROLE mfm_runtime NOSUPERUSER NOINHERIT NOCREATEROLE NOCREATEDB LOGIN NOREPLICATION NOBYPASSRLS;
    ALTER ROLE mfm_runtime PASSWORD '$runtime_password';
    SQL

    runtime_url="postgresql://mfm_runtime:$runtime_password@''${host:postgres}:''${port:postgres}/postgres?sslmode=verify-full"
    admin_url="postgresql://postgres:$ambient_password@''${host:postgres}:''${port:postgres}/postgres?sslmode=verify-full"
    wrong_host_url="postgresql://mfm_runtime:$runtime_password@localhost:''${port:postgres}/postgres?sslmode=verify-full"
    root_spec="{\"kind\":\"pem-file\",\"path\":\"$ca_path\",\"digest\":\"content:sha256-v1:$ca_hex\"}"
    alternate_spec="{\"kind\":\"pem-file\",\"path\":\"$alternate_ca_path\",\"digest\":\"content:sha256-v1:$alternate_ca_hex\"}"
    wrong_pin_spec="{\"kind\":\"pem-file\",\"path\":\"$ca_path\",\"digest\":\"content:sha256-v1:0000000000000000000000000000000000000000000000000000000000000000\"}"
    export MFM_TEST_ADMIN_STORE_LOCATOR="{\"v\":1,\"url\":\"$admin_url\",\"tls_roots\":$root_spec}"
    export MFM_TEST_RUNTIME_STORE_LOCATOR="{\"v\":1,\"url\":\"$runtime_url\",\"tls_roots\":$root_spec}"
    export MFM_TEST_WRONG_PIN_STORE_LOCATOR="{\"v\":1,\"url\":\"$runtime_url\",\"tls_roots\":$wrong_pin_spec}"
    export MFM_TEST_ALTERNATE_CA_STORE_LOCATOR="{\"v\":1,\"url\":\"$runtime_url\",\"tls_roots\":$alternate_spec}"
    export MFM_TEST_WRONG_HOST_STORE_LOCATOR="{\"v\":1,\"url\":\"$wrong_host_url\",\"tls_roots\":$root_spec}"
    export PGHOST=192.0.2.1 PGPORT=1 PGUSER=ambient PGDATABASE=ambient
    export PGPASSWORD="$ambient_password" PGPASSFILE="''${stateDir}/absent-pgpass"
    export PGSERVICE=ambient SSL_CERT_FILE="$alternate_ca_path"
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
  nixfied.closures.pg-secure-prepare = {
    package = pgSecurePrepare;
    executable = "bin/mfm-pg-secure-prepare";
    effects = [
      "process"
      "file-write"
    ];
  };

  nixfied.tasks.pg-init = lib.mkForce {
    invocation = {
      tools = [ "pg-secure-prepare" ];
      run = [
        "mfm-pg-secure-prepare"
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
          (securePostgresRun ''
            exec cargo test -p mfm-storage-postgres --lib -- --include-ignored --test-threads=1
          '')
        ];
        tools = [
          "pg-psql"
          pkgs.coreutils
          pkgs.openssl
        ];
      })
      // {
        requires = [ "postgres" ];
      };
    test-db = {
      kind = "composite";
      steps = nixfiedLib.seq [ "postgres-test" ];
    };
    cli-e2e =
      (cargoLeaf {
        run = [
          "bash"
          "-c"
          (securePostgresRun ''
            env -u PGSERVICE -u PGHOST -u PGPORT -u PGUSER -u PGDATABASE \
              -u PGPASSWORD -u PGPASSFILE \
              psql "$admin_dsn" -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
            DROP SCHEMA IF EXISTS mfm_catalog CASCADE;
            DROP SCHEMA IF EXISTS public CASCADE;
            CREATE SCHEMA public AUTHORIZATION CURRENT_USER;
            SQL
            export MFM_E2E_ADMIN_STORE_LOCATOR="$MFM_TEST_ADMIN_STORE_LOCATOR"
            export MFM_E2E_RUNTIME_STORE_LOCATOR="$MFM_TEST_RUNTIME_STORE_LOCATOR"
            exec cargo test -p mfm --test cli_e2e -- --include-ignored --test-threads=1
          '')
        ];
        env = {
          MFM_E2E_RPC_URL = "http://\${host:reth}:\${port:reth}";
        };
        tools = [
          "pg-psql"
          pkgs.coreutils
          pkgs.openssl
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
        "cli-e2e"
      ];
    };
  };

  nixfied.surface.verbs = [ "ci" ];
}
