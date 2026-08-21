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
    export MFM_TEST_ADMIN_STORE_LOCATOR="{\"v\":1,\"url\":\"$admin_url\"}"
    export MFM_TEST_RUNTIME_STORE_LOCATOR="{\"v\":1,\"url\":\"$runtime_url\"}"
    export PGHOST=192.0.2.1 PGPORT=1 PGUSER=ambient PGDATABASE=ambient
    export PGPASSWORD="$ambient_password" PGPASSFILE="''${stateDir}/absent-pgpass"
    export PGSERVICE=ambient PGSSLMODE=verify-full
    ${cargoArgs}
  '';
  evmTlsPrepare = pkgs.writeShellApplication {
    name = "mfm-evm-tls-prepare";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.openssl
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
            echo "unknown evm tls prepare argument" >&2
            exit 64
            ;;
        esac
      done
      if [[ -z "$state_dir" || "$state_dir" == "/" ]]; then
        echo "missing or unsafe --state-dir argument" >&2
        exit 64
      fi
      tls_dir="$state_dir/evm-tls"
      if [[ ! -s "$tls_dir/ca.pem" || ! -s "$tls_dir/server.pem" || ! -s "$tls_dir/server.key" || ! -s "$tls_dir/alternate-ca.pem" ]]; then
        rm -rf "$tls_dir"
        mkdir -p "$tls_dir"
        chmod 700 "$tls_dir"
        openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
          -subj "/CN=mfm-evm-test-ca" \
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
          -subj "/CN=mfm-evm-alternate-ca" \
          -keyout "$tls_dir/alternate-ca.key" -out "$tls_dir/alternate-ca.pem" \
          >/dev/null 2>&1
        chmod 600 "$tls_dir/server.key" "$tls_dir/ca.key" "$tls_dir/alternate-ca.key"
      fi
    '';
  };
  evmTlsProxy = pkgs.writeShellApplication {
    name = "mfm-evm-tls-proxy";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.nginx
    ];
    text = ''
      state_dir="" listen_host="" listen_port="" upstream_host="" upstream_port=""
      while [[ $# -gt 0 ]]; do
        case "$1" in
          --state-dir) state_dir="''${2:?missing value}"; shift 2 ;;
          --listen-host) listen_host="''${2:?missing value}"; shift 2 ;;
          --listen-port) listen_port="''${2:?missing value}"; shift 2 ;;
          --upstream-host) upstream_host="''${2:?missing value}"; shift 2 ;;
          --upstream-port) upstream_port="''${2:?missing value}"; shift 2 ;;
          *) echo "unknown evm tls proxy argument" >&2; exit 64 ;;
        esac
      done
      if [[ -z "$state_dir" || "$state_dir" == "/" || -z "$listen_host" || -z "$listen_port" || -z "$upstream_host" || -z "$upstream_port" ]]; then
        echo "missing or unsafe evm tls proxy argument" >&2
        exit 64
      fi
      tls_dir="$state_dir/evm-tls"
      config="$tls_dir/nginx.conf"
      {
        printf '%s\n' 'daemon off;'
        printf 'pid %s;\n' "$tls_dir/nginx.pid"
        printf '%s\n' 'error_log stderr warn;' 'events {}' 'http {' '  access_log off;' '  server {'
        printf '    listen %s:%s ssl;\n' "$listen_host" "$listen_port"
        printf '    ssl_certificate %s;\n' "$tls_dir/server.pem"
        printf '    ssl_certificate_key %s;\n' "$tls_dir/server.key"
        printf '%s\n' '    ssl_protocols TLSv1.2 TLSv1.3;' '    location = /redirect {'
        printf '      return 302 https://%s:%s/;\n' "$listen_host" "$listen_port"
        printf '%s\n' '    }' '    location / {'
        printf '      proxy_pass http://%s:%s;\n' "$upstream_host" "$upstream_port"
        printf '%s\n' "      proxy_set_header Host \$host;" '    }' '  }' '}'
      } > "$config"
      exec nginx -c "$config" -p "$tls_dir"
    '';
  };
  evmTlsProbe = pkgs.writeShellApplication {
    name = "mfm-evm-tls-probe";
    runtimeInputs = [ pkgs.curl ];
    text = ''
      exec curl --fail --silent --show-error \
        --cacert "''${1:?missing CA path}" \
        --header 'content-type: application/json' \
        --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' \
        "''${2:?missing endpoint}"
    '';
  };
  secureEvmRun = cargoArgs: ''
    set -euo pipefail
    tls_dir="''${stateDir}/evm-tls"
    ca_path="$tls_dir/ca.pem"
    alternate_ca_path="$tls_dir/alternate-ca.pem"
    ca_hex="$(sha256sum "$ca_path" | cut -d ' ' -f 1)"
    alternate_ca_hex="$(sha256sum "$alternate_ca_path" | cut -d ' ' -f 1)"
    rpc_url="https://''${host:evm-tls}:''${port:evm-tls}"
    wrong_host_url="https://localhost:''${port:evm-tls}"
    root_spec="{\"kind\":\"pem-file\",\"path\":\"$ca_path\",\"digest\":\"content:sha256-v1:$ca_hex\"}"
    alternate_spec="{\"kind\":\"pem-file\",\"path\":\"$alternate_ca_path\",\"digest\":\"content:sha256-v1:$alternate_ca_hex\"}"
    wrong_pin_spec="{\"kind\":\"pem-file\",\"path\":\"$ca_path\",\"digest\":\"content:sha256-v1:0000000000000000000000000000000000000000000000000000000000000000\"}"
    export MFM_TEST_EVM_ADAPTER_LOCATOR="{\"v\":1,\"url\":\"$rpc_url\",\"tls_roots\":$root_spec}"
    export MFM_TEST_EVM_REDIRECT_LOCATOR="{\"v\":1,\"url\":\"$rpc_url/redirect\",\"tls_roots\":$root_spec}"
    export MFM_TEST_EVM_WRONG_PIN_LOCATOR="{\"v\":1,\"url\":\"$rpc_url\",\"tls_roots\":$wrong_pin_spec}"
    export MFM_TEST_EVM_ALTERNATE_CA_LOCATOR="{\"v\":1,\"url\":\"$rpc_url\",\"tls_roots\":$alternate_spec}"
    export MFM_TEST_EVM_WRONG_HOST_LOCATOR="{\"v\":1,\"url\":\"$wrong_host_url\",\"tls_roots\":$root_spec}"
    export HTTPS_PROXY=http://127.0.0.1:1 HTTP_PROXY=http://127.0.0.1:1 ALL_PROXY=http://127.0.0.1:1
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
  nixfied.closures.evm-tls-prepare = {
    package = evmTlsPrepare;
    executable = "bin/mfm-evm-tls-prepare";
    effects = [
      "process"
      "file-write"
    ];
  };
  nixfied.closures.evm-tls-proxy = {
    package = evmTlsProxy;
    executable = "bin/mfm-evm-tls-proxy";
    effects = [
      "process"
      "network-listener"
      "file-write"
    ];
  };
  nixfied.closures.evm-tls-probe = {
    package = evmTlsProbe;
    executable = "bin/mfm-evm-tls-probe";
    effects = [
      "process"
      "network-listener"
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
  nixfied.tasks.evm-tls-init = {
    invocation = {
      tools = [ "evm-tls-prepare" ];
      run = [
        "mfm-evm-tls-prepare"
        "--state-dir"
        "\${stateDir}"
      ];
      timeoutMs = 60000;
    };
  };
  nixfied.services.evm-tls = {
    connectsTo = [ "reth" ];
    lifecycle = {
      prepare.task = "evm-tls-init";
      start.invocation = {
        tools = [ "evm-tls-proxy" ];
        run = [
          "mfm-evm-tls-proxy"
          "--state-dir"
          "\${stateDir}"
          "--listen-host"
          "127.0.0.1"
          "--listen-port"
          "\${port}"
          "--upstream-host"
          "\${host:reth}"
          "--upstream-port"
          "\${port:reth}"
        ];
      };
      ready.probe = {
        kind = "exec";
        invocation = {
          tools = [ "evm-tls-probe" ];
          run = [
            "mfm-evm-tls-probe"
            "\${stateDir}/evm-tls/ca.pem"
            "https://\${host}:\${port}"
          ];
        };
        timeoutMs = 2000;
        retryIntervalMs = 250;
        maxAttempts = 120;
      };
      health.probe = {
        kind = "exec";
        invocation = {
          tools = [ "evm-tls-probe" ];
          run = [
            "mfm-evm-tls-probe"
            "\${stateDir}/evm-tls/ca.pem"
            "https://\${host}:\${port}"
          ];
        };
        timeoutMs = 2000;
        retryIntervalMs = 250;
        maxAttempts = 120;
      };
      stop.timeoutMs = 10000;
    };
    endpoint.endpointId = "evm-json-rpc-tls";
    stateRefs = [ "slot" ];
    logRefs = [ "service.evm-tls" ];
    containment = "process-tree";
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
    transport-authority-test =
      (cargoLeaf {
        run = [
          "bash"
          "-c"
          (secureEvmRun ''
            exec cargo test -p mfm-evm-live managed_tls_authority -- --include-ignored --test-threads=1
          '')
        ];
        tools = [
          pkgs.coreutils
        ];
      })
      // {
        requires = [ "evm-tls" ];
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
          (localPostgresRun (secureEvmRun ''
            env -u PGSERVICE -u PGHOST -u PGPORT -u PGUSER -u PGDATABASE \
              -u PGPASSWORD -u PGPASSFILE \
              psql "$admin_dsn" -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
            DROP SCHEMA IF EXISTS mfm_catalog CASCADE;
            DROP SCHEMA IF EXISTS public CASCADE;
            CREATE SCHEMA public AUTHORIZATION CURRENT_USER;
            SQL
            export MFM_E2E_ADMIN_STORE_LOCATOR="$MFM_TEST_ADMIN_STORE_LOCATOR"
            export MFM_E2E_RUNTIME_STORE_LOCATOR="$MFM_TEST_RUNTIME_STORE_LOCATOR"
            export MFM_E2E_EVM_ADAPTER_LOCATOR="$MFM_TEST_EVM_ADAPTER_LOCATOR"
            exec cargo test -p mfm --test cli_e2e -- --include-ignored --test-threads=1
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
          "evm-tls"
        ];
      };
    rest-e2e =
      (cargoLeaf {
        run = [
          "bash"
          "-c"
          (localPostgresRun (secureEvmRun ''
            env -u PGSERVICE -u PGHOST -u PGPORT -u PGUSER -u PGDATABASE \
              -u PGPASSWORD -u PGPASSFILE \
              psql "$admin_dsn" -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
            DROP SCHEMA IF EXISTS mfm_catalog CASCADE;
            DROP SCHEMA IF EXISTS public CASCADE;
            CREATE SCHEMA public AUTHORIZATION CURRENT_USER;
            SQL
            export MFM_E2E_ADMIN_STORE_LOCATOR="$MFM_TEST_ADMIN_STORE_LOCATOR"
            export MFM_E2E_RUNTIME_STORE_LOCATOR="$MFM_TEST_RUNTIME_STORE_LOCATOR"
            export MFM_E2E_EVM_ADAPTER_LOCATOR="$MFM_TEST_EVM_ADAPTER_LOCATOR"
            cargo build -p mfm -p mfm-rest-api --bins
            export MFM_E2E_CLI_BIN="$CARGO_TARGET_DIR/debug/mfm_cli"
            export MFM_E2E_REST_BIN="$CARGO_TARGET_DIR/debug/mfm_rest_api"
            test -x "$MFM_E2E_CLI_BIN" -a -x "$MFM_E2E_REST_BIN"
            exec cargo test -p mfm-rest-api --test parity_e2e -- \
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
          "evm-tls"
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
        "transport-authority-test"
        "cli-e2e"
        "rest-e2e"
      ];
    };
  };

  nixfied.surface.verbs = [ "ci" ];
}
