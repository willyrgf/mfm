# Helios lifecycle management - init, start, stop, restart, status, health, check-config
{
  pkgs,
  project,
  slots,
  config,
  loggingPrelude,
}:

let
  lib = pkgs.lib;
  runtimeDefaults = import ../../../core/runtime-defaults.nix;
  managedServiceLifecycle = import ../../helpers/managed-service-lifecycle.nix { inherit pkgs; };
  probeCommands = import ../../helpers/probe-commands.nix { inherit pkgs; };
  probePlanRuntime = import ../../helpers/probe-plan-runtime.nix {
    inherit
      lib
      pkgs
      probeCommands
      ;
    postgresProbePkg = if pkgs ? postgresql_16 then pkgs.postgresql_16 else pkgs.postgresql;
  };
  slotEnvRuntime = import ../../helpers/slot-env-runtime.nix { inherit pkgs; };
  runtimeEvents = import ../../helpers/runtime-events.nix { inherit pkgs project; };
  observability = import ../../helpers/service-observability.nix {
    inherit
      pkgs
      slots
      runtimeEvents
      ;
  };
  helios = config.package;
  rpcPortVar = slots.portVarName config.portKeyRpc;
  executionRpcPortVar = slots.portVarName config.executionRpcPortKey;
  heliosDirExpr = slots.getServiceDir config.dataDirName;
  extraArgs = lib.escapeShellArgs (config.extraArgs or [ ]);
  serviceSource = if (config.defaultSource or "") == "" then "unspecified" else config.defaultSource;
  healthPlan = config.probePlans.health or { steps = [ ]; };
  readyPlan =
    config.probePlans.ready or {
      steps = [ ];
      wait = null;
    };
  renderPlanBody =
    mode: plan:
    probePlanRuntime.renderPlanBody {
      inherit
        mode
        plan
        ;
      serviceName = "helios";
      endpoints = config.resolvedEndpoints or { };
      portExprForEndpoint =
        endpointName:
        if endpointName == "rpc" then
          "$HELIOS_RPC_PORT"
        else if endpointName == "execution" then
          "$HELIOS_EXECUTION_PORT"
        else
          throw "helios lifecycle: unsupported probe endpoint '${endpointName}'";
    };
  healthPlanBody = renderPlanBody "health" healthPlan;
  readyPlanBody = renderPlanBody "ready" readyPlan;
  readyWait = readyPlan.wait or runtimeDefaults.probes.wait;

  runtimePrelude = ''
    ${slotEnvRuntime.loadJsonFromCommand {
      outVar = "SLOT_INFO_JSON_OUT";
      command = toString slots.getSlotInfo;
      exportVars = false;
    }}

    HELIOS_RPC_PORT_VAR="${rpcPortVar}"
    HELIOS_EXECUTION_PORT_VAR="${executionRpcPortVar}"

    ${slotEnvRuntime.readPortFromJson {
      targetVar = "HELIOS_RPC_PORT";
      jsonVar = "SLOT_INFO_JSON_OUT";
      keyExpr = "$HELIOS_RPC_PORT_VAR";
    }}
    ${slotEnvRuntime.readPortFromJson {
      targetVar = "HELIOS_EXECUTION_PORT";
      jsonVar = "SLOT_INFO_JSON_OUT";
      keyExpr = "$HELIOS_EXECUTION_PORT_VAR";
    }}
    HELIOS_DIR="${heliosDirExpr}"
    HELIOS_PID_FILE="$HELIOS_DIR/run/helios.pid"
    HELIOS_LOG_FILE="$HELIOS_DIR/logs/helios.log"
    SERVICE_DIR="$HELIOS_DIR"
    SERVICE_PID_FILE="$HELIOS_PID_FILE"
    SERVICE_LOG_FILE="$HELIOS_LOG_FILE"

    HELIOS_NETWORK="''${HELIOS_NETWORK:-${config.network or "local"}}"
    HELIOS_EXECUTION_RPC_URL="''${HELIOS_EXECUTION_RPC_URL:-${config.executionRpcUrl or ""}}"
    HELIOS_CONSENSUS_RPC_URL="''${HELIOS_CONSENSUS_RPC_URL:-${config.consensusRpcUrl or ""}}"
    HELIOS_DEFAULT_CONSENSUS_RPC_URL="''${HELIOS_DEFAULT_CONSENSUS_RPC_URL:-${
      config.defaultConsensusRpcUrl or ""
    }}"
    HELIOS_CHECKPOINT="''${HELIOS_CHECKPOINT:-${config.checkpoint or ""}}"

    if [ -z "$HELIOS_EXECUTION_RPC_URL" ] && [ -n "$HELIOS_EXECUTION_PORT" ]; then
      HELIOS_EXECUTION_RPC_URL="${probeCommands.localHttpUrlExpr "$HELIOS_EXECUTION_PORT"}"
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
      log_error "helios RPC port variable is not set"
      exit 1
    fi

    ${observability.mkEmitServiceEventFunction "helios"}
  '';

  heliosRpcUrlExpr = probeCommands.localHttpUrlExpr "$HELIOS_RPC_PORT";
  healthCheck = probeCommands.jsonRpcHasResultCmd {
    urlExpr = heliosRpcUrlExpr;
    method = "eth_chainId";
  };
  managedLifecycle = managedServiceLifecycle.mkPidFileManagedLifecycle {
    service = "helios";
    inherit
      loggingPrelude
      runtimePrelude
      ;
    initBody = ''
      mkdir -p "$SERVICE_DIR/data"
      mkdir -p "$SERVICE_DIR/config"
      mkdir -p "$SERVICE_DIR/run"
      mkdir -p "$SERVICE_DIR/logs"

      log_ok "helios initialized dir=$HELIOS_DIR slot=$SLOT env=$ENV"
    '';
    checkConfigBody = ''
      if [ ! -x "${helios}/bin/helios" ]; then
        log_error "missing helios binary at ${helios}/bin/helios"
        exit 1
      fi

      if [ "$HELIOS_NETWORK" != "local" ] && [ -z "$HELIOS_CONSENSUS_RPC_URL" ]; then
        log_error "HELIOS_CONSENSUS_RPC_URL is required when network is not local"
        exit 1
      fi

      if [ -z "$HELIOS_EXECUTION_RPC_URL" ]; then
        log_error "HELIOS_EXECUTION_RPC_URL is required (or set executionRpcPortKey to a valid port key)"
        exit 1
      fi

      # Ensure the expected Helios subcommand exists; CLI shape changes should fail fast here.
      ${helios}/bin/helios ethereum --help >/dev/null 2>&1
      log_ok "helios configuration valid dir=$HELIOS_DIR network=$HELIOS_NETWORK"
    '';
    startPreflight = ''
      if [ ! -x "${helios}/bin/helios" ]; then
        log_error "missing helios binary at ${helios}/bin/helios"
        exit 1
      fi

      if [ -z "$HELIOS_EXECUTION_RPC_URL" ]; then
        log_error "HELIOS_EXECUTION_RPC_URL is required (or set executionRpcPortKey to a valid port key)"
        exit 1
      fi

      # Derive a recent weak-subjectivity checkpoint when not pinned explicitly.
      #
      # This avoids a common failure mode where Helios stays "healthy" but remains unable to answer
      # `eth_blockNumber` because the consensus light client never bootstrapped.
      if [ "$HELIOS_NETWORK" != "local" ] && [ -z "$HELIOS_CHECKPOINT" ]; then
        if [ -z "$HELIOS_CONSENSUS_RPC_URL" ]; then
          log_error "cannot derive HELIOS_CHECKPOINT: HELIOS_CONSENSUS_RPC_URL is empty"
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

          log_info "deriving HELIOS_CHECKPOINT from consensus endpoint cons=$CONS"

          FINALIZED_URL="$CONS/eth/v1/beacon/headers/finalized"
          if ! FINALIZED_JSON="$(${pkgs.curl}/bin/curl -fsS --max-time 10 \
            --retry 3 --retry-delay 1 --retry-max-time 30 \
            -H 'accept: application/json' \
            "$FINALIZED_URL" 2>&1)"; then
            log_warn "failed to fetch finalized header cons=$CONS url=$FINALIZED_URL"
            echo "DETAIL: $FINALIZED_JSON" >&2
            continue
          fi

          slot="$(echo "$FINALIZED_JSON" | ${pkgs.jq}/bin/jq -r '.data.header.message.slot|tonumber' 2>/dev/null || true)"
          case "$slot" in
            *[!0-9]*|"")
              log_warn "failed to parse finalized slot from consensus response cons=$CONS"
              continue
              ;;
          esac

          epoch_start=$((slot - (slot % 32)))

          EPOCH_URL="$CONS/eth/v1/beacon/headers/$epoch_start"
          if ! EPOCH_JSON="$(${pkgs.curl}/bin/curl -fsS --max-time 10 \
            --retry 3 --retry-delay 1 --retry-max-time 30 \
            -H 'accept: application/json' \
            "$EPOCH_URL" 2>&1)"; then
            log_warn "failed to fetch epoch boundary header cons=$CONS url=$EPOCH_URL"
            echo "DETAIL: $EPOCH_JSON" >&2
            continue
          fi

          checkpoint="$(echo "$EPOCH_JSON" | ${pkgs.jq}/bin/jq -r '.data.root // empty' 2>/dev/null || true)"
          if ! echo "$checkpoint" | ${pkgs.gnugrep}/bin/grep -Eq '^0x[0-9a-fA-F]{64}$'; then
            log_warn "invalid checkpoint root from consensus endpoint cons=$CONS root='$checkpoint'"
            continue
          fi

          # Sanity-check that the light-client bootstrap endpoint is served for this checkpoint.
          BOOTSTRAP_URL="$CONS/eth/v1/beacon/light_client/bootstrap/$checkpoint"
          if ! BOOTSTRAP_ERR="$(${pkgs.curl}/bin/curl -fsS --max-time 10 \
            --retry 3 --retry-delay 1 --retry-max-time 30 \
            -H 'accept: application/json' \
            -o /dev/null \
            "$BOOTSTRAP_URL" 2>&1)"; then
            log_warn "consensus endpoint does not serve light_client/bootstrap cons=$CONS url=$BOOTSTRAP_URL"
            echo "DETAIL: $BOOTSTRAP_ERR" >&2
            continue
          fi

          HELIOS_CONSENSUS_RPC_URL="$CONS"
          HELIOS_CHECKPOINT="$checkpoint"
          log_info "derived HELIOS_CHECKPOINT=$HELIOS_CHECKPOINT (cons=$HELIOS_CONSENSUS_RPC_URL)"
          DERIVED=1
          break
        done

        if [ "$DERIVED" -ne 1 ]; then
          log_error "failed to derive HELIOS_CHECKPOINT from consensus endpoint(s)."
          log_hint "set HELIOS_CONSENSUS_RPC_URL and HELIOS_CHECKPOINT explicitly."
          exit 1
        fi
      fi

      if [ "$HELIOS_NETWORK" != "local" ] && [ -z "$HELIOS_CONSENSUS_RPC_URL" ]; then
        log_error "HELIOS_CONSENSUS_RPC_URL is required when network is not local"
        exit 1
      fi

      ARGS=(
        ethereum
        --network "$HELIOS_NETWORK"
        --rpc-bind-ip ${runtimeDefaults.hosts.loopbackIp}
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
    '';
    startCommand = ''
      "${helios}/bin/helios" "''${ARGS[@]}" > "$LOG_FILE" 2>&1 &
    '';
    startAlreadyRunningBody = managedServiceLifecycle.mkReadyOutcomeBody {
      level = "ok";
      pidExpr = ''"$PID"'';
      message = "helios already running pid=$PID rpc_port=$HELIOS_RPC_PORT";
    };
    startPostLaunchBody = managedServiceLifecycle.mkStartupReadinessBody {
      probeCommand = healthCheck;
      serviceLabel = "helios";
      probeAttempts = runtimeDefaults.probes.startupReadiness.extendedAttempts;
      degradedWaitReason = "failed_startup_health";
      degradedLastError = "helios failed initial health checks";
      failureMessage = "helios failed to become healthy";
      successMessage = "helios started pid=$CHILD_PID rpc_port=$HELIOS_RPC_PORT";
    };
    startExitFailureBody = managedServiceLifecycle.mkProcessExitFailureBody {
      waitReason = "helios_process_exit code=$RC";
      lastError = "helios process exited non-zero";
    };
    statusMergeBlock = observability.mkStatusMergeBlock {
      service = "helios";
      defaultLogPathExpr = ''"$HELIOS_LOG_FILE"'';
    };
    statusBody = observability.mkStatusLine {
      service = "helios";
      afterPidFields = [
        "rpc_port=$HELIOS_RPC_PORT"
        "network=$HELIOS_NETWORK"
      ];
    };
    healthBody = managedServiceLifecycle.mkPlanProbeBody {
      planBody = ''
        service_source=${lib.escapeShellArg serviceSource}
        ${healthPlanBody}
      '';
      skipMessage = "SKIP: helios health check has no probe steps";
    };
    readyBody =
      if !(readyWait.enabled or false) then
        managedServiceLifecycle.mkPlanProbeBody {
          planBody = ''
            service_source=${lib.escapeShellArg serviceSource}
            ${readyPlanBody}
          '';
          skipMessage = "SKIP: helios readiness check has no probe steps";
        }
      else
        ''
          TIMEOUT_SECS="''${HELIOS_READY_TIMEOUT_SECS:-${toString readyWait.timeoutSeconds}}"
          INTERVAL_SECS="''${HELIOS_READY_INTERVAL_SECS:-${toString readyWait.intervalSeconds}}"

          case "$TIMEOUT_SECS" in
            *[!0-9]*|"")
              log_error "HELIOS_READY_TIMEOUT_SECS must be an integer seconds value (got '$TIMEOUT_SECS')"
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
                log_error "helios not running (stale pid file) pid_file=$HELIOS_PID_FILE pid=''${PID:-unknown}"
              else
                log_error "helios not running (missing pid file) pid_file=$HELIOS_PID_FILE"
              fi
              exit 1
            fi

            ${pkgs.coreutils}/bin/sleep "$INTERVAL_SECS"
          done

          attempt=0

          while true; do
            attempt=$((attempt + 1))

            set +e
            (
              service_source=${lib.escapeShellArg serviceSource}
              ${readyPlanBody}
            ) >/dev/null 2>&1
            probe_rc=$?
            set -e

            if [ "$probe_rc" -eq 0 ]; then
              service_source=${lib.escapeShellArg serviceSource}
              ${readyPlanBody}
              exit 0
            fi

            if [ $((attempt % 10)) -eq 0 ]; then
              ${runtimeEvents.emitEvent} \
                --event-type readiness_progress \
                --service helios \
                --state waiting \
                --slot "$SLOT" \
                --env "$ENV" \
                --pid "$PID" \
                --log-path "$HELIOS_LOG_FILE" \
                --wait-reason "helios_ready_attempt=$attempt" >/dev/null 2>&1 || true
              log_info "helios not ready yet attempt=$attempt"
            fi

            now_ts="$(${pkgs.coreutils}/bin/date +%s)"
            if [ $((now_ts - start_ts)) -ge "$TIMEOUT_SECS" ]; then
              set +e
              (
                service_source=${lib.escapeShellArg serviceSource}
                ${readyPlanBody}
              )
              set -e
              log_error "helios not ready after $TIMEOUT_SECS s"
              log_hint "set HELIOS_CHECKPOINT and HELIOS_CONSENSUS_RPC_URL explicitly for mainnet."
              exit 1
            fi

            ${pkgs.coreutils}/bin/sleep "$INTERVAL_SECS"
          done
        '';
    stopWaitAttempts = runtimeDefaults.probes.managedStop.extendedWaitAttempts;
    stopWaitInterval = runtimeDefaults.probes.managedStop.extendedWaitIntervalSeconds;
  };
in
{
  inherit helios;
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
