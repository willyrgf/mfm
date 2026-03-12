# MinIO lifecycle management - init, start, stop, restart, status, health, check-config
{
  pkgs,
  project,
  slots,
  config,
  loggingPrelude,
}:

let
  managedServiceLifecycle = import ../../helpers/managed-service-lifecycle.nix { inherit pkgs; };
  probeCommands = import ../../helpers/probe-commands.nix { inherit pkgs; };
  slotEnvRuntime = import ../../helpers/slot-env-runtime.nix { inherit pkgs; };
  runtimeEvents = import ../../helpers/runtime-events.nix { inherit pkgs project; };
  observability = import ../../helpers/service-observability.nix {
    inherit
      pkgs
      slots
      runtimeEvents
      ;
  };
  minio = config.package or pkgs.minio;
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
    SERVICE_DIR="$MINIO_DIR"
    SERVICE_PID_FILE="$MINIO_PID_FILE"
    SERVICE_LOG_FILE="$MINIO_LOG_FILE"

    if [ -z "$MINIO_API_PORT" ] || [ -z "$MINIO_CONSOLE_PORT" ]; then
      log_error "minio port variables are not set (api/console)"
      exit 1
    fi

    ${emitHelper}
  '';
  managedLifecycle = managedServiceLifecycle.mkPidFileManagedLifecycle {
    service = "minio";
    inherit
      loggingPrelude
      runtimePrelude
      ;
    initBody = ''
      mkdir -p "$SERVICE_DIR/data"
      mkdir -p "$SERVICE_DIR/config"
      mkdir -p "$SERVICE_DIR/run"
      mkdir -p "$SERVICE_DIR/logs"

      log_ok "minio initialized dir=$MINIO_DIR slot=$SLOT env=$ENV"
    '';
    checkConfigBody = ''
      if [ ! -d "$MINIO_DIR/config" ]; then
        log_error "missing minio config directory at $MINIO_DIR/config"
        exit 1
      fi

      ${minio}/bin/minio --help >/dev/null
      log_ok "minio configuration valid dir=$MINIO_DIR"
    '';
    startPreflight = ''
      ROOT_USER="''${MINIO_ROOT_USER:-${config.rootUser}}"
      ROOT_PASSWORD="''${MINIO_ROOT_PASSWORD:-${config.rootPassword}}"

      export MINIO_ROOT_USER="$ROOT_USER"
      export MINIO_ROOT_PASSWORD="$ROOT_PASSWORD"
      export MINIO_BROWSER="${browserValue}"
    '';
    startCommand = ''
      ${minio}/bin/minio server "$MINIO_DIR/data" \
        --address "127.0.0.1:$MINIO_API_PORT" \
        --console-address "127.0.0.1:$MINIO_CONSOLE_PORT" \
        --config-dir "$MINIO_DIR/config" \
        > "$LOG_FILE" 2>&1 &
    '';
    startAlreadyRunningBody = managedServiceLifecycle.mkReadyOutcomeBody {
      level = "ok";
      pidExpr = ''"$PID"'';
      message = "minio already running pid=$PID api_port=$MINIO_API_PORT";
    };
    startPostLaunchBody = managedServiceLifecycle.mkReadyOutcomeBody {
      level = "info";
      pidExpr = ''"$CHILD_PID"'';
      message = "minio started pid=$CHILD_PID api_port=$MINIO_API_PORT console_port=$MINIO_CONSOLE_PORT";
    };
    startExitFailureBody = managedServiceLifecycle.mkProcessExitFailureBody {
      waitReason = "minio_process_exit code=$RC";
      lastError = "minio process exited non-zero";
    };
    stopMissingLogPathExpr = null;
    stopStateLogPathExpr = null;
    statusMergeBlock = observability.mkStatusMergeBlock {
      service = "minio";
      defaultLogPathExpr = ''"$MINIO_LOG_FILE"'';
    };
    statusBody = observability.mkStatusLine {
      service = "minio";
      afterPidFields = [
        "api_port=$MINIO_API_PORT"
        "console_port=$MINIO_CONSOLE_PORT"
      ];
    };
    healthBody = managedServiceLifecycle.mkSimpleProbeBody {
      probeCommand = probeCommands.httpGetOkCmd {
        urlExpr = "http://127.0.0.1:$MINIO_API_PORT/minio/health/live";
      };
      successMessage = "minio healthy api_port=$MINIO_API_PORT";
      failureMessage = "minio unhealthy api_port=$MINIO_API_PORT";
    };
    readyBody = managedServiceLifecycle.mkSimpleProbeBody {
      probeCommand = probeCommands.httpGetOkCmd {
        urlExpr = "http://127.0.0.1:$MINIO_API_PORT/minio/health/ready";
      };
      successMessage = "minio ready api_port=$MINIO_API_PORT";
      failureMessage = "minio not ready api_port=$MINIO_API_PORT";
    };
  };

  inherit (managedLifecycle)
    init
    start
    stop
    restart
    status
    checkConfig
    health
    ready
    fullStart
    fullStartTest
    ;

  exportS3Env = pkgs.writeShellScript "minio-export-s3-env" ''
    ${loggingPrelude}

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
    exportS3Env
    ;
  inherit (managedLifecycle)
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
