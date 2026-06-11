{
  adapters,
  lib,
  pkgs,
  ...
}:
let
  rustToolchain = pkgs.rust-bin.stable."1.96.0".minimal.override {
    extensions = [
      "clippy"
      "rustfmt"
    ];
  };
  darwinLinkInputs = lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
    pkgs.libiconv
  ];
  ccEnvSuffix = lib.replaceStrings [ "-" ] [ "_" ] pkgs.stdenv.hostPlatform.config;
  darwinLinkSetup = lib.optionalString pkgs.stdenv.hostPlatform.isDarwin ''
    export NIX_LDFLAGS_${ccEnvSuffix}="''${NIX_LDFLAGS_${ccEnvSuffix}:-} -L${pkgs.libiconv}/lib"
    export CPATH="${pkgs.libiconv}/include:''${CPATH:-}"
  '';

  mfmRunner = pkgs.writeShellApplication {
    name = "mfm-nixfied-runner";
    runtimeInputs =
      [
        pkgs.coreutils
        pkgs.git
        pkgs.pkg-config
        rustToolchain
        pkgs.stdenv.cc
      ]
      ++ darwinLinkInputs;
    text = ''
      command_name="''${1:?missing mfm task command}"
      shift

      state_dir=""
      postgres_port=""
      reth_port=""
      while [[ $# -gt 0 ]]; do
        case "$1" in
          --state-dir)
            state_dir="''${2:?missing --state-dir value}"
            shift 2
            ;;
          --postgres-port)
            postgres_port="''${2:?missing --postgres-port value}"
            shift 2
            ;;
          --reth-port)
            reth_port="''${2:?missing --reth-port value}"
            shift 2
            ;;
          *)
            echo "unknown argument for $command_name: $1" >&2
            exit 64
            ;;
        esac
      done

      if [[ -z "$state_dir" ]]; then
        echo "missing required --state-dir argument" >&2
        exit 64
      fi

      mkdir -p "$state_dir/cargo-target"
      export CARGO_TARGET_DIR="''${CARGO_TARGET_DIR:-$state_dir/cargo-target}"
      export RUST_BACKTRACE="''${RUST_BACKTRACE:-1}"
      ${darwinLinkSetup}

      require_postgres_port() {
        if [[ -z "$postgres_port" ]]; then
          echo "missing required --postgres-port argument for $command_name" >&2
          exit 64
        fi
        export DATABASE_URL="postgresql://postgres@127.0.0.1:$postgres_port/postgres"
      }

      require_reth_port() {
        if [[ -z "$reth_port" ]]; then
          echo "missing required --reth-port argument for $command_name" >&2
          exit 64
        fi
        rpc_url="http://127.0.0.1:$reth_port"
        export RETH_HTTP_PORT="$reth_port"
        export MFM_EVM_RPC_SOURCES_JSON="{\"sources\":[{\"id\":\"reth-local\",\"rpc_url\":\"$rpc_url\",\"authorization\":null}],\"policies\":[{\"id\":\"reth-local\",\"ordered_sources\":[\"reth-local\"]}]}"
      }

      case "$command_name" in
        fmt)
          exec cargo fmt --all -- --check
          ;;
        clippy)
          exec cargo clippy --workspace --lib --examples --tests --benches --all-features -- -D warnings
          ;;
        cargo-metadata-contract)
          exec cargo test -p mfm-integration-tests --test cargo_metadata_contract
          ;;
        architecture-namespace-contract)
          exec cargo test -p mfm-integration-tests --test architecture_namespace_contract
          ;;
        workspace-tests)
          exec cargo test --workspace
          ;;
        parity-cli-keystore)
          exec cargo test -p mfm --features parity-tests --test parity_keystore_reth_tx_send -- --nocapture
          ;;
        parity-postgres-rest-api)
          require_postgres_port
          exec cargo test -p mfm-integration-tests --features parity-tests --test parity_rest_api_postgres_typed_smoke -- --nocapture
          ;;
        parity-postgres-state-events)
          require_postgres_port
          exec cargo test -p mfm-stream-store-postgres --features parity-tests -- --nocapture
          ;;
        parity-reth-contracts)
          require_reth_port
          exec cargo test -p mfm-integration-tests --features parity-tests --test parity_evm_contract_lifecycle_reth -- --nocapture
          ;;
        parity-reth-portfolio)
          require_reth_port
          exec cargo test -p mfm-integration-tests --features parity-tests --test parity_portfolio_tracker_reth_snapshot -- --nocapture
          ;;
        *)
          echo "unknown mfm task command: $command_name" >&2
          exit 64
          ;;
      esac
    '';
  };

  mfmTask = taskId: command: {
    operationId = "task.mfm.${taskId}.run";
    execId = "mfm-runner";
    args = [
      command
      "--state-dir"
      "\${stateDir}"
    ];
    logRefs = [ "task.mfm.${taskId}" ];
    summaryRefs = [ "summary" ];
  };

  mfmServiceTask =
    taskId: command: service:
    let
      baseTask = mfmTask taskId command;
    in
    baseTask
    // {
      dependsOnServicesReady = [ service ];
      args = baseTask.args ++ [
        "--${service}-port"
        "\${port}"
      ];
    };
in
{
  # Postgres and Reth come from the upstream reference adapters: idempotent
  # prepare, protocol probes (pg_isready / JSON-RPC), platform behavior, and
  # lifecycle are framework-owned. MFM declares only its own tasks/workflows.
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

  # Keep deterministic service windows outside common OS ephemeral ranges. Reth's
  # wrapper also derives ws/auth/p2p listeners as http+1/+2/+3, which the planner
  # does not reserve yet, so this range is dedicated to MFM.
  nixfied.placement.ports = {
    base = 28080;
    windowSize = 16;
    slotStride = 100;
  };

  nixfied.closures.mfm-runner = {
    package = mfmRunner;
    executable = "bin/mfm-nixfied-runner";
    kind = "executable";
    requiresExecutable = true;
    operationBindings = [
      "task.mfm.fmt.run"
      "task.mfm.clippy.run"
      "task.mfm.cargo-metadata-contract.run"
      "task.mfm.architecture-namespace-contract.run"
      "task.mfm.workspace-tests.run"
      "task.mfm.parity-cli-keystore.run"
      "task.mfm.parity-postgres-rest-api.run"
      "task.mfm.parity-postgres-state-events.run"
      "task.mfm.parity-reth-contracts.run"
      "task.mfm.parity-reth-portfolio.run"
    ];
    effects = [
      "process"
      "source-read"
      "file-write"
    ];
  };

  nixfied.execs.mfm-runner = {
    closureId = "mfm-runner";
    timeoutMs = 7200000;
  };

  nixfied.tasks = {
    mfm-fmt = mfmTask "fmt" "fmt";
    mfm-clippy = mfmTask "clippy" "clippy";
    mfm-cargo-metadata-contract = mfmTask "cargo-metadata-contract" "cargo-metadata-contract";
    mfm-architecture-namespace-contract = mfmTask "architecture-namespace-contract" "architecture-namespace-contract";
    mfm-workspace-tests = mfmTask "workspace-tests" "workspace-tests";
    mfm-parity-cli-keystore = mfmTask "parity-cli-keystore" "parity-cli-keystore";
    mfm-parity-postgres-rest-api =
      mfmServiceTask "parity-postgres-rest-api" "parity-postgres-rest-api"
        "postgres";
    mfm-parity-postgres-state-events =
      mfmServiceTask "parity-postgres-state-events" "parity-postgres-state-events"
        "postgres";
    mfm-parity-reth-contracts = mfmServiceTask "parity-reth-contracts" "parity-reth-contracts" "reth";
    mfm-parity-reth-portfolio = mfmServiceTask "parity-reth-portfolio" "parity-reth-portfolio" "reth";
  };

  nixfied.environments.dev = lib.mkForce {
    services = [
      "postgres"
      "reth"
    ];
    tasks = [ ];
  };

  nixfied.workflows.check.nodes = {
    fmt = {
      taskId = "mfm-fmt";
    };
    clippy = {
      taskId = "mfm-clippy";
      dependsOn = [ "fmt" ];
    };
    cargo-metadata-contract = {
      taskId = "mfm-cargo-metadata-contract";
      dependsOn = [ "clippy" ];
    };
    architecture-namespace-contract = {
      taskId = "mfm-architecture-namespace-contract";
      dependsOn = [ "cargo-metadata-contract" ];
    };
  };

  nixfied.workflows.test.nodes = {
    workspace-tests = {
      taskId = "mfm-workspace-tests";
    };
  };

  nixfied.workflows.ci = {
    servicesRequired = [
      "postgres"
      "reth"
    ];
    nodes = {
      fmt = {
        taskId = "mfm-fmt";
      };
      clippy = {
        taskId = "mfm-clippy";
        dependsOn = [ "fmt" ];
      };
      cargo-metadata-contract = {
        taskId = "mfm-cargo-metadata-contract";
        dependsOn = [ "clippy" ];
      };
      architecture-namespace-contract = {
        taskId = "mfm-architecture-namespace-contract";
        dependsOn = [ "cargo-metadata-contract" ];
      };
      workspace-tests = {
        taskId = "mfm-workspace-tests";
        dependsOn = [ "architecture-namespace-contract" ];
      };
      parity-cli-keystore = {
        taskId = "mfm-parity-cli-keystore";
        dependsOn = [ "workspace-tests" ];
      };
      parity-postgres-rest-api = {
        taskId = "mfm-parity-postgres-rest-api";
        dependsOn = [ "parity-cli-keystore" ];
      };
      parity-postgres-state-events = {
        taskId = "mfm-parity-postgres-state-events";
        dependsOn = [ "parity-postgres-rest-api" ];
      };
      parity-reth-contracts = {
        taskId = "mfm-parity-reth-contracts";
        dependsOn = [ "parity-postgres-state-events" ];
      };
      parity-reth-portfolio = {
        taskId = "mfm-parity-reth-portfolio";
        dependsOn = [ "parity-reth-contracts" ];
      };
    };
  };
}
