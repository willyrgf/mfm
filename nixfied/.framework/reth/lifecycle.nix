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
  managedServiceLifecycle = import ../lib/managed-service-lifecycle.nix { inherit pkgs; };
  probeCommands = import ../lib/probe-commands.nix { inherit pkgs; };
  slotEnvRuntime = import ../lib/slot-env-runtime.nix { inherit pkgs; };
  runtimeEvents = import ../lib/runtime-events.nix { inherit pkgs project; };
  observability = import ../lib/service-observability.nix {
    inherit
      pkgs
      slots
      runtimeEvents
      ;
  };
  reth = config.package or pkgs.reth;
  httpPortVar = slots.portVarName config.portKeyHttp;
  wsPortVar = slots.portVarName config.portKeyWs;
  authPortVar = slots.portVarName config.portKeyAuth;
  rethDirExpr = slots.getServiceDir config.dataDirName;
  useDevMode = config.devMode or false;
  extraArgs = lib.escapeShellArgs (config.extraArgs or [ ]);
  emitHelper = observability.mkEmitServiceEventFunction "reth";

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
    SERVICE_DIR="$RETH_DIR"
    SERVICE_PID_FILE="$RETH_PID_FILE"
    SERVICE_LOG_FILE="$RETH_LOG_FILE"
    RETH_NETWORK="''${RETH_NETWORK:-${config.network or "local"}}"
    RETH_USE_DEV="${if useDevMode then "1" else "0"}"

    if [ "$RETH_NETWORK" = "local" ]; then
      RETH_USE_DEV="1"
    fi

    if [ -z "$RETH_HTTP_PORT" ] || [ -z "$RETH_WS_PORT" ] || [ -z "$RETH_AUTH_PORT" ]; then
      log_error "reth port variables are not set (http/ws/auth)"
      exit 1
    fi

    ${emitHelper}
  '';

  healthCheck = probeCommands.jsonRpcHasResultCmd {
    urlExpr = "http://127.0.0.1:$RETH_HTTP_PORT";
    method = "web3_clientVersion";
  };
  managedLifecycle = managedServiceLifecycle.mkPidFileManagedLifecycle {
    service = "reth";
    inherit
      loggingPrelude
      runtimePrelude
      ;
    initBody = ''
      mkdir -p "$SERVICE_DIR/data"
      mkdir -p "$SERVICE_DIR/config"
      mkdir -p "$SERVICE_DIR/run"
      mkdir -p "$SERVICE_DIR/logs"

      if [ ! -f "$RETH_JWT_FILE" ]; then
        printf '%064x\n' 0 > "$RETH_JWT_FILE"
      fi
      chmod 600 "$RETH_JWT_FILE" 2>/dev/null || true

      log_ok "reth initialized dir=$RETH_DIR slot=$SLOT env=$ENV"
    '';
    checkConfigBody = ''
      if [ ! -x "${reth}/bin/reth" ]; then
        log_error "missing reth binary at ${reth}/bin/reth"
        exit 1
      fi

      mkdir -p "$RETH_DIR/config"
      ${reth}/bin/reth --version >/dev/null 2>&1
      log_ok "reth configuration valid dir=$RETH_DIR network=$RETH_NETWORK"
    '';
    startPreflight = ''
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
    '';
    startCommand = ''
      "${reth}/bin/reth" "''${ARGS[@]}" > "$LOG_FILE" 2>&1 &
    '';
    startAlreadyRunningBody = managedServiceLifecycle.mkReadyOutcomeBody {
      level = "ok";
      pidExpr = ''"$PID"'';
      message = "reth already running pid=$PID http_port=$RETH_HTTP_PORT";
    };
    startPostLaunchBody = managedServiceLifecycle.mkStartupReadinessBody {
      probeCommand = healthCheck;
      serviceLabel = "reth";
      probeAttempts = 80;
      degradedWaitReason = "failed_readiness";
      degradedLastError = "reth failed health check during startup";
      failureMessage = "reth failed to become healthy";
      successMessage = "reth started pid=$CHILD_PID http_port=$RETH_HTTP_PORT ws_port=$RETH_WS_PORT auth_port=$RETH_AUTH_PORT";
    };
    startExitFailureBody = managedServiceLifecycle.mkProcessExitFailureBody {
      waitReason = "reth_process_exit code=$RC";
      lastError = "reth process exited non-zero";
    };
    statusMergeBlock = observability.mkStatusMergeBlock {
      service = "reth";
      defaultLogPathExpr = ''"$RETH_LOG_FILE"'';
    };
    statusBody = observability.mkStatusLine {
      service = "reth";
      afterPidFields = [
        "http_port=$RETH_HTTP_PORT"
        "ws_port=$RETH_WS_PORT"
        "auth_port=$RETH_AUTH_PORT"
        "network=$RETH_NETWORK"
      ];
    };
    healthBody = managedServiceLifecycle.mkSimpleProbeBody {
      probeCommand = healthCheck;
      successMessage = "reth healthy http_port=$RETH_HTTP_PORT";
      failureMessage = "reth unhealthy http_port=$RETH_HTTP_PORT";
    };
    readyBody = managedServiceLifecycle.mkSimpleProbeBody {
      probeCommand = healthCheck;
      successMessage = "reth ready http_port=$RETH_HTTP_PORT";
      failureMessage = "reth not ready http_port=$RETH_HTTP_PORT";
    };
    stopWaitAttempts = 40;
    stopWaitInterval = "0.25";
  };
in
{
  inherit reth;
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
