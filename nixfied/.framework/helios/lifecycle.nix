# Helios lifecycle management - init, start, stop, restart, status, health, check-config
{
  pkgs,
  project,
  slots,
  config,
}:

let
  lib = pkgs.lib;
  processRegistry = import ../lib/process-registry.nix { inherit pkgs project; };
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
    HELIOS_DEFAULT_CONSENSUS_RPC_URL="''${HELIOS_DEFAULT_CONSENSUS_RPC_URL:-${
      config.defaultConsensusRpcUrl or ""
    }}"
    HELIOS_CHECKPOINT="''${HELIOS_CHECKPOINT:-${config.checkpoint or ""}}"

    if [ -z "$HELIOS_EXECUTION_RPC_URL" ] && [ -n "$HELIOS_EXECUTION_PORT" ]; then
      HELIOS_EXECUTION_RPC_URL="http://127.0.0.1:$HELIOS_EXECUTION_PORT"
    fi

    # Default consensus endpoint for mainnet if not explicitly configured.
    # This keeps testnets and other networks explicit to avoid accidentally mixing networks.
    if [ "$HELIOS_NETWORK" = "mainnet" ] && [ -z "$HELIOS_CONSENSUS_RPC_URL" ] && [ -n "$HELIOS_DEFAULT_CONSENSUS_RPC_URL" ]; then
      HELIOS_CONSENSUS_RPC_URL="$HELIOS_DEFAULT_CONSENSUS_RPC_URL"
    fi

    # Helios local profile expects a consensus endpoint; default to execution RPC
    # so local dev/testing can run without a separate consensus client.
    if [ "$HELIOS_NETWORK" = "local" ] && [ -z "$HELIOS_CONSENSUS_RPC_URL" ] && [ -n "$HELIOS_EXECUTION_RPC_URL" ]; then
      HELIOS_CONSENSUS_RPC_URL="$HELIOS_EXECUTION_RPC_URL"
    fi

    # Normalize for composing paths.
    HELIOS_CONSENSUS_RPC_URL="''${HELIOS_CONSENSUS_RPC_URL%/}"

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
        ${processRegistry.emitEvent} \
          --event-type service_ready \
          --service helios \
          --state ready \
          --slot "$SLOT" \
          --env "$ENV" \
          --pid "$PID" \
          --log-path "$HELIOS_LOG_FILE" >/dev/null 2>&1 || true
        echo "OK: helios already running pid=$PID rpc_port=$HELIOS_RPC_PORT"
        exit 0
      fi
      rm -f "$HELIOS_PID_FILE"
    fi

    if [ ! -x "${helios}/bin/helios" ]; then
      echo "ERROR: missing helios binary at ${helios}/bin/helios" >&2
      exit 1
    fi

    if [ -z "$HELIOS_EXECUTION_RPC_URL" ]; then
      echo "ERROR: HELIOS_EXECUTION_RPC_URL is required (or set executionRpcPortKey to a valid port key)" >&2
      exit 1
    fi

    # Derive a recent weak-subjectivity checkpoint when not pinned explicitly.
    #
    # This avoids a common failure mode where Helios stays "healthy" but remains unable to answer
    # `eth_blockNumber` because the consensus light client never bootstrapped.
    if [ "$HELIOS_NETWORK" != "local" ] && [ -z "$HELIOS_CHECKPOINT" ]; then
      if [ -z "$HELIOS_CONSENSUS_RPC_URL" ]; then
        echo "ERROR: cannot derive HELIOS_CHECKPOINT: HELIOS_CONSENSUS_RPC_URL is empty" >&2
        exit 1
      fi

      # Mainnet fallback: lightclientdata can be temporarily unavailable (e.g. 503).
      # Prefer it first (Helios upstream default), then try a known public Lodestar endpoint.
      CONS_CANDIDATES=("$HELIOS_CONSENSUS_RPC_URL")
      if [ "$HELIOS_NETWORK" = "mainnet" ] && [ "$HELIOS_CONSENSUS_RPC_URL" = "https://www.lightclientdata.org" ]; then
        CONS_CANDIDATES+=("https://lodestar-mainnet.chainsafe.io")
      fi

      DERIVED=0
      for CONS in "''${CONS_CANDIDATES[@]}"; do
        if [ -z "''${CONS:-}" ]; then
          continue
        fi
        CONS="''${CONS%/}"

        echo "INFO: deriving HELIOS_CHECKPOINT from consensus endpoint cons=$CONS" >&2

        FINALIZED_URL="$CONS/eth/v1/beacon/headers/finalized"
        if ! FINALIZED_JSON="$(${pkgs.curl}/bin/curl -fsS --max-time 10 \
          --retry 3 --retry-delay 1 --retry-max-time 30 \
          -H 'accept: application/json' \
          "$FINALIZED_URL" 2>&1)"; then
          echo "WARN: failed to fetch finalized header cons=$CONS url=$FINALIZED_URL" >&2
          echo "DETAIL: $FINALIZED_JSON" >&2
          continue
        fi

        slot="$(echo "$FINALIZED_JSON" | ${pkgs.jq}/bin/jq -r '.data.header.message.slot|tonumber' 2>/dev/null || true)"
        case "$slot" in
          *[!0-9]*|"")
            echo "WARN: failed to parse finalized slot from consensus response cons=$CONS" >&2
            continue
            ;;
        esac

        epoch_start=$((slot - (slot % 32)))

        EPOCH_URL="$CONS/eth/v1/beacon/headers/$epoch_start"
        if ! EPOCH_JSON="$(${pkgs.curl}/bin/curl -fsS --max-time 10 \
          --retry 3 --retry-delay 1 --retry-max-time 30 \
          -H 'accept: application/json' \
          "$EPOCH_URL" 2>&1)"; then
          echo "WARN: failed to fetch epoch boundary header cons=$CONS url=$EPOCH_URL" >&2
          echo "DETAIL: $EPOCH_JSON" >&2
          continue
        fi

        checkpoint="$(echo "$EPOCH_JSON" | ${pkgs.jq}/bin/jq -r '.data.root // empty' 2>/dev/null || true)"
        if ! echo "$checkpoint" | ${pkgs.gnugrep}/bin/grep -Eq '^0x[0-9a-fA-F]{64}$'; then
          echo "WARN: invalid checkpoint root from consensus endpoint cons=$CONS root='$checkpoint'" >&2
          continue
        fi

        # Sanity-check that the light-client bootstrap endpoint is served for this checkpoint.
        BOOTSTRAP_URL="$CONS/eth/v1/beacon/light_client/bootstrap/$checkpoint"
        if ! BOOTSTRAP_ERR="$(${pkgs.curl}/bin/curl -fsS --max-time 10 \
          --retry 3 --retry-delay 1 --retry-max-time 30 \
          -H 'accept: application/json' \
          -o /dev/null \
          "$BOOTSTRAP_URL" 2>&1)"; then
          echo "WARN: consensus endpoint does not serve light_client/bootstrap cons=$CONS url=$BOOTSTRAP_URL" >&2
          echo "DETAIL: $BOOTSTRAP_ERR" >&2
          continue
        fi

        HELIOS_CONSENSUS_RPC_URL="$CONS"
        HELIOS_CHECKPOINT="$checkpoint"
        echo "INFO: derived HELIOS_CHECKPOINT=$HELIOS_CHECKPOINT (cons=$HELIOS_CONSENSUS_RPC_URL)" >&2
        DERIVED=1
        break
      done

      if [ "$DERIVED" -ne 1 ]; then
        echo "ERROR: failed to derive HELIOS_CHECKPOINT from consensus endpoint(s)." >&2
        echo "HINT: set HELIOS_CONSENSUS_RPC_URL and HELIOS_CHECKPOINT explicitly." >&2
        exit 1
      fi
    fi

    if [ "$HELIOS_NETWORK" != "local" ] && [ -z "$HELIOS_CONSENSUS_RPC_URL" ]; then
      echo "ERROR: HELIOS_CONSENSUS_RPC_URL is required when network is not local" >&2
      exit 1
    fi

    ARGS=(
      ethereum
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

    ${processRegistry.emitEvent} \
      --event-type service_starting \
      --service helios \
      --state starting \
      --slot "$SLOT" \
      --env "$ENV" \
      --pid "$CHILD_PID" \
      --log-path "$HELIOS_LOG_FILE" >/dev/null 2>&1 || true

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
      ${processRegistry.emitEvent} \
        --event-type service_degraded \
        --service helios \
        --state degraded \
        --slot "$SLOT" \
        --env "$ENV" \
        --pid "$CHILD_PID" \
        --log-path "$HELIOS_LOG_FILE" \
        --wait-reason "failed_startup_health" \
        --last-error "helios failed initial health checks" >/dev/null 2>&1 || true
      echo "ERROR: helios failed to become healthy. log=$HELIOS_LOG_FILE" >&2
      if [ -f "$HELIOS_LOG_FILE" ]; then
        echo "INFO: helios log tail path=$HELIOS_LOG_FILE lines=50" >&2
        tail -50 "$HELIOS_LOG_FILE" >&2 || true
      else
        echo "WARN: helios log file missing path=$HELIOS_LOG_FILE" >&2
      fi
      exit 1
    fi

    ${processRegistry.emitEvent} \
      --event-type service_ready \
      --service helios \
      --state ready \
      --slot "$SLOT" \
      --env "$ENV" \
      --pid "$CHILD_PID" \
      --log-path "$HELIOS_LOG_FILE" >/dev/null 2>&1 || true

    echo "INFO: helios started pid=$CHILD_PID rpc_port=$HELIOS_RPC_PORT"
    set +e
    wait "$CHILD_PID"
    RC=$?
    set -e

    if [ "$RC" -eq 0 ]; then
      ${processRegistry.emitEvent} \
        --event-type service_stopped \
        --service helios \
        --state stopped \
        --slot "$SLOT" \
        --env "$ENV" \
        --pid "$CHILD_PID" \
        --log-path "$HELIOS_LOG_FILE" >/dev/null 2>&1 || true
    else
      ${processRegistry.emitEvent} \
        --event-type service_degraded \
        --service helios \
        --state degraded \
        --slot "$SLOT" \
        --env "$ENV" \
        --pid "$CHILD_PID" \
        --log-path "$HELIOS_LOG_FILE" \
        --wait-reason "helios_process_exit code=$RC" \
        --last-error "helios process exited non-zero" >/dev/null 2>&1 || true
    fi
    exit "$RC"
  '';

  stop = pkgs.writeShellScript "helios-stop" ''
    set -euo pipefail
    ${runtimePrelude}

    if [ ! -f "$HELIOS_PID_FILE" ]; then
      ${processRegistry.emitEvent} \
        --event-type service_stopped \
        --service helios \
        --state stopped \
        --slot "$SLOT" \
        --env "$ENV" \
        --log-path "$HELIOS_LOG_FILE" >/dev/null 2>&1 || true
      echo "OK: helios not running"
      exit 0
    fi

    PID=$(cat "$HELIOS_PID_FILE" 2>/dev/null || true)
    if [ -z "$PID" ] || ! kill -0 "$PID" 2>/dev/null; then
      rm -f "$HELIOS_PID_FILE"
      ${processRegistry.emitEvent} \
        --event-type service_stopped \
        --service helios \
        --state stopped \
        --slot "$SLOT" \
        --env "$ENV" \
        --pid "$PID" \
        --log-path "$HELIOS_LOG_FILE" >/dev/null 2>&1 || true
      echo "OK: helios pid file cleaned"
      exit 0
    fi

    kill "$PID" 2>/dev/null || true
    for _ in $(seq 1 40); do
      if ! kill -0 "$PID" 2>/dev/null; then
        rm -f "$HELIOS_PID_FILE"
        ${processRegistry.emitEvent} \
          --event-type service_stopped \
          --service helios \
          --state stopped \
          --slot "$SLOT" \
          --env "$ENV" \
          --pid "$PID" \
          --log-path "$HELIOS_LOG_FILE" >/dev/null 2>&1 || true
        echo "OK: helios stopped pid=$PID"
        exit 0
      fi
      sleep 0.25
    done

    kill -KILL "$PID" 2>/dev/null || true
    rm -f "$HELIOS_PID_FILE"
    ${processRegistry.emitEvent} \
      --event-type service_stopped \
      --service helios \
      --state stopped \
      --slot "$SLOT" \
      --env "$ENV" \
      --pid "$PID" \
      --log-path "$HELIOS_LOG_FILE" >/dev/null 2>&1 || true
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

    LOCAL_RUNNING="$RUNNING"
    REGISTRY_FOUND="0"
    REGISTRY_RUNNING="false"
    REGISTRY_STATE="unknown"
    OWNER_RUN_ID=""
    OWNER_SCOPE=""
    EPHEMERAL_ROOT=""
    WAIT_REASON=""
    LOG_PATH=""
    SLOT_OWNER=""
    REGISTRY_SCOPE="global"

    REG_OUT="$(${processRegistry.serviceStatus} --service helios --slot "$SLOT" --env "$ENV" 2>/dev/null || true)"
    if [ -n "$REG_OUT" ]; then
      eval "$REG_OUT"
    fi

    if [ "$RUNNING" != "true" ] && [ "$REGISTRY_RUNNING" = "true" ]; then
      RUNNING=true
    fi

    SCOPE="none"
    if [ "$LOCAL_RUNNING" = "true" ]; then
      SCOPE="local"
    elif [ "$REGISTRY_RUNNING" = "true" ]; then
      SCOPE="global"
    fi

    EFFECTIVE_LOG_PATH="$HELIOS_LOG_FILE"
    if [ -n "$LOG_PATH" ]; then
      EFFECTIVE_LOG_PATH="$LOG_PATH"
    fi

    echo "service=helios slot=$SLOT env=$ENV running=$RUNNING pid=''${PID:-unknown} rpc_port=$HELIOS_RPC_PORT network=$HELIOS_NETWORK scope=$SCOPE owner_run_id=''${OWNER_RUN_ID:-unknown} owner_scope=''${OWNER_SCOPE:-unknown} ephemeral_root=''${EPHEMERAL_ROOT:-none} registry_state=''${REGISTRY_STATE:-unknown} slot_owner=''${SLOT_OWNER:-unknown} wait_reason=''${WAIT_REASON:-none} log_path=$EFFECTIVE_LOG_PATH"

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

    # Ensure the expected Helios subcommand exists; CLI shape changes should fail fast here.
    ${helios}/bin/helios ethereum --help >/dev/null 2>&1
    echo "OK: helios configuration valid dir=$HELIOS_DIR network=$HELIOS_NETWORK"
  '';

  ready = pkgs.writeShellScript "helios-ready" ''
    set -euo pipefail
    ${runtimePrelude}

    TIMEOUT_SECS="''${HELIOS_READY_TIMEOUT_SECS:-300}"
    INTERVAL_SECS="''${HELIOS_READY_INTERVAL_SECS:-1}"

    case "$TIMEOUT_SECS" in
      *[!0-9]*|"")
        echo "ERROR: HELIOS_READY_TIMEOUT_SECS must be an integer seconds value (got '$TIMEOUT_SECS')" >&2
        exit 1
        ;;
    esac

    start_ts="$(${pkgs.coreutils}/bin/date +%s)"

    # HELIOS_START runs asynchronously in tests/helpers; wait for pid file creation
    # so readiness checks do not fail before startup has finished writing runtime state.
    while true; do
      PID=""
      PID_STATE="missing"
      if [ -f "$HELIOS_PID_FILE" ]; then
        PID=$(cat "$HELIOS_PID_FILE" 2>/dev/null || true)
        if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
          break
        fi
        PID_STATE="stale"
      fi

      now_ts="$(${pkgs.coreutils}/bin/date +%s)"
      if [ $((now_ts - start_ts)) -ge "$TIMEOUT_SECS" ]; then
        if [ "$PID_STATE" = "stale" ]; then
          echo "ERROR: helios not running (stale pid file) pid_file=$HELIOS_PID_FILE pid=''${PID:-unknown}" >&2
        else
          echo "ERROR: helios not running (missing pid file) pid_file=$HELIOS_PID_FILE" >&2
        fi
        exit 1
      fi

      ${pkgs.coreutils}/bin/sleep "$INTERVAL_SECS"
    done

    attempt=0

    while true; do
      attempt=$((attempt + 1))

      RESP="$(${pkgs.curl}/bin/curl -fsS --max-time 2 \
        -H 'content-type: application/json' \
        --data '{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}' \
        "http://127.0.0.1:$HELIOS_RPC_PORT" 2>/dev/null || true)"

      if [ -n "$RESP" ] && echo "$RESP" | ${pkgs.jq}/bin/jq -e '.result | strings' >/dev/null 2>&1; then
        echo "OK: helios ready rpc_port=$HELIOS_RPC_PORT"
        exit 0
      fi

      if [ "$HELIOS_NETWORK" = "local" ]; then
        if ${healthCheck}
        then
          echo "OK: helios ready rpc_port=$HELIOS_RPC_PORT mode=local_chainid_fallback"
          exit 0
        fi
      fi

      if [ $((attempt % 10)) -eq 0 ]; then
        ${processRegistry.emitEvent} \
          --event-type readiness_progress \
          --service helios \
          --state waiting \
          --slot "$SLOT" \
          --env "$ENV" \
          --pid "$PID" \
          --log-path "$HELIOS_LOG_FILE" \
          --wait-reason "helios_ready_attempt=$attempt" >/dev/null 2>&1 || true

        ERR_MSG=""
        if [ -n "$RESP" ]; then
          ERR_MSG="$(echo "$RESP" | ${pkgs.jq}/bin/jq -r '.error.message // empty' 2>/dev/null || true)"
        fi

        SYNC_STATUS="$(${pkgs.curl}/bin/curl -fsS --max-time 2 \
          -H 'content-type: application/json' \
          --data '{"jsonrpc":"2.0","id":1,"method":"eth_syncing","params":[]}' \
          "http://127.0.0.1:$HELIOS_RPC_PORT" 2>/dev/null \
          | ${pkgs.jq}/bin/jq -r '.result | if type == "object" then "\(.currentBlock)/\(.highestBlock)" else "not_syncing" end' 2>/dev/null \
          || true)"

        if [ -n "''${ERR_MSG:-}" ] && [ -n "''${SYNC_STATUS:-}" ]; then
          echo "INFO: helios not ready yet: $ERR_MSG (eth_syncing=$SYNC_STATUS)" >&2
        elif [ -n "''${ERR_MSG:-}" ]; then
          echo "INFO: helios not ready yet: $ERR_MSG" >&2
        elif [ -n "''${SYNC_STATUS:-}" ]; then
          echo "INFO: helios eth_syncing=$SYNC_STATUS" >&2
        fi
      fi

      now_ts="$(${pkgs.coreutils}/bin/date +%s)"
      if [ $((now_ts - start_ts)) -ge "$TIMEOUT_SECS" ]; then
        echo "ERROR: helios not ready after $TIMEOUT_SECS s (eth_blockNumber still failing) rpc_port=$HELIOS_RPC_PORT" >&2
        echo "HINT: set HELIOS_CHECKPOINT and HELIOS_CONSENSUS_RPC_URL explicitly for mainnet." >&2
        exit 1
      fi

      ${pkgs.coreutils}/bin/sleep "$INTERVAL_SECS"
    done
  '';

  fullStart = pkgs.writeShellScript "helios-full-start" ''
    set -euo pipefail

    ${init}
    ${checkConfig}
    exec ${start}
  '';

  fullStartTest = pkgs.writeShellScript "helios-full-start-test" ''
    set -euo pipefail

    ${init}
    ${checkConfig}
    exec ${start}
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
    ready
    fullStart
    fullStartTest
    ;
}
