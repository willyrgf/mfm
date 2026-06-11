{
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

  mfmRunner = pkgs.writeShellApplication {
    name = "mfm-nixfied-runner";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.git
      pkgs.pkg-config
      rustToolchain
      pkgs.stdenv.cc
    ];
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

  postgresWrapper = pkgs.writeShellApplication {
    name = "mfm-nixfied-postgres";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.postgresql
    ];
    # Slot state is reused across runs; prepare tolerates an existing cluster and
    # rebuilds only incomplete service-owned pgdata.
    text = ''
      command_name="''${1:?missing postgres command}"
      shift

      state_dir=""
      port=""
      host="127.0.0.1"

      while [[ $# -gt 0 ]]; do
        case "$1" in
          --state-dir)
            state_dir="''${2:?missing --state-dir value}"
            shift 2
            ;;
          --port)
            port="''${2:?missing --port value}"
            shift 2
            ;;
          --host)
            host="''${2:?missing --host value}"
            shift 2
            ;;
          *)
            echo "unknown postgres argument: $1" >&2
            exit 64
            ;;
        esac
      done

      if [[ -z "$state_dir" || "$state_dir" == "/" ]]; then
        echo "missing or unsafe --state-dir argument" >&2
        exit 64
      fi

      pgdata="$state_dir/pgdata"

      case "$command_name" in
        prepare)
          if [[ -s "$pgdata/PG_VERSION" ]]; then
            exit 0
          fi

          rm -rf "$pgdata"
          mkdir -p "$pgdata"
          exec initdb \
            -D "$pgdata" \
            -U postgres \
            -A trust \
            --no-locale \
            --encoding=UTF8
          ;;
        start)
          if [[ -z "$port" ]]; then
            echo "missing required --port argument" >&2
            exit 64
          fi
          exec postgres \
            -D "$pgdata" \
            -c unix_socket_directories= \
            -h "$host" \
            -p "$port"
          ;;
        *)
          echo "unknown postgres command: $command_name" >&2
          exit 64
          ;;
      esac
    '';
  };

  rethWrapper = pkgs.writeShellApplication {
    name = "mfm-nixfied-reth";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.reth
    ];
    text = ''
      http_port=""
      state_dir=""
      host="127.0.0.1"

      while [[ $# -gt 0 ]]; do
        case "$1" in
          --http-port)
            http_port="''${2:?missing --http-port value}"
            shift 2
            ;;
          --state-dir)
            state_dir="''${2:?missing --state-dir value}"
            shift 2
            ;;
          --host)
            host="''${2:?missing --host value}"
            shift 2
            ;;
          *)
            echo "unknown reth argument: $1" >&2
            exit 64
            ;;
        esac
      done

      if [[ -z "$http_port" || -z "$state_dir" ]]; then
        echo "missing required --http-port or --state-dir argument" >&2
        exit 64
      fi

      ws_port=$((http_port + 1))
      auth_port=$((http_port + 2))
      p2p_port=$((http_port + 3))
      reth_dir="$state_dir/reth"
      jwt_file="$reth_dir/config/jwt.hex"
      ipc_path="/tmp/mfm-reth-$http_port.ipc"

      mkdir -p "$reth_dir/data" "$reth_dir/run" "$reth_dir/config"
      if [[ ! -s "$jwt_file" ]]; then
        printf '%064x\n' 0 > "$jwt_file"
      fi
      chmod 600 "$jwt_file" 2>/dev/null || true
      rm -f "$ipc_path"

      exec reth node \
        --datadir "$reth_dir/data" \
        --ipcpath "$ipc_path" \
        --port "$p2p_port" \
        --http \
        --http.addr "$host" \
        --http.port "$http_port" \
        --ws \
        --ws.addr "$host" \
        --ws.port "$ws_port" \
        --authrpc.addr "$host" \
        --authrpc.port "$auth_port" \
        --authrpc.jwtsecret "$jwt_file" \
        --dev
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
  nixfied.project.projectId = "mfm";
  nixfied.project.name = "MFM";
  nixfied.codebases.main.logicalRoot = ".";

  nixfied.slotPolicy = {
    min = 0;
    default = 0;
    max = 9;
  };

  nixfied.placement.ports = {
    base = 38080;
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

  nixfied.closures.mfm-postgres = {
    package = postgresWrapper;
    executable = "bin/mfm-nixfied-postgres";
    kind = "executable";
    requiresExecutable = true;
    operationBindings = [
      "service.postgres.prepare"
      "service.postgres.start"
    ];
    effects = [
      "process"
      "network-listener"
      "file-write"
    ];
  };

  nixfied.closures.mfm-reth = {
    package = rethWrapper;
    executable = "bin/mfm-nixfied-reth";
    kind = "executable";
    requiresExecutable = true;
    operationBindings = [ "service.reth.start" ];
    effects = [
      "process"
      "network-listener"
      "file-write"
    ];
  };

  nixfied.execs = {
    mfm-runner = {
      closureId = "mfm-runner";
      timeoutMs = 7200000;
    };
    mfm-postgres-prepare = {
      closureId = "mfm-postgres";
      timeoutMs = 60000;
      args = [
        "prepare"
        "--state-dir"
        "\${stateDir}"
      ];
    };
    mfm-postgres-start = {
      closureId = "mfm-postgres";
      timeoutMs = 60000;
      args = [
        "start"
        "--state-dir"
        "\${stateDir}"
        "--port"
        "\${port}"
      ];
    };
    mfm-reth = {
      closureId = "mfm-reth";
      timeoutMs = 60000;
    };
  };

  nixfied.services.postgres = {
    lifecycle = {
      prepare = {
        operationId = "service.postgres.prepare";
        execId = "mfm-postgres-prepare";
        terminal = {
          success = "initialized";
          failure = "failed";
        };
      };
      start = {
        operationId = "service.postgres.start";
        execId = "mfm-postgres-start";
        terminal = {
          success = "spawned";
          failure = "failed";
        };
      };
      ready = {
        operationId = "service.postgres.ready";
        probe = {
          timeoutMs = 1000;
          retryIntervalMs = 200;
          maxAttempts = 60;
        };
        terminal = {
          success = "ready";
          failure = "not-ready";
        };
      };
      health = {
        operationId = "service.postgres.health";
        probe = {
          timeoutMs = 1000;
          retryIntervalMs = 200;
          maxAttempts = 60;
        };
        terminal = {
          success = "healthy";
          failure = "unhealthy";
        };
      };
      stop = {
        operationId = "service.postgres.stop";
        signal = "INT";
        terminal = {
          success = "stopped";
          failure = "failed";
        };
      };
      clean = {
        operationId = "service.postgres.clean";
        terminal = {
          success = "cleaned";
          failure = "failed";
        };
      };
    };
    endpoint = {
      endpointId = "postgres-tcp";
    };
    stateRefs = [ "slot" ];
    logRefs = [ "service.postgres" ];
    containment = "process-tree";
  };

  nixfied.services.reth = {
    lifecycle = {
      prepare = {
        operationId = "service.reth.prepare";
        terminal = {
          success = "prepared";
          failure = "failed";
        };
      };
      start = {
        operationId = "service.reth.start";
        execId = "mfm-reth";
        execArgs = [
          "--http-port"
          "\${port}"
          "--state-dir"
          "\${stateDir}"
        ];
        terminal = {
          success = "spawned";
          failure = "failed";
        };
      };
      ready = {
        operationId = "service.reth.ready";
        probe = {
          timeoutMs = 1000;
          retryIntervalMs = 500;
          maxAttempts = 180;
        };
        terminal = {
          success = "ready";
          failure = "not-ready";
        };
      };
      health = {
        operationId = "service.reth.health";
        probe = {
          timeoutMs = 1000;
          retryIntervalMs = 500;
          maxAttempts = 180;
        };
        terminal = {
          success = "healthy";
          failure = "unhealthy";
        };
      };
      stop = {
        operationId = "service.reth.stop";
        signal = "INT";
        timeoutMs = 10000;
        terminal = {
          success = "stopped";
          failure = "failed";
        };
      };
      clean = {
        operationId = "service.reth.clean";
        terminal = {
          success = "cleaned";
          failure = "failed";
        };
      };
    };
    endpoint = {
      endpointId = "reth-http";
    };
    stateRefs = [ "slot" ];
    logRefs = [ "service.reth" ];
    containment = "process-tree";
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

  nixfied.workflows.check = {
    nodes = [
      {
        nodeId = "fmt";
        taskId = "mfm-fmt";
      }
      {
        nodeId = "clippy";
        taskId = "mfm-clippy";
        dependsOn = [ "fmt" ];
      }
      {
        nodeId = "cargo-metadata-contract";
        taskId = "mfm-cargo-metadata-contract";
        dependsOn = [ "clippy" ];
      }
      {
        nodeId = "architecture-namespace-contract";
        taskId = "mfm-architecture-namespace-contract";
        dependsOn = [ "cargo-metadata-contract" ];
      }
    ];
  };

  nixfied.workflows.test = {
    nodes = [
      {
        nodeId = "workspace-tests";
        taskId = "mfm-workspace-tests";
      }
    ];
  };

  nixfied.workflows.ci = {
    servicesRequired = [
      "postgres"
      "reth"
    ];
    nodes = [
      {
        nodeId = "fmt";
        taskId = "mfm-fmt";
      }
      {
        nodeId = "clippy";
        taskId = "mfm-clippy";
        dependsOn = [ "fmt" ];
      }
      {
        nodeId = "cargo-metadata-contract";
        taskId = "mfm-cargo-metadata-contract";
        dependsOn = [ "clippy" ];
      }
      {
        nodeId = "architecture-namespace-contract";
        taskId = "mfm-architecture-namespace-contract";
        dependsOn = [ "cargo-metadata-contract" ];
      }
      {
        nodeId = "workspace-tests";
        taskId = "mfm-workspace-tests";
        dependsOn = [ "architecture-namespace-contract" ];
      }
      {
        nodeId = "parity-cli-keystore";
        taskId = "mfm-parity-cli-keystore";
        dependsOn = [ "workspace-tests" ];
      }
      {
        nodeId = "parity-postgres-rest-api";
        taskId = "mfm-parity-postgres-rest-api";
        dependsOn = [ "parity-cli-keystore" ];
      }
      {
        nodeId = "parity-postgres-state-events";
        taskId = "mfm-parity-postgres-state-events";
        dependsOn = [ "parity-postgres-rest-api" ];
      }
      {
        nodeId = "parity-reth-contracts";
        taskId = "mfm-parity-reth-contracts";
        dependsOn = [ "parity-postgres-state-events" ];
      }
      {
        nodeId = "parity-reth-portfolio";
        taskId = "mfm-parity-reth-portfolio";
        dependsOn = [ "parity-reth-contracts" ];
      }
    ];
  };
}
