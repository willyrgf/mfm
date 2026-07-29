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
  ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [
    pkgs.bubblewrap
    "ldd"
  ]
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
  bitcoinCore =
    assert lib.versionAtLeast pkgs.bitcoind.version "28";
    assert pkgs.bitcoind.version == "31.0";
    pkgs.bitcoind;
  bitcoinCoreDisplayVersion = "Bitcoin Core daemon version v31.0.0 bitcoind";
  bitcoinNode = pkgs.writeShellApplication {
    name = "nixfied-bitcoind";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.findutils
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
      if [[ -e "$bitcoin_dir" ]]; then
        find "$bitcoin_dir" -depth -delete
      fi
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
  # Postgres uses its upstream adapter; Bitcoin Core remains project-owned below.
  imports = [
    adapters.postgres
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
  nixfied.closures.ldd = lib.mkIf pkgs.stdenv.hostPlatform.isLinux {
    package = pkgs.glibc.bin;
    executable = "bin/ldd";
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
          authoritative_schema_check() {
            cargo test --features parity-tests --lib \
              tests::verification_probe_accepts_the_current_authoritative_schema \
              -- --ignored --exact --nocapture
          }
          prepare_check
          authoritative_schema_check

          # Runtime queries deliberately consume the closed schema through the
          # authoritative validator rather than SQLx compile-time macros. Probe
          # that exact model even when Cargo reuses its compiled artifacts.
          psql "$admin_database_url" -v ON_ERROR_STOP=1 -c "ALTER TABLE \"$schema\"._sqlx_migrations DROP COLUMN checksum"
          if mutation_output="$(authoritative_schema_check 2>&1)"; then
            echo "schema mutation was not detected by authoritative validation" >&2
            exit 1
          fi
          mutation_output="''${mutation_output,,}"
          if [[ "$mutation_output" != *schemaauthoritymismatch* ]]; then
            echo "schema mutation failed for an unexpected reason" >&2
            exit 1
          fi
          echo "schema mutation correctly rejected by authoritative validation"

          psql "$admin_database_url" -v ON_ERROR_STOP=1 -c "DROP SCHEMA IF EXISTS \"$schema\" CASCADE"
          psql "$admin_database_url" -v ON_ERROR_STOP=1 -c "CREATE SCHEMA \"$schema\""
          cargo sqlx migrate run --source migrations
          prepare_check
          authoritative_schema_check
          echo "restored schema accepted by SQLx and authoritative validation"
        ''
      ];
      env = postgresSqlxEnv;
      requires = [ "postgres" ];
    };
    recoverability-postgres-v2 = cargoLeaf {
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
    executor-postgres-qualification = cargoLeaf {
      run = [
        "cargo"
        "test"
        "-p"
        "mfm-storage-executor-postgres"
        "--features"
        "qualification-tests"
        "--test"
        "qualification"
        "--"
        "--nocapture"
      ];
      env = postgresEnv;
      requires = [ "postgres" ];
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
    # Keep workspace and doctest commands as explicit leaves so each has its
    # own evidence. The workspace run enables app test support in-place.
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
        executor-postgres-qualification.task = "executor-postgres-qualification";
        postgres-sqlx-check.task = "postgres-sqlx-check";
        recoverability-postgres-v2 = {
          task = "recoverability-postgres-v2";
          dependsOn = [ "postgres-sqlx-check" ];
        };
      };
    };

    # Order service-free verification before PostgreSQL and project-owned
    # Bitcoin parity, then record the source revision that completed the gate.
    ci = {
      kind = "composite";
      steps = {
        check.task = "check";
        test = {
          task = "test";
          dependsOn = [ "check" ];
        };
        test-db = {
          task = "test-db";
          dependsOn = [ "test" ];
        };
        parity-bitcoin-core = {
          task = "parity-bitcoin-core";
          dependsOn = [ "test-db" ];
        };
        closing-source-revision = {
          task = "closing-source-revision";
          dependsOn = [ "parity-bitcoin-core" ];
        };
      };
    };
  };

  nixfied.surface.verbs = [
    "check"
    "test"
    "test-db"
    "ci"
  ];
}
