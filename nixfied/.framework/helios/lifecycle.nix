# Helios lifecycle management - init, start, stop, restart, status, health, check-config
{
  pkgs,
  project,
  slots,
  config,
}:

let
  lib = pkgs.lib;
  helios = config.package;
  rpcPortVar = slots.portVarName config.portKeyRpc;
  executionRpcPortVar = slots.portVarName config.executionRpcPortKey;
  heliosDirExpr = slots.getServiceDir config.dataDirName;
  extraArgs = lib.escapeShellArgs (config.extraArgs or [ ]);

  runtimePrelude = ''
    SLOT_INFO_OUT="$(${slots.getSlotInfo})" || exit 1
    eval "$SLOT_INFO_OUT"

    HELIOS_RPC_PORT_VAR="${rpcPortVar}"
    HELIOS_EXECUTION_PORT_VAR="${executionRpcPortVar}"

    HELIOS_RPC_PORT="''${!HELIOS_RPC_PORT_VAR:-}"
    HELIOS_EXECUTION_PORT="''${!HELIOS_EXECUTION_PORT_VAR:-}"
    HELIOS_DIR="${heliosDirExpr}"
    HELIOS_PID_FILE="$HELIOS_DIR/run/helios.pid"
    HELIOS_LOG_FILE="$HELIOS_DIR/logs/helios.log"

    HELIOS_NETWORK="''${HELIOS_NETWORK:-${config.network or "local"}}"
    HELIOS_EXECUTION_RPC_URL="''${HELIOS_EXECUTION_RPC_URL:-${config.executionRpcUrl or ""}}"
    HELIOS_CONSENSUS_RPC_URL="''${HELIOS_CONSENSUS_RPC_URL:-${config.consensusRpcUrl or ""}}"
    HELIOS_CHECKPOINT="''${HELIOS_CHECKPOINT:-${config.checkpoint or ""}}"

    if [ -z "$HELIOS_EXECUTION_RPC_URL" ] && [ -n "$HELIOS_EXECUTION_PORT" ]; then
      HELIOS_EXECUTION_RPC_URL="http://127.0.0.1:$HELIOS_EXECUTION_PORT"
    fi

    if [ -z "$HELIOS_RPC_PORT" ]; then
      echo "ERROR: helios RPC port variable is not set" >&2
      exit 1
    fi
  '';

  healthCheck = ''
    ${pkgs.curl}/bin/curl -fsS --max-time 2 \
      -H 'content-type: application/json' \
      --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' \
      "http://127.0.0.1:$HELIOS_RPC_PORT" \
      | ${pkgs.gnugrep}/bin/grep -q '"result"'
  '';

  init = pkgs.writeShellScript "helios-init" ''
    set -euo pipefail
    ${runtimePrelude}

    mkdir -p "$HELIOS_DIR/data"
    mkdir -p "$HELIOS_DIR/config"
    mkdir -p "$HELIOS_DIR/run"
    mkdir -p "$HELIOS_DIR/logs"

    echo "OK: helios initialized dir=$HELIOS_DIR slot=$SLOT env=$ENV"
  '';

  start = pkgs.writeShellScript "helios-start" ''
    set -euo pipefail
    ${runtimePrelude}

    ${init}

    if [ -f "$HELIOS_PID_FILE" ]; then
      PID=$(cat "$HELIOS_PID_FILE" 2>/dev/null || true)
      if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        echo "OK: helios already running pid=$PID rpc_port=$HELIOS_RPC_PORT"
        exit 0
      fi
      rm -f "$HELIOS_PID_FILE"
    fi

    if [ ! -x "${helios}/bin/helios" ]; then
      echo "ERROR: missing helios binary at ${helios}/bin/helios" >&2
      exit 1
    fi

    if [ "$HELIOS_NETWORK" != "local" ] && [ -z "$HELIOS_CONSENSUS_RPC_URL" ]; then
      echo "ERROR: HELIOS_CONSENSUS_RPC_URL is required when network is not local" >&2
      exit 1
    fi

    if [ -z "$HELIOS_EXECUTION_RPC_URL" ]; then
      echo "ERROR: HELIOS_EXECUTION_RPC_URL is required (or set executionRpcPortKey to a valid port key)" >&2
      exit 1
    fi

    ARGS=(
      --network "$HELIOS_NETWORK"
      --rpc-bind-ip 127.0.0.1
      --rpc-port "$HELIOS_RPC_PORT"
      --data-dir "$HELIOS_DIR/data"
      --execution-rpc "$HELIOS_EXECUTION_RPC_URL"
    )

    if [ -n "$HELIOS_CONSENSUS_RPC_URL" ]; then
      ARGS+=(--consensus-rpc "$HELIOS_CONSENSUS_RPC_URL")
    fi

    if [ -n "$HELIOS_CHECKPOINT" ]; then
      ARGS+=(--checkpoint "$HELIOS_CHECKPOINT")
    fi

    ${lib.optionalString ((config.extraArgs or [ ]) != [ ]) ''
      EXTRA_ARGS=(${extraArgs})
      ARGS+=("''${EXTRA_ARGS[@]}")
    ''}

    "${helios}/bin/helios" "''${ARGS[@]}" > "$HELIOS_LOG_FILE" 2>&1 &
    CHILD_PID=$!
    echo "$CHILD_PID" > "$HELIOS_PID_FILE"

    cleanup() {
      if [ -n "''${CHILD_PID:-}" ] && kill -0 "$CHILD_PID" 2>/dev/null; then
        kill "$CHILD_PID" 2>/dev/null || true
        wait "$CHILD_PID" 2>/dev/null || true
      fi
      rm -f "$HELIOS_PID_FILE"
    }

    trap cleanup EXIT INT TERM

    READY=0
    for _ in $(seq 1 80); do
      if ! kill -0 "$CHILD_PID" 2>/dev/null; then
        break
      fi
      if ${healthCheck}
      then
        READY=1
        break
      fi
      sleep 0.25
    done

    if [ "$READY" -ne 1 ]; then
      echo "ERROR: helios failed to become healthy. log=$HELIOS_LOG_FILE" >&2
      tail -50 "$HELIOS_LOG_FILE" >&2 || true
      exit 1
    fi

    echo "INFO: helios started pid=$CHILD_PID rpc_port=$HELIOS_RPC_PORT"
    wait "$CHILD_PID"
  '';

  stop = pkgs.writeShellScript "helios-stop" ''
    set -euo pipefail
    ${runtimePrelude}

    if [ ! -f "$HELIOS_PID_FILE" ]; then
      echo "OK: helios not running"
      exit 0
    fi

    PID=$(cat "$HELIOS_PID_FILE" 2>/dev/null || true)
    if [ -z "$PID" ] || ! kill -0 "$PID" 2>/dev/null; then
      rm -f "$HELIOS_PID_FILE"
      echo "OK: helios pid file cleaned"
      exit 0
    fi

    kill "$PID" 2>/dev/null || true
    for _ in $(seq 1 40); do
      if ! kill -0 "$PID" 2>/dev/null; then
        rm -f "$HELIOS_PID_FILE"
        echo "OK: helios stopped pid=$PID"
        exit 0
      fi
      sleep 0.25
    done

    kill -KILL "$PID" 2>/dev/null || true
    rm -f "$HELIOS_PID_FILE"
    echo "WARN: helios force-killed pid=$PID"
  '';

  restart = pkgs.writeShellScript "helios-restart" ''
    set -euo pipefail

    ${stop}
    exec ${start}
  '';

  status = pkgs.writeShellScript "helios-status" ''
    set -euo pipefail
    ${runtimePrelude}

    RUNNING=false
    PID=""

    if [ -f "$HELIOS_PID_FILE" ]; then
      PID=$(cat "$HELIOS_PID_FILE" 2>/dev/null || true)
      if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        RUNNING=true
      fi
    fi

    echo "service=helios slot=$SLOT env=$ENV running=$RUNNING pid=''${PID:-unknown} rpc_port=$HELIOS_RPC_PORT network=$HELIOS_NETWORK"

    if [ "$RUNNING" = "true" ]; then
      exit 0
    fi
    exit 1
  '';

  health = pkgs.writeShellScript "helios-health" ''
    set -euo pipefail
    ${runtimePrelude}

    if ${healthCheck}
    then
      echo "OK: helios healthy rpc_port=$HELIOS_RPC_PORT"
      exit 0
    fi

    echo "ERROR: helios unhealthy rpc_port=$HELIOS_RPC_PORT" >&2
    exit 1
  '';

  checkConfig = pkgs.writeShellScript "helios-check-config" ''
    set -euo pipefail
    ${runtimePrelude}

    if [ ! -x "${helios}/bin/helios" ]; then
      echo "ERROR: missing helios binary at ${helios}/bin/helios" >&2
      exit 1
    fi

    if [ "$HELIOS_NETWORK" != "local" ] && [ -z "$HELIOS_CONSENSUS_RPC_URL" ]; then
      echo "ERROR: HELIOS_CONSENSUS_RPC_URL is required when network is not local" >&2
      exit 1
    fi

    if [ -z "$HELIOS_EXECUTION_RPC_URL" ]; then
      echo "ERROR: HELIOS_EXECUTION_RPC_URL is required (or set executionRpcPortKey to a valid port key)" >&2
      exit 1
    fi

    ${helios}/bin/helios --help >/dev/null 2>&1 || true
    echo "OK: helios configuration valid dir=$HELIOS_DIR network=$HELIOS_NETWORK"
  '';
in
{
  inherit
    helios
    init
    start
    stop
    restart
    status
    health
    checkConfig
    ;
}
