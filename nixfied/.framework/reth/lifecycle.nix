# Reth lifecycle management - init, start, stop, restart, status, health, check-config
{
  pkgs,
  project,
  slots,
  config,
  loggingPrelude,
}:

let
  lib = pkgs.lib;
  slotEnvRuntime = import ../lib/slot-env-runtime.nix { inherit pkgs; };
  processRegistry = import ../lib/process-registry.nix { inherit pkgs project; };
  observability = import ../lib/service-observability.nix {
    inherit
      pkgs
      slots
      processRegistry
      ;
  };
  reth = config.package or (project.modules.reth.package or pkgs.reth);
  httpPortVar = slots.portVarName config.portKeyHttp;
  wsPortVar = slots.portVarName config.portKeyWs;
  authPortVar = slots.portVarName config.portKeyAuth;
  rethDirExpr = slots.getServiceDir config.dataDirName;
  useDevMode = config.devMode or false;
  extraArgs = lib.escapeShellArgs (config.extraArgs or [ ]);

  runtimePrelude = ''
    ${slotEnvRuntime.loadJsonFromCommand {
      outVar = "SLOT_INFO_JSON_OUT";
      command = toString slots.getSlotInfoJson;
      exportVars = false;
    }}

    HTTP_PORT_VAR="${httpPortVar}"
    WS_PORT_VAR="${wsPortVar}"
    AUTH_PORT_VAR="${authPortVar}"

    ${slotEnvRuntime.readPortFromJson {
      targetVar = "RETH_HTTP_PORT";
      jsonVar = "SLOT_INFO_JSON_OUT";
      keyExpr = "$HTTP_PORT_VAR";
    }}
    ${slotEnvRuntime.readPortFromJson {
      targetVar = "RETH_WS_PORT";
      jsonVar = "SLOT_INFO_JSON_OUT";
      keyExpr = "$WS_PORT_VAR";
    }}
    ${slotEnvRuntime.readPortFromJson {
      targetVar = "RETH_AUTH_PORT";
      jsonVar = "SLOT_INFO_JSON_OUT";
      keyExpr = "$AUTH_PORT_VAR";
    }}
    RETH_DIR="${rethDirExpr}"
    RETH_PID_FILE="$RETH_DIR/run/reth.pid"
    RETH_LOG_FILE="$RETH_DIR/logs/reth.log"
    RETH_JWT_FILE="$RETH_DIR/config/jwt.hex"
    RETH_NETWORK="''${RETH_NETWORK:-${config.network or "local"}}"
    RETH_USE_DEV="${if useDevMode then "1" else "0"}"

    if [ "$RETH_NETWORK" = "local" ]; then
      RETH_USE_DEV="1"
    fi

    if [ -z "$RETH_HTTP_PORT" ] || [ -z "$RETH_WS_PORT" ] || [ -z "$RETH_AUTH_PORT" ]; then
      log_error "reth port variables are not set (http/ws/auth)"
      exit 1
    fi

    ${observability.mkEmitServiceEventFunction "reth"}
  '';

  healthCheck = ''
    ${pkgs.curl}/bin/curl -fsS --max-time 2 \
      -H 'content-type: application/json' \
      --data '{"jsonrpc":"2.0","id":1,"method":"web3_clientVersion","params":[]}' \
      "http://127.0.0.1:$RETH_HTTP_PORT" \
      | ${pkgs.gnugrep}/bin/grep -q '"result"'
  '';

  init = pkgs.writeShellScript "reth-init" ''
    ${loggingPrelude}

    set -euo pipefail
    ${runtimePrelude}

    mkdir -p "$RETH_DIR/data"
    mkdir -p "$RETH_DIR/config"
    mkdir -p "$RETH_DIR/run"
    mkdir -p "$RETH_DIR/logs"

    if [ ! -f "$RETH_JWT_FILE" ]; then
      printf '%064x\n' 0 > "$RETH_JWT_FILE"
    fi
    chmod 600 "$RETH_JWT_FILE" 2>/dev/null || true

    log_ok "reth initialized dir=$RETH_DIR slot=$SLOT env=$ENV"
  '';

  start = pkgs.writeShellScript "reth-start" ''
    ${loggingPrelude}

    set -euo pipefail
    ${runtimePrelude}

    ${init}

    if [ -f "$RETH_PID_FILE" ]; then
      PID=$(cat "$RETH_PID_FILE" 2>/dev/null || true)
      if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        emit_service_event service_ready ready --pid "$PID" --log-path "$RETH_LOG_FILE"
        log_ok "reth already running pid=$PID http_port=$RETH_HTTP_PORT"
        exit 0
      fi
      rm -f "$RETH_PID_FILE"
    fi

    if [ ! -x "${reth}/bin/reth" ]; then
      log_error "reth binary not executable at ${reth}/bin/reth"
      exit 1
    fi

    ARGS=(
      node
      --datadir "$RETH_DIR/data"
      --ipcpath "$RETH_DIR/run/reth.ipc"
      --http
      --http.addr 127.0.0.1
      --http.port "$RETH_HTTP_PORT"
      --ws
      --ws.addr 127.0.0.1
      --ws.port "$RETH_WS_PORT"
      --authrpc.addr 127.0.0.1
      --authrpc.port "$RETH_AUTH_PORT"
      --authrpc.jwtsecret "$RETH_JWT_FILE"
    )

    if [ "$RETH_USE_DEV" = "1" ]; then
      ARGS+=(--dev)
    else
      ARGS+=(--chain "$RETH_NETWORK")
    fi

    ${lib.optionalString ((config.extraArgs or [ ]) != [ ]) ''
      EXTRA_ARGS=(${extraArgs})
      ARGS+=("''${EXTRA_ARGS[@]}")
    ''}

    "${reth}/bin/reth" "''${ARGS[@]}" > "$RETH_LOG_FILE" 2>&1 &
    CHILD_PID=$!
    echo "$CHILD_PID" > "$RETH_PID_FILE"

    emit_service_event service_starting starting --pid "$CHILD_PID" --log-path "$RETH_LOG_FILE"

    cleanup() {
      if [ -n "''${CHILD_PID:-}" ] && kill -0 "$CHILD_PID" 2>/dev/null; then
        kill "$CHILD_PID" 2>/dev/null || true
        wait "$CHILD_PID" 2>/dev/null || true
      fi
      rm -f "$RETH_PID_FILE"
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
      emit_service_event service_degraded degraded \
        --pid "$CHILD_PID" \
        --log-path "$RETH_LOG_FILE" \
        --wait-reason "failed_readiness" \
        --last-error "reth failed health check during startup"
      log_error "reth failed to become healthy. log=$RETH_LOG_FILE"
      if [ -f "$RETH_LOG_FILE" ]; then
        log_info "reth log tail path=$RETH_LOG_FILE lines=50"
        tail -50 "$RETH_LOG_FILE" >&2 || true
      else
        log_warn "reth log file missing path=$RETH_LOG_FILE"
      fi
      exit 1
    fi

    emit_service_event service_ready ready --pid "$CHILD_PID" --log-path "$RETH_LOG_FILE"

    log_info "reth started pid=$CHILD_PID http_port=$RETH_HTTP_PORT ws_port=$RETH_WS_PORT auth_port=$RETH_AUTH_PORT"
    set +e
    wait "$CHILD_PID"
    RC=$?
    set -e

    if [ "$RC" -eq 0 ]; then
      emit_service_event service_stopped stopped --pid "$CHILD_PID" --log-path "$RETH_LOG_FILE"
    else
      emit_service_event service_degraded degraded \
        --pid "$CHILD_PID" \
        --log-path "$RETH_LOG_FILE" \
        --wait-reason "reth_process_exit code=$RC" \
        --last-error "reth process exited non-zero"
    fi
    exit "$RC"
  '';

  stop = pkgs.writeShellScript "reth-stop" ''
    ${loggingPrelude}

    set -euo pipefail
    ${runtimePrelude}

    if [ ! -f "$RETH_PID_FILE" ]; then
      emit_service_event service_stopped stopped --log-path "$RETH_LOG_FILE"
      log_ok "reth not running"
      exit 0
    fi

    PID=$(cat "$RETH_PID_FILE" 2>/dev/null || true)
    if [ -z "$PID" ] || ! kill -0 "$PID" 2>/dev/null; then
      rm -f "$RETH_PID_FILE"
      emit_service_event service_stopped stopped --pid "$PID" --log-path "$RETH_LOG_FILE"
      log_ok "reth pid file cleaned"
      exit 0
    fi

    kill "$PID" 2>/dev/null || true
    for _ in $(seq 1 40); do
      if ! kill -0 "$PID" 2>/dev/null; then
        rm -f "$RETH_PID_FILE"
        emit_service_event service_stopped stopped --pid "$PID" --log-path "$RETH_LOG_FILE"
        log_ok "reth stopped pid=$PID"
        exit 0
      fi
      sleep 0.25
    done

    kill -KILL "$PID" 2>/dev/null || true
    rm -f "$RETH_PID_FILE"
    emit_service_event service_stopped stopped --pid "$PID" --log-path "$RETH_LOG_FILE"
    log_warn "reth force-killed pid=$PID"
  '';

  restart = pkgs.writeShellScript "reth-restart" ''
    ${loggingPrelude}

    set -euo pipefail

    ${stop}
    exec ${start}
  '';

  status = pkgs.writeShellScript "reth-status" ''
    ${loggingPrelude}

    set -euo pipefail
    ${runtimePrelude}

    RUNNING=false
    PID=""

    if [ -f "$RETH_PID_FILE" ]; then
      PID=$(cat "$RETH_PID_FILE" 2>/dev/null || true)
      if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        RUNNING=true
      fi
    fi

    ${observability.mkStatusMergeBlock {
      service = "reth";
      defaultLogPathExpr = ''"$RETH_LOG_FILE"'';
    }}

    echo "service=reth slot=$SLOT env=$ENV running=$RUNNING pid=''${PID:-unknown} http_port=$RETH_HTTP_PORT ws_port=$RETH_WS_PORT auth_port=$RETH_AUTH_PORT network=$RETH_NETWORK scope=$SCOPE owner_run_id=''${OWNER_RUN_ID:-unknown} owner_scope=''${OWNER_SCOPE:-unknown} ephemeral_root=''${EPHEMERAL_ROOT:-none} registry_state=''${REGISTRY_STATE:-unknown} slot_owner=''${SLOT_OWNER:-unknown} wait_reason=''${WAIT_REASON:-none} log_path=$EFFECTIVE_LOG_PATH"

    if [ "$RUNNING" = "true" ]; then
      exit 0
    fi
    exit 1
  '';

  health = pkgs.writeShellScript "reth-health" ''
    ${loggingPrelude}

    set -euo pipefail
    ${runtimePrelude}

    if ${healthCheck}
    then
      log_ok "reth healthy http_port=$RETH_HTTP_PORT"
      exit 0
    fi

    log_error "reth unhealthy http_port=$RETH_HTTP_PORT"
    exit 1
  '';

  ready = pkgs.writeShellScript "reth-ready" ''
    ${loggingPrelude}

    set -euo pipefail
    ${runtimePrelude}

    if ${health} >/dev/null 2>&1; then
      log_ok "reth ready http_port=$RETH_HTTP_PORT"
      exit 0
    fi

    log_error "reth not ready http_port=$RETH_HTTP_PORT"
    exit 1
  '';

  checkConfig = pkgs.writeShellScript "reth-check-config" ''
    ${loggingPrelude}

    set -euo pipefail
    ${runtimePrelude}

    if [ ! -x "${reth}/bin/reth" ]; then
      log_error "missing reth binary at ${reth}/bin/reth"
      exit 1
    fi

    mkdir -p "$RETH_DIR/config"
    ${reth}/bin/reth --version >/dev/null 2>&1
    log_ok "reth configuration valid dir=$RETH_DIR network=$RETH_NETWORK"
  '';

  fullStart = pkgs.writeShellScript "reth-full-start" ''
    ${loggingPrelude}

    set -euo pipefail

    ${init}
    ${checkConfig}
    exec ${start}
  '';

  fullStartTest = pkgs.writeShellScript "reth-full-start-test" ''
    ${loggingPrelude}

    set -euo pipefail

    ${init}
    ${checkConfig}
    exec ${start}
  '';
in
{
  inherit
    reth
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
