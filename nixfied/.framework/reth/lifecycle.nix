# Reth lifecycle management - init, start, stop, restart, status, health, check-config
{
  pkgs,
  project,
  slots,
  config,
}:

let
  lib = pkgs.lib;
  reth = config.package or (project.modules.reth.package or pkgs.reth);
  httpPortVar = slots.portVarName config.portKeyHttp;
  wsPortVar = slots.portVarName config.portKeyWs;
  authPortVar = slots.portVarName config.portKeyAuth;
  rethDirExpr = slots.getServiceDir config.dataDirName;
  useDevMode = config.devMode or false;
  extraArgs = lib.escapeShellArgs (config.extraArgs or [ ]);

  runtimePrelude = ''
    SLOT_INFO_OUT="$(${slots.getSlotInfo})" || exit 1
    eval "$SLOT_INFO_OUT"

    HTTP_PORT_VAR="${httpPortVar}"
    WS_PORT_VAR="${wsPortVar}"
    AUTH_PORT_VAR="${authPortVar}"

    RETH_HTTP_PORT="''${!HTTP_PORT_VAR:-}"
    RETH_WS_PORT="''${!WS_PORT_VAR:-}"
    RETH_AUTH_PORT="''${!AUTH_PORT_VAR:-}"
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
      echo "ERROR: reth port variables are not set (http/ws/auth)" >&2
      exit 1
    fi
  '';

  healthCheck = ''
    ${pkgs.curl}/bin/curl -fsS --max-time 2 \
      -H 'content-type: application/json' \
      --data '{"jsonrpc":"2.0","id":1,"method":"web3_clientVersion","params":[]}' \
      "http://127.0.0.1:$RETH_HTTP_PORT" \
      | ${pkgs.gnugrep}/bin/grep -q '"result"'
  '';

  init = pkgs.writeShellScript "reth-init" ''
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

    echo "OK: reth initialized dir=$RETH_DIR slot=$SLOT env=$ENV"
  '';

  start = pkgs.writeShellScript "reth-start" ''
    set -euo pipefail
    ${runtimePrelude}

    ${init}

    if [ -f "$RETH_PID_FILE" ]; then
      PID=$(cat "$RETH_PID_FILE" 2>/dev/null || true)
      if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        echo "OK: reth already running pid=$PID http_port=$RETH_HTTP_PORT"
        exit 0
      fi
      rm -f "$RETH_PID_FILE"
    fi

    if [ ! -x "${reth}/bin/reth" ]; then
      echo "ERROR: reth binary not executable at ${reth}/bin/reth" >&2
      exit 1
    fi

    ARGS=(
      node
      --datadir "$RETH_DIR/data"
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
      echo "ERROR: reth failed to become healthy. log=$RETH_LOG_FILE" >&2
      tail -50 "$RETH_LOG_FILE" >&2 || true
      exit 1
    fi

    echo "INFO: reth started pid=$CHILD_PID http_port=$RETH_HTTP_PORT ws_port=$RETH_WS_PORT auth_port=$RETH_AUTH_PORT"
    wait "$CHILD_PID"
  '';

  stop = pkgs.writeShellScript "reth-stop" ''
    set -euo pipefail
    ${runtimePrelude}

    if [ ! -f "$RETH_PID_FILE" ]; then
      echo "OK: reth not running"
      exit 0
    fi

    PID=$(cat "$RETH_PID_FILE" 2>/dev/null || true)
    if [ -z "$PID" ] || ! kill -0 "$PID" 2>/dev/null; then
      rm -f "$RETH_PID_FILE"
      echo "OK: reth pid file cleaned"
      exit 0
    fi

    kill "$PID" 2>/dev/null || true
    for _ in $(seq 1 40); do
      if ! kill -0 "$PID" 2>/dev/null; then
        rm -f "$RETH_PID_FILE"
        echo "OK: reth stopped pid=$PID"
        exit 0
      fi
      sleep 0.25
    done

    kill -KILL "$PID" 2>/dev/null || true
    rm -f "$RETH_PID_FILE"
    echo "WARN: reth force-killed pid=$PID"
  '';

  restart = pkgs.writeShellScript "reth-restart" ''
    set -euo pipefail

    ${stop}
    exec ${start}
  '';

  status = pkgs.writeShellScript "reth-status" ''
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

    echo "service=reth slot=$SLOT env=$ENV running=$RUNNING pid=''${PID:-unknown} http_port=$RETH_HTTP_PORT ws_port=$RETH_WS_PORT auth_port=$RETH_AUTH_PORT network=$RETH_NETWORK"

    if [ "$RUNNING" = "true" ]; then
      exit 0
    fi
    exit 1
  '';

  health = pkgs.writeShellScript "reth-health" ''
    set -euo pipefail
    ${runtimePrelude}

    if ${healthCheck}
    then
      echo "OK: reth healthy http_port=$RETH_HTTP_PORT"
      exit 0
    fi

    echo "ERROR: reth unhealthy http_port=$RETH_HTTP_PORT" >&2
    exit 1
  '';

  checkConfig = pkgs.writeShellScript "reth-check-config" ''
    set -euo pipefail
    ${runtimePrelude}

    if [ ! -x "${reth}/bin/reth" ]; then
      echo "ERROR: missing reth binary at ${reth}/bin/reth" >&2
      exit 1
    fi

    mkdir -p "$RETH_DIR/config"
    ${reth}/bin/reth --version >/dev/null 2>&1
    echo "OK: reth configuration valid dir=$RETH_DIR network=$RETH_NETWORK"
  '';

  fullStart = pkgs.writeShellScript "reth-full-start" ''
    set -euo pipefail

    ${init}
    ${checkConfig}
    exec ${start}
  '';

  fullStartTest = pkgs.writeShellScript "reth-full-start-test" ''
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
    fullStart
    fullStartTest
    ;
}
