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
    pkgs.bash
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
    "pg-psql"
    sqlxCli
  ];

  ccEnvSuffix = lib.replaceStrings [ "-" ] [ "_" ] pkgs.stdenv.hostPlatform.config;
  # Verification-only profile policy: keep line tables for file/line backtraces,
  # avoid incremental and split-debug artifacts, and leave direct Cargo profiles unchanged.
  # These values are inherited by nested Cargo invocations such as trybuild and SQLx.
  cargoEnv = {
    CARGO_TARGET_DIR = "target/verification";
    CARGO_INCREMENTAL = "0";
    CARGO_PROFILE_DEV_DEBUG = "1";
    CARGO_PROFILE_TEST_DEBUG = "1";
    CARGO_PROFILE_DEV_SPLIT_DEBUGINFO = "off";
    CARGO_PROFILE_TEST_SPLIT_DEBUGINFO = "off";
    # Keep managed Rust checks within the memory envelope of the smallest
    # supported CI runner; Cargo can otherwise link too many proc macros at once.
    CARGO_BUILD_JOBS = "2";
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
    MFM_RETH_PARITY_HTTP_URL = "http://127.0.0.1:\${port:reth}";
  };
  bitcoinCore =
    assert lib.versionAtLeast pkgs.bitcoind.version "28";
    assert pkgs.bitcoind.version == "31.0";
    pkgs.bitcoind;
  bitcoinCoreDisplayVersion = "Bitcoin Core daemon version v31.0.0 bitcoind";
  bitcoinPrepare = pkgs.writeShellApplication {
    name = "nixfied-bitcoin-prepare";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.findutils
    ];
    text = ''
      state_dir="''${1:-}"
      if [[ -z "$state_dir" || "$state_dir" == "/" ]]; then
        echo "missing or unsafe Bitcoin Core state directory" >&2
        exit 64
      fi

      bitcoin_dir="$state_dir/bitcoin"
      if [[ -e "$bitcoin_dir" ]]; then
        find "$bitcoin_dir" -depth -delete
      fi
      umask 077
      mkdir -p "$bitcoin_dir"
    '';
  };
  bitcoinNode = pkgs.writeShellApplication {
    name = "nixfied-bitcoind";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.gnused
      bitcoinCore
    ];
    text = ''
      rpc_port=""
      state_dir=""
      host="127.0.0.1"

      while [[ $# -gt 0 ]]; do
        case "$1" in
          --rpc-port)
            rpc_port="''${2:?missing --rpc-port value}"
            shift 2
            ;;
          --state-dir)
            state_dir="''${2:?missing --state-dir value}"
            shift 2
            ;;
          *)
            echo "unknown bitcoind argument" >&2
            exit 64
            ;;
        esac
      done

      if [[ -z "$rpc_port" || -z "$state_dir" || "$state_dir" == "/" ]]; then
        echo "missing or unsafe Bitcoin Core service argument" >&2
        exit 64
      fi

      bitcoin_dir="$state_dir/bitcoin"
      umask 077
      mkdir -p "$bitcoin_dir"
      export HOME="$bitcoin_dir"

      actual_version="$(bitcoind -version | sed -n '1p')"
      if [[ "$actual_version" != "${bitcoinCoreDisplayVersion}" ]]; then
        echo "pinned Bitcoin Core executable version did not match ${bitcoinCoreDisplayVersion}" >&2
        exit 1
      fi

      exec bitcoind \
        -regtest \
        -datadir="$bitcoin_dir" \
        -server=1 \
        -disablewallet=1 \
        -daemon=0 \
        -printtoconsole=1 \
        -listen=0 \
        -discover=0 \
        -dnsseed=0 \
        -fixedseeds=0 \
        -listenonion=0 \
        -rest=0 \
        -rpcbind="$host" \
        -rpcallowip="$host" \
        -rpcport="$rpc_port" \
        -rpccookiefile="$bitcoin_dir/rpc.cookie"
    '';
  };
  bitcoinRpcProbeInvocation = {
    tools = [ "bitcoin-core-cli" ];
    run = [
      "bitcoin-cli"
      "-regtest"
      "-rpcconnect=127.0.0.1"
      "-rpcport=\${port}"
      "-rpccookiefile=\${stateDir}/bitcoin/rpc.cookie"
      "getblockchaininfo"
    ];
  };
  bitcoinParityEnv = {
    MFM_BITCOIN_CORE_VERSION = bitcoinCore.version;
    MFM_BITCOIN_PARITY_RPC_HOST = "127.0.0.1";
    MFM_BITCOIN_PARITY_RPC_PORT = "\${port:bitcoin-core}";
    MFM_BITCOIN_PARITY_COOKIE_FILE = "\${stateDir}/bitcoin/rpc.cookie";
  };

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
      inherit requires;
    };
in
{
  # Postgres and Reth come from upstream reference adapters: idempotent prepare,
  # protocol probes, platform behavior, and lifecycle are framework-owned.
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

  # Keep the deterministic service window outside common OS ephemeral ranges.
  nixfied.placement.ports = {
    base = 28080;
    windowSize = 16;
    slotStride = 100;
  };

  # Bash anchors the Cargo-leaf wrapper executable; the Rust toolchain and cc
  # remain PATH members for Cargo and build scripts. Effects are the one
  # hand-declared attestation.
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
  nixfied.closures.bitcoin-core-node = {
    package = bitcoinNode;
    executable = "bin/nixfied-bitcoind";
    effects = [
      "process"
      "network-listener"
      "file-write"
    ];
  };
  nixfied.closures.bitcoin-core-prepare = {
    package = bitcoinPrepare;
    executable = "bin/nixfied-bitcoin-prepare";
    effects = [
      "process"
      "file-write"
    ];
  };
  nixfied.closures.bitcoin-core-cli = {
    package = bitcoinCore;
    executable = "bin/bitcoin-cli";
    effects = [
      "process"
      "network-listener"
    ];
  };

  nixfied.services.bitcoin-core = {
    lifecycle = {
      prepare.task = "bitcoin-core-init";
      start.invocation = {
        tools = [ "bitcoin-core-node" ];
        run = [
          "nixfied-bitcoind"
          "--rpc-port"
          "\${port}"
          "--state-dir"
          "\${stateDir}"
        ];
      };
      ready.probe = {
        kind = "exec";
        invocation = bitcoinRpcProbeInvocation;
        timeoutMs = 2000;
        retryIntervalMs = 250;
        maxAttempts = 120;
      };
      health.probe = {
        kind = "exec";
        invocation = bitcoinRpcProbeInvocation;
        timeoutMs = 2000;
        retryIntervalMs = 250;
        maxAttempts = 120;
      };
      stop.timeoutMs = 10000;
    };
    endpoint.endpointId = "bitcoin-core-rpc";
    stateRefs = [ "slot" ];
    logRefs = [ "service.bitcoin-core" ];
    containment = "process-tree";
  };

  nixfied.tasks = {
    bitcoin-core-init = {
      invocation = {
        tools = [ "bitcoin-core-prepare" ];
        run = [
          "nixfied-bitcoin-prepare"
          "\${stateDir}"
        ];
        timeoutMs = 60000;
      };
    };
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
        "mfm-storage-postgres"
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
        "--features"
        "mfm-app/test-support"
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

          cd crates/storages/postgres
          cargo sqlx migrate run --source migrations
          prepare_check() {
            cargo sqlx prepare --check -- --all-targets --features parity-tests
          }
          prepare_check

          # The migration-ledger query must notice schema changes even when Rust
          # sources are unchanged and Cargo would otherwise reuse its artifacts.
          psql "$admin_database_url" -v ON_ERROR_STOP=1 -c "ALTER TABLE \"$schema\"._sqlx_migrations DROP COLUMN checksum"
          if mutation_output="$(prepare_check 2>&1)"; then
            echo "schema mutation was not detected by cargo sqlx prepare --check" >&2
            exit 1
          fi
          mutation_output="''${mutation_output,,}"
          if [[ "$mutation_output" != *checksum* ]]; then
            echo "schema mutation failed for an unexpected reason" >&2
            exit 1
          fi
          echo "schema mutation correctly rejected by cargo sqlx prepare --check"

          psql "$admin_database_url" -v ON_ERROR_STOP=1 -c "DROP SCHEMA IF EXISTS \"$schema\" CASCADE"
          psql "$admin_database_url" -v ON_ERROR_STOP=1 -c "CREATE SCHEMA \"$schema\""
          cargo sqlx migrate run --source migrations
          prepare_check
          echo "restored schema accepted by cargo sqlx prepare --check"
        ''
      ];
      env = postgresSqlxEnv;
      requires = [ "postgres" ];
    };
    mfm-store = {
      serviceLifetime = "persistent-until-down";
      invocation = {
        tools = [ sqlxCli ];
        run = [
          "sqlx"
          "migrate"
          "run"
          "--source"
          "crates/storages/postgres/migrations"
        ];
        env = postgresSqlxEnv;
        timeoutMs = 60000;
      };
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
    parity-cli-setup = cargoLeaf {
      run = [
        "cargo"
        "test"
        "-p"
        "mfm"
        "--features"
        "parity-tests"
        "--test"
        "setup_postgres"
        "--"
        "--nocapture"
      ];
      env = postgresEnv;
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
    parity-postgres-state-events = cargoLeaf {
      run = [
        "cargo"
        "test"
        "-p"
        "mfm-storage-postgres"
        "--features"
        "parity-tests"
        "--"
        "--nocapture"
      ];
      env = postgresEnv;
      requires = [ "postgres" ];
    };
    parity-reth-eip1559 = cargoLeaf {
      run = [
        "cargo"
        "test"
        "-p"
        "mfm-integration-tests"
        "--features"
        "parity-tests"
        "--test"
        "parity_reth_eip1559"
        "--"
        "--nocapture"
      ];
      env = rethEnv;
      requires = [ "reth" ];
    };
    parity-bitcoin-core = cargoLeaf {
      tools = cargoTools ++ [ "bitcoin-core-cli" ];
      run = [
        "cargo"
        "test"
        "-p"
        "mfm-bitcoin-live"
        "--features"
        "parity-tests"
        "--test"
        "parity_bitcoin_core"
        "--"
        "--nocapture"
      ];
      env = bitcoinParityEnv;
      requires = [ "bitcoin-core" ];
    };
    closing-source-revision = cargoLeaf {
      run = [
        "git"
        "rev-parse"
        "--verify"
        "HEAD^{commit}"
      ];
    };
    # Keep workspace tests and doctests as explicit leaves so each command has
    # its own evidence. The workspace run enables app test support in-place to
    # avoid executing the app's default tests a second time.
    workspace-tests = {
      kind = "composite";
      steps = nixfiedLib.seq [
        "nextest-run"
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
        parity-cli-setup = {
          task = "parity-cli-setup";
          dependsOn = [ "mfm-cli-build" ];
        };
        parity-postgres-rest-api = {
          task = "parity-postgres-rest-api";
          dependsOn = [ "parity-postgres-state-events" ];
        };
      };
    };

    # The full gate, composed from the public verbs: `.#ci` runs the same
    # `check` and `test` composites that `.#check`/`.#test` expose (run-once is
    # per step, so nesting reuses them without duplication), then the parity
    # chain. The runtime starts the managed services declared by the parity
    # leaves' `requires` before the nodes execute.
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
        parity-reth-eip1559 = {
          task = "parity-reth-eip1559";
          dependsOn = [ "test-db" ];
        };
        parity-bitcoin-core = {
          task = "parity-bitcoin-core";
          dependsOn = [ "parity-reth-eip1559" ];
        };
        closing-source-revision = {
          task = "closing-source-revision";
          dependsOn = [ "parity-bitcoin-core" ];
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
