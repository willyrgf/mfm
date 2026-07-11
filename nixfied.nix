{
  adapters,
  lib,
  pkgs,
  nixfiedLib,
  ...
}:
let
  # One pinned toolchain value, shared with flake.nix (nix/rust-toolchain.nix).
  rustToolchain = import ./nix/rust-toolchain.nix { inherit pkgs; };

  # The shared tool set every cargo leaf runs with: the runtime assembles the
  # child PATH from these roots and nothing else (hermetic env).
  cargoTools = [
    "rust-toolchain"
    pkgs.cargo-nextest
    pkgs.git
    pkgs.pkg-config
  ]
  ++ [ "cc" ]
  ++ lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];
  sqlxCli =
    assert pkgs.sqlx-cli.version == "0.9.0";
    pkgs.sqlx-cli;
  sqlxTools = cargoTools ++ [
    pkgs.bash
    "pg-psql"
    sqlxCli
  ];

  ccEnvSuffix = lib.replaceStrings [ "-" ] [ "_" ] pkgs.stdenv.hostPlatform.config;
  # Hermetic environment values; no append-to-inherited behavior because the child env starts empty.
  cargoEnv = {
    CARGO_TARGET_DIR = "\${stateDir}/cargo-target";
    RUST_BACKTRACE = "1";
    TMPDIR = "\${stateDir}";
  }
  // lib.optionalAttrs pkgs.stdenv.hostPlatform.isDarwin {
    "NIX_LDFLAGS_${ccEnvSuffix}" = "-L${pkgs.libiconv}/lib";
    CPATH = "${pkgs.libiconv}/include";
  };

  postgresEnv = {
    DATABASE_URL = "postgresql://postgres@\${host:postgres}:\${port:postgres}/postgres";
    SQLX_OFFLINE = "true";
  };
  postgresSqlxEnv = {
    DATABASE_URL = "postgresql://postgres@\${host:postgres}:\${port:postgres}/postgres";
    SQLX_OFFLINE = "false";
  };
  rethEnv = {
    RETH_HTTP_PORT = "\${port:reth}";
    MFM_RUNTIME_CONFIG_FILE = "\${stateDir}/runtime-config/reth-runtime.toml";
  };
  rethTools = cargoTools ++ [
    pkgs.bash
    pkgs.coreutils
  ];
  writeRethRuntimeConfig = ''
    set -euo pipefail
    runtime_config_file="$MFM_RUNTIME_CONFIG_FILE"
    runtime_config_dir="$(dirname "$runtime_config_file")"
    mkdir -p "$runtime_config_dir"
    chmod 700 "$runtime_config_dir"
    runtime_config_tmp="$(mktemp "$runtime_config_dir/reth-runtime.toml.tmp.XXXXXX")"
    cleanup_runtime_config() {
      rm -f "$runtime_config_tmp"
    }
    trap cleanup_runtime_config EXIT
    printf '%s\n' \
      '[evm.sources."reth-local"]' \
      'rpc_url = "http://''${host:reth}:''${port:reth}"' \
      "" \
      '[evm.routes."reth-local"]' \
      'source_ref = "reth-local"' \
      > "$runtime_config_tmp"
    chmod 600 "$runtime_config_tmp"
    mv -f "$runtime_config_tmp" "$runtime_config_file"
    trap - EXIT
  '';

  # A cargo leaf: argv + extra env + service requirements. Reuse is this Nix
  # function; the model carries the fully-applied copies.
  cargoLeaf =
    {
      run,
      env ? { },
      requires ? [ ],
      tools ? cargoTools,
    }:
    {
      invocation = {
        inherit tools;
        inherit run;
        env = cargoEnv // env;
        timeoutMs = 7200000;
      };
      inherit requires;
    };
in
{
  # Postgres and Reth come from the upstream reference adapters: idempotent
  # prepare, protocol probes (pg_isready / JSON-RPC), platform behavior, and
  # lifecycle are framework-owned. MFM declares only its own tasks.
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

  # Keep deterministic service windows outside common OS ephemeral ranges. Reth
  # models each listener endpoint explicitly, so this range is fully reserved by
  # the planner rather than derived in a wrapper.
  nixfied.placement.ports = {
    base = 28080;
    windowSize = 16;
    slotStride = 100;
  };

  # The toolchain closure anchors run[0] = "cargo"; cc anchors nothing (PATH
  # member for build scripts). Effects are the one hand-declared attestation.
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
        "--lib"
        "--examples"
        "--tests"
        "--benches"
        "--all-features"
        "--"
        "-D"
        "warnings"
      ];
    };
    cargo-metadata-contract = cargoLeaf {
      run = [
        "cargo"
        "test"
        "-p"
        "mfm-integration-tests"
        "--test"
        "cargo_metadata_contract"
      ];
    };
    postgres-sqlx-offline-check = cargoLeaf {
      run = [
        "cargo"
        "check"
        "-p"
        "mfm-stream-store-postgres"
        "--features"
        "parity-tests"
        "--all-targets"
      ];
      env = {
        DATABASE_URL = "postgresql://offline/offline";
        SQLX_OFFLINE = "true";
      };
    };
    nextest-run = cargoLeaf {
      run = [
        "cargo"
        "nextest"
        "run"
        "--workspace"
      ];
    };
    app-test-support = cargoLeaf {
      run = [
        "cargo"
        "nextest"
        "run"
        "-p"
        "mfm-app"
        "--features"
        "test-support"
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
    parity-cli-keystore = cargoLeaf {
      run = [
        "cargo"
        "test"
        "-p"
        "mfm"
        "--features"
        "parity-tests"
        "--test"
        "parity_keystore_reth_tx_send"
        "--"
        "--nocapture"
      ];
    };
    postgres-sqlx-check = cargoLeaf {
      tools = sqlxTools;
      run = [
        "bash"
        "-lc"
        ''
          set -euo pipefail
          admin_database_url="$DATABASE_URL"
          schema="sqlx_prepare_$$"
          psql "$admin_database_url" -v ON_ERROR_STOP=1 -c "CREATE SCHEMA $schema"
          cleanup() {
            psql "$admin_database_url" -v ON_ERROR_STOP=1 -c "DROP SCHEMA IF EXISTS $schema CASCADE"
          }
          trap cleanup EXIT

          if [[ "$admin_database_url" == *\?* ]]; then
            separator='&'
          else
            separator='?'
          fi
          export DATABASE_URL="$admin_database_url''${separator}options=-csearch_path%3D$schema"

          cd crates/storages/stream-store-postgres
          cargo sqlx migrate run --source migrations
          cargo clean -p mfm-stream-store-postgres
          cargo sqlx prepare --check -- --all-targets --features parity-tests
        ''
      ];
      env = postgresSqlxEnv;
      requires = [ "postgres" ];
    };
    mfm-cli-build = cargoLeaf {
      run = [
        "cargo"
        "build"
        "-p"
        "mfm"
        "--bin"
        "mfm_cli"
      ];
    };
    mfm-start-store = {
      serviceLifetime = "persistent-until-down";
      invocation = {
        tools = [ sqlxCli ];
        run = [
          "sqlx"
          "migrate"
          "run"
          "--source"
          "crates/storages/stream-store-postgres/migrations"
        ];
        env = postgresSqlxEnv;
        timeoutMs = 60000;
      };
      requires = [ "postgres" ];
    };
    parity-postgres-rest-api = cargoLeaf {
      run = [
        "cargo"
        "test"
        "-p"
        "mfm-integration-tests"
        "--features"
        "parity-tests"
        "--test"
        "parity_rest_api_postgres_smoke"
        "--"
        "--nocapture"
      ];
      env = postgresEnv;
      requires = [ "postgres" ];
    };
    parity-collect-then-report = cargoLeaf {
      run = [
        "cargo"
        "test"
        "-p"
        "mfm-integration-tests"
        "--features"
        "parity-tests"
        "--test"
        "collect_then_report_native_balances"
        "--"
        "--nocapture"
      ];
      env = postgresEnv;
      requires = [ "postgres" ];
    };
    parity-cli-postgres-status = cargoLeaf {
      run = [
        "cargo"
        "test"
        "-p"
        "mfm"
        "--features"
        "parity-tests"
        "--test"
        "status_contract_postgres"
        "--"
        "--nocapture"
      ];
      env = postgresEnv;
      requires = [ "postgres" ];
    };
    parity-postgres-state-events = cargoLeaf {
      run = [
        "cargo"
        "test"
        "-p"
        "mfm-stream-store-postgres"
        "--features"
        "parity-tests"
        "--"
        "--nocapture"
      ];
      env = postgresEnv;
      requires = [ "postgres" ];
    };
    parity-reth-portfolio = cargoLeaf {
      tools = rethTools;
      run = [
        "bash"
        "-lc"
        ''
          ${writeRethRuntimeConfig}
          cargo test -p mfm-integration-tests --features parity-tests --test parity_portfolio_tracker_reth_snapshot -- --nocapture
        ''
      ];
      env = rethEnv;
      requires = [ "reth" ];
    };

    # Keep workspace tests as explicit leaves so each command has its own evidence.
    workspace-tests = {
      kind = "composite";
      steps = nixfiedLib.seq [
        "nextest-run"
        "app-test-support"
        "doc-tests"
      ];
    };

    check = {
      kind = "composite";
      steps = nixfiedLib.seq [
        "fmt"
        "clippy"
        "cargo-metadata-contract"
        "postgres-sqlx-offline-check"
      ];
    };

    test = {
      kind = "composite";
      steps.workspace-tests.task = "workspace-tests";
    };

    test-db = {
      kind = "composite";
      steps = {
        postgres-sqlx-check.task = "postgres-sqlx-check";
        mfm-cli-build = {
          task = "mfm-cli-build";
          dependsOn = [ "postgres-sqlx-check" ];
        };
        parity-postgres-state-events = {
          task = "parity-postgres-state-events";
          dependsOn = [ "postgres-sqlx-check" ];
        };
        parity-collect-then-report = {
          task = "parity-collect-then-report";
          dependsOn = [
            "mfm-cli-build"
            "parity-postgres-state-events"
          ];
        };
        parity-postgres-rest-api = {
          task = "parity-postgres-rest-api";
          dependsOn = [ "parity-collect-then-report" ];
        };
        parity-cli-postgres-status = {
          task = "parity-cli-postgres-status";
          dependsOn = [ "parity-postgres-rest-api" ];
        };
      };
    };

    # The full gate, composed from the public verbs: `.#ci` runs the same
    # `check` and `test` composites that `.#check`/`.#test` expose (run-once is
    # per step, so nesting reuses them without duplication), then the parity
    # chain. The runtime starts the derived service union (postgres + reth, from
    # the parity leaves' `requires`) before the nodes execute.
    ci = {
      kind = "composite";
      steps = {
        check.task = "check";
        test = {
          task = "test";
          dependsOn = [ "check" ];
        };
        parity-cli-keystore = {
          task = "parity-cli-keystore";
          dependsOn = [ "test" ];
        };
        test-db = {
          task = "test-db";
          dependsOn = [ "parity-cli-keystore" ];
        };
        parity-reth-portfolio = {
          task = "parity-reth-portfolio";
          dependsOn = [ "test-db" ];
        };
      };
    };
  };

  # MFM's public verbs, in MFM's vocabulary: `nix run .#check`, `.#test`,
  # `.#ci`. Admission lives at the generated `.#model-check`.
  nixfied.surface.verbs = [
    "check"
    "test"
    "test-db"
    "ci"
  ];
}
