# MinIO lifecycle management - init, start, stop, restart, status, health, check-config
{
  pkgs,
  project,
  slots,
  config,
}:

let
  cfg = project.modules.minio or { };
  slotEnvRuntime = import ../lib/slot-env-runtime.nix { inherit pkgs; };
  processRegistry = import ../lib/process-registry.nix { inherit pkgs project; };
  observability = import ../lib/service-observability.nix {
    inherit
      pkgs
      slots
      processRegistry
      ;
  };
  minio = cfg.package or pkgs.minio;
  apiPortVar = slots.portVarName config.portKeyApi;
  consolePortVar = slots.portVarName config.portKeyConsole;
  minioDirExpr = slots.getServiceDir config.dataDirName;
  browserValue = if config.browser then "on" else "off";
  emitHelper = observability.mkEmitServiceEventFunction "minio";
  runtimePrelude = ''
    ${slotEnvRuntime.loadJsonFromCommand {
      outVar = "SLOT_INFO_JSON_OUT";
      command = toString slots.getSlotInfoJson;
      exportVars = false;
    }}

    API_PORT_VAR="${apiPortVar}"
    CONSOLE_PORT_VAR="${consolePortVar}"

    ${slotEnvRuntime.readPortFromJson {
      targetVar = "MINIO_API_PORT";
      jsonVar = "SLOT_INFO_JSON_OUT";
      keyExpr = "$API_PORT_VAR";
    }}
    ${slotEnvRuntime.readPortFromJson {
      targetVar = "MINIO_CONSOLE_PORT";
      jsonVar = "SLOT_INFO_JSON_OUT";
      keyExpr = "$CONSOLE_PORT_VAR";
    }}
    MINIO_DIR="${minioDirExpr}"
    MINIO_PID_FILE="$MINIO_DIR/run/minio.pid"
    MINIO_LOG_FILE="$MINIO_DIR/logs/minio.log"

    if [ -z "$MINIO_API_PORT" ] || [ -z "$MINIO_CONSOLE_PORT" ]; then
      echo "ERROR: minio port variables are not set (api/console)" >&2
      exit 1
    fi

    ${emitHelper}
  '';

  init = pkgs.writeShellScript "minio-init" ''
    set -euo pipefail
    ${runtimePrelude}

    mkdir -p "$MINIO_DIR/data"
    mkdir -p "$MINIO_DIR/config"
    mkdir -p "$MINIO_DIR/run"
    mkdir -p "$MINIO_DIR/logs"

    echo "OK: minio initialized dir=$MINIO_DIR slot=$SLOT env=$ENV"
  '';

  start = pkgs.writeShellScript "minio-start" ''
    set -euo pipefail
    ${runtimePrelude}

    LOG_FILE="$MINIO_LOG_FILE"

    ${init}

    if [ -f "$MINIO_PID_FILE" ]; then
      PID=$(cat "$MINIO_PID_FILE" 2>/dev/null || true)
      if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        emit_service_event service_ready ready --pid "$PID" --log-path "$LOG_FILE"
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

    emit_service_event service_starting starting --pid "$CHILD_PID" --log-path "$LOG_FILE"

    cleanup() {
      if [ -n "''${CHILD_PID:-}" ] && kill -0 "$CHILD_PID" 2>/dev/null; then
        kill "$CHILD_PID" 2>/dev/null || true
        wait "$CHILD_PID" 2>/dev/null || true
      fi
      rm -f "$MINIO_PID_FILE"
    }

    trap cleanup EXIT INT TERM

    emit_service_event service_ready ready --pid "$CHILD_PID" --log-path "$LOG_FILE"

    echo "INFO: minio started pid=$CHILD_PID api_port=$MINIO_API_PORT console_port=$MINIO_CONSOLE_PORT"
    set +e
    wait "$CHILD_PID"
    RC=$?
    set -e

    if [ "$RC" -eq 0 ]; then
      emit_service_event service_stopped stopped --pid "$CHILD_PID" --log-path "$LOG_FILE"
    else
      emit_service_event service_degraded degraded \
        --pid "$CHILD_PID" \
        --log-path "$LOG_FILE" \
        --wait-reason "minio_process_exit code=$RC" \
        --last-error "minio process exited non-zero"
    fi
    exit "$RC"
  '';

  stop = pkgs.writeShellScript "minio-stop" ''
    set -euo pipefail
    ${runtimePrelude}

    if [ ! -f "$MINIO_PID_FILE" ]; then
      emit_service_event service_stopped stopped
      echo "OK: minio not running"
      exit 0
    fi

    PID=$(cat "$MINIO_PID_FILE" 2>/dev/null || true)
    if [ -z "$PID" ] || ! kill -0 "$PID" 2>/dev/null; then
      rm -f "$MINIO_PID_FILE"
      emit_service_event service_stopped stopped --pid "$PID"
      echo "OK: minio pid file cleaned"
      exit 0
    fi

    kill "$PID" 2>/dev/null || true

    for _ in $(seq 1 20); do
      if ! kill -0 "$PID" 2>/dev/null; then
        rm -f "$MINIO_PID_FILE"
        emit_service_event service_stopped stopped --pid "$PID"
        echo "OK: minio stopped pid=$PID"
        exit 0
      fi
      sleep 0.2
    done

    kill -KILL "$PID" 2>/dev/null || true
    rm -f "$MINIO_PID_FILE"
    emit_service_event service_stopped stopped --pid "$PID"
    echo "WARN: minio force-killed pid=$PID"
  '';

  restart = pkgs.writeShellScript "minio-restart" ''
    set -euo pipefail

    ${stop}
    exec ${start}
  '';

  status = pkgs.writeShellScript "minio-status" ''
    set -euo pipefail
    ${runtimePrelude}

    RUNNING=false
    PID=""

    if [ -f "$MINIO_PID_FILE" ]; then
      PID=$(cat "$MINIO_PID_FILE" 2>/dev/null || true)
      if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        RUNNING=true
      fi
    fi

    ${observability.mkStatusMergeBlock {
      service = "minio";
      defaultLogPathExpr = ''"$MINIO_LOG_FILE"'';
    }}

    echo "service=minio slot=$SLOT env=$ENV running=$RUNNING pid=''${PID:-unknown} api_port=$MINIO_API_PORT console_port=$MINIO_CONSOLE_PORT scope=$SCOPE owner_run_id=''${OWNER_RUN_ID:-unknown} owner_scope=''${OWNER_SCOPE:-unknown} ephemeral_root=''${EPHEMERAL_ROOT:-none} registry_state=''${REGISTRY_STATE:-unknown} slot_owner=''${SLOT_OWNER:-unknown} wait_reason=''${WAIT_REASON:-none} log_path=$EFFECTIVE_LOG_PATH"

    if [ "$RUNNING" = "true" ]; then
      exit 0
    fi
    exit 1
  '';

  health = pkgs.writeShellScript "minio-health" ''
    set -euo pipefail
    ${runtimePrelude}

    if ${pkgs.curl}/bin/curl -fsS --max-time 2 "http://127.0.0.1:$MINIO_API_PORT/minio/health/live" >/dev/null 2>&1; then
      echo "OK: minio healthy api_port=$MINIO_API_PORT"
      exit 0
    fi

    echo "ERROR: minio unhealthy api_port=$MINIO_API_PORT" >&2
    exit 1
  '';

  ready = pkgs.writeShellScript "minio-ready" ''
    set -euo pipefail
    ${runtimePrelude}

    if ${pkgs.curl}/bin/curl -fsS --max-time 2 "http://127.0.0.1:$MINIO_API_PORT/minio/health/ready" >/dev/null 2>&1; then
      echo "OK: minio ready api_port=$MINIO_API_PORT"
      exit 0
    fi

    echo "ERROR: minio not ready api_port=$MINIO_API_PORT" >&2
    exit 1
  '';

  checkConfig = pkgs.writeShellScript "minio-check-config" ''
    set -euo pipefail
    ${runtimePrelude}

    if [ ! -d "$MINIO_DIR/config" ]; then
      echo "ERROR: missing minio config directory at $MINIO_DIR/config" >&2
      exit 1
    fi

    ${minio}/bin/minio --help >/dev/null
    echo "OK: minio configuration valid dir=$MINIO_DIR"
  '';

  fullStart = pkgs.writeShellScript "minio-full-start" ''
    set -euo pipefail

    ${init}
    ${checkConfig}
    exec ${start}
  '';

  fullStartTest = pkgs.writeShellScript "minio-full-start-test" ''
    set -euo pipefail

    ${init}
    ${checkConfig}
    exec ${start}
  '';

  exportS3Env = pkgs.writeShellScript "minio-export-s3-env" ''
    set -euo pipefail
    ${runtimePrelude}

    BUCKET="''${1:-''${MINIO_BUCKET:-}}"
    PREFIX="''${2:-''${MINIO_PREFIX:-}}"
    REGION="''${3:-''${MINIO_REGION:-us-east-1}}"

    ROOT_USER="''${MINIO_ROOT_USER:-${config.rootUser}}"
    ROOT_PASSWORD="''${MINIO_ROOT_PASSWORD:-${config.rootPassword}}"

    echo "export AWS_ACCESS_KEY_ID=\"$ROOT_USER\""
    echo "export AWS_SECRET_ACCESS_KEY=\"$ROOT_PASSWORD\""
    echo "export AWS_EC2_METADATA_DISABLED=\"true\""
    echo "export MINIO_ENDPOINT=\"http://127.0.0.1:$MINIO_API_PORT\""
    echo "export MINIO_REGION=\"$REGION\""
    echo "export MINIO_BUCKET=\"$BUCKET\""
    echo "export MINIO_PREFIX=\"$PREFIX\""
    echo "export S3_ENDPOINT=\"http://127.0.0.1:$MINIO_API_PORT\""
    echo "export S3_REGION=\"$REGION\""
    echo "export S3_BUCKET=\"$BUCKET\""
    echo "export S3_PREFIX=\"$PREFIX\""
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
    ready
    fullStart
    fullStartTest
    exportS3Env
    ;
}
