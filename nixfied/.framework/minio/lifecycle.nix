# MinIO lifecycle management - init, start, stop, restart, status, health, check-config
{
  pkgs,
  project,
  slots,
  config,
}:

let
  cfg = project.modules.minio or { };
  minio = cfg.package or pkgs.minio;
  apiPortVar = slots.portVarName config.portKeyApi;
  consolePortVar = slots.portVarName config.portKeyConsole;
  minioDirExpr = slots.getServiceDir config.dataDirName;
  browserValue = if config.browser then "on" else "off";

  init = pkgs.writeShellScript "minio-init" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    MINIO_DIR="${minioDirExpr}"
    mkdir -p "$MINIO_DIR/data"
    mkdir -p "$MINIO_DIR/config"
    mkdir -p "$MINIO_DIR/run"
    mkdir -p "$MINIO_DIR/logs"

    echo "OK: minio initialized dir=$MINIO_DIR slot=$SLOT env=$ENV"
  '';

  start = pkgs.writeShellScript "minio-start" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    API_PORT_VAR="${apiPortVar}"
    CONSOLE_PORT_VAR="${consolePortVar}"

    MINIO_API_PORT="''${!API_PORT_VAR}"
    MINIO_CONSOLE_PORT="''${!CONSOLE_PORT_VAR}"
    MINIO_DIR="${minioDirExpr}"
    MINIO_PID_FILE="$MINIO_DIR/run/minio.pid"
    LOG_FILE="$MINIO_DIR/logs/minio.log"

    ${init}

    if [ -f "$MINIO_PID_FILE" ]; then
      PID=$(cat "$MINIO_PID_FILE" 2>/dev/null || true)
      if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        echo "OK: minio already running pid=$PID api_port=$MINIO_API_PORT"
        exit 0
      fi
      rm -f "$MINIO_PID_FILE"
    fi

    ROOT_USER="''${MINIO_ROOT_USER:-${config.rootUser}}"
    ROOT_PASSWORD="''${MINIO_ROOT_PASSWORD:-${config.rootPassword}}"

    export MINIO_ROOT_USER="$ROOT_USER"
    export MINIO_ROOT_PASSWORD="$ROOT_PASSWORD"
    export MINIO_BROWSER="${browserValue}"

    ${minio}/bin/minio server "$MINIO_DIR/data" \
      --address "127.0.0.1:$MINIO_API_PORT" \
      --console-address "127.0.0.1:$MINIO_CONSOLE_PORT" \
      --config-dir "$MINIO_DIR/config" \
      > "$LOG_FILE" 2>&1 &
    CHILD_PID=$!
    echo "$CHILD_PID" > "$MINIO_PID_FILE"

    cleanup() {
      if [ -n "''${CHILD_PID:-}" ] && kill -0 "$CHILD_PID" 2>/dev/null; then
        kill "$CHILD_PID" 2>/dev/null || true
        wait "$CHILD_PID" 2>/dev/null || true
      fi
      rm -f "$MINIO_PID_FILE"
    }

    trap cleanup EXIT INT TERM

    echo "INFO: minio started pid=$CHILD_PID api_port=$MINIO_API_PORT console_port=$MINIO_CONSOLE_PORT"
    wait "$CHILD_PID"
  '';

  stop = pkgs.writeShellScript "minio-stop" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    MINIO_DIR="${minioDirExpr}"
    MINIO_PID_FILE="$MINIO_DIR/run/minio.pid"

    if [ ! -f "$MINIO_PID_FILE" ]; then
      echo "OK: minio not running"
      exit 0
    fi

    PID=$(cat "$MINIO_PID_FILE" 2>/dev/null || true)
    if [ -z "$PID" ] || ! kill -0 "$PID" 2>/dev/null; then
      rm -f "$MINIO_PID_FILE"
      echo "OK: minio pid file cleaned"
      exit 0
    fi

    kill "$PID" 2>/dev/null || true

    for _ in $(seq 1 20); do
      if ! kill -0 "$PID" 2>/dev/null; then
        rm -f "$MINIO_PID_FILE"
        echo "OK: minio stopped pid=$PID"
        exit 0
      fi
      sleep 0.2
    done

    kill -KILL "$PID" 2>/dev/null || true
    rm -f "$MINIO_PID_FILE"
    echo "WARN: minio force-killed pid=$PID"
  '';

  restart = pkgs.writeShellScript "minio-restart" ''
    set -euo pipefail

    ${stop}
    exec ${start}
  '';

  status = pkgs.writeShellScript "minio-status" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    API_PORT_VAR="${apiPortVar}"
    CONSOLE_PORT_VAR="${consolePortVar}"

    MINIO_API_PORT="''${!API_PORT_VAR}"
    MINIO_CONSOLE_PORT="''${!CONSOLE_PORT_VAR}"
    MINIO_DIR="${minioDirExpr}"
    MINIO_PID_FILE="$MINIO_DIR/run/minio.pid"

    RUNNING=false
    PID=""

    if [ -f "$MINIO_PID_FILE" ]; then
      PID=$(cat "$MINIO_PID_FILE" 2>/dev/null || true)
      if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        RUNNING=true
      fi
    fi

    echo "service=minio slot=$SLOT env=$ENV running=$RUNNING pid=''${PID:-unknown} api_port=$MINIO_API_PORT console_port=$MINIO_CONSOLE_PORT"

    if [ "$RUNNING" = "true" ]; then
      exit 0
    fi
    exit 1
  '';

  health = pkgs.writeShellScript "minio-health" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    API_PORT_VAR="${apiPortVar}"
    MINIO_API_PORT="''${!API_PORT_VAR}"

    if ${pkgs.curl}/bin/curl -fsS --max-time 2 "http://127.0.0.1:$MINIO_API_PORT/minio/health/live" >/dev/null 2>&1; then
      echo "OK: minio healthy api_port=$MINIO_API_PORT"
      exit 0
    fi

    echo "ERROR: minio unhealthy api_port=$MINIO_API_PORT" >&2
    exit 1
  '';

  checkConfig = pkgs.writeShellScript "minio-check-config" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    MINIO_DIR="${minioDirExpr}"

    if [ ! -d "$MINIO_DIR/config" ]; then
      echo "ERROR: missing minio config directory at $MINIO_DIR/config" >&2
      exit 1
    fi

    ${minio}/bin/minio --help >/dev/null
    echo "OK: minio configuration valid dir=$MINIO_DIR"
  '';
in
{
  inherit
    minio
    init
    start
    stop
    restart
    status
    health
    checkConfig
    ;
}
