# Managed service lifecycle helpers and wrapped service-runtime scripts.
# Pid-file managed services expose SERVICE_DIR, SERVICE_PID_FILE, and
# SERVICE_LOG_FILE from their runtime prelude.
{ pkgs }:

let
  lib = pkgs.lib;

  mkStopOutcomeBody =
    {
      serviceName,
      level,
      message,
      pidExpr ? null,
      logPathExpr ? ''"$SERVICE_LOG_FILE"'',
    }:
    ''
      emit_service_event service_stopped stopped${
        lib.optionalString (pidExpr != null) " --pid ${pidExpr}"
      }${lib.optionalString (logPathExpr != null) " --log-path ${logPathExpr}"}
      log_${level} "${serviceName} ${message}"
    '';

  mkProcessExitFailureBody =
    {
      waitReason,
      lastError,
    }:
    ''
      emit_service_event service_degraded degraded \
        --pid "$CHILD_PID" \
        --log-path "$LOG_FILE" \
        --wait-reason "${waitReason}" \
        --last-error "${lastError}"
    '';

  mkReadyOutcomeBody =
    {
      level,
      message,
      pidExpr,
      logPathExpr ? ''"$LOG_FILE"'',
    }:
    ''
      emit_service_event service_ready ready --pid ${pidExpr}${
        lib.optionalString (logPathExpr != null) " --log-path ${logPathExpr}"
      }
      log_${level} "${message}"
    '';

  mkSimpleProbeBody =
    {
      probeCommand,
      successMessage,
      failureMessage,
      successBody ? "",
      failureBody ? "",
    }:
    ''
      if ${probeCommand}; then
        ${successBody}
        log_ok "${successMessage}"
        exit 0
      fi

      ${failureBody}
      log_error "${failureMessage}"
      exit 1
    '';

  mkStartupReadinessBody =
    {
      probeCommand,
      serviceLabel,
      successMessage,
      failureMessage,
      degradedWaitReason,
      degradedLastError,
      probeAttempts ? 40,
      probeInterval ? "0.25",
      tailLines ? 50,
      successBody ? "",
    }:
    ''
      READY=0
      for _ in $(seq 1 ${toString probeAttempts}); do
        if ! kill -0 "$CHILD_PID" 2>/dev/null; then
          break
        fi
        if ${probeCommand}
        then
          READY=1
          break
        fi
        sleep ${probeInterval}
      done

      if [ "$READY" -ne 1 ]; then
        emit_service_event service_degraded degraded \
          --pid "$CHILD_PID" \
          --log-path "$LOG_FILE" \
          --wait-reason "${degradedWaitReason}" \
          --last-error "${degradedLastError}"
        log_error "${failureMessage}. log=$LOG_FILE"
        print_log_tail "$LOG_FILE" ${toString tailLines} "${serviceLabel}"
        exit 1
      fi

      emit_service_event service_ready ready --pid "$CHILD_PID" --log-path "$LOG_FILE"
      ${successBody}
      log_info "${successMessage}"
    '';

  mkWrappedScript =
    {
      name,
      loggingPrelude,
      runtimePrelude,
      body,
    }:
    pkgs.writeShellScript name ''
      ${loggingPrelude}

      set -euo pipefail
      ${runtimePrelude}

      ${body}
    '';

  mkObservedStatusScript =
    {
      name,
      loggingPrelude,
      runtimePrelude,
      runningStateBody,
      statusMergeBlock,
      statusBody,
    }:
    mkWrappedScript {
      inherit
        name
        loggingPrelude
        runtimePrelude
        ;
      body = ''
        RUNNING=false
        PID=""

        ${runningStateBody}

        ${statusMergeBlock}

        ${statusBody}

        if [ "$RUNNING" = "true" ]; then
          exit 0
        fi
        exit 1
      '';
    };

  mkPidFileManagedLifecycle =
    {
      service,
      loggingPrelude,
      runtimePrelude,
      initBody,
      checkConfigBody,
      startPreflight ? "",
      startCommand,
      startAlreadyRunningBody,
      startOnSpawnBody ? ''
        emit_service_event service_starting starting --pid "$CHILD_PID" --log-path "$LOG_FILE"
      '',
      startPostLaunchBody,
      startExitSuccessBody ? ''
        emit_service_event service_stopped stopped --pid "$CHILD_PID" --log-path "$LOG_FILE"
      '',
      startExitFailureBody,
      stopRequestBody ? ''
        kill "$PID" 2>/dev/null || true
      '',
      stopServiceName ? service,
      stopMissingLogPathExpr ? ''"$SERVICE_LOG_FILE"'',
      stopStateLogPathExpr ? stopMissingLogPathExpr,
      stopMissingBody ? mkStopOutcomeBody {
        serviceName = stopServiceName;
        level = "ok";
        message = "not running";
        logPathExpr = stopMissingLogPathExpr;
      },
      stopStaleBody ? mkStopOutcomeBody {
        serviceName = stopServiceName;
        level = "ok";
        message = "pid file cleaned";
        pidExpr = ''"$PID"'';
        logPathExpr = stopStateLogPathExpr;
      },
      stopStoppedBody ? mkStopOutcomeBody {
        serviceName = stopServiceName;
        level = "ok";
        message = "stopped pid=$PID";
        pidExpr = ''"$PID"'';
        logPathExpr = stopStateLogPathExpr;
      },
      stopForceKilledBody ? mkStopOutcomeBody {
        serviceName = stopServiceName;
        level = "warn";
        message = "force-killed pid=$PID";
        pidExpr = ''"$PID"'';
        logPathExpr = stopStateLogPathExpr;
      },
      statusMergeBlock,
      statusBody,
      healthBody,
      readyBody,
      stopWaitAttempts ? 20,
      stopWaitInterval ? "0.2",
      fullStartBody ? null,
      fullStartTestBody ? null,
    }:
    let
      init = mkWrappedScript {
        name = "${service}-init";
        inherit
          loggingPrelude
          runtimePrelude
          ;
        body = initBody;
      };

      start = mkWrappedScript {
        name = "${service}-start";
        inherit
          loggingPrelude
          runtimePrelude
          ;
        body = ''
          LOG_FILE="$SERVICE_LOG_FILE"

          ${init}

          if [ -f "$SERVICE_PID_FILE" ]; then
            PID=$(cat "$SERVICE_PID_FILE" 2>/dev/null || true)
            if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
              ${startAlreadyRunningBody}
              exit 0
            fi
            rm -f "$SERVICE_PID_FILE"
          fi

          ${startPreflight}
          ${startCommand}
          CHILD_PID=$!
          echo "$CHILD_PID" > "$SERVICE_PID_FILE"

          ${startOnSpawnBody}

          cleanup() {
            if [ -n "''${CHILD_PID:-}" ] && kill -0 "$CHILD_PID" 2>/dev/null; then
              kill "$CHILD_PID" 2>/dev/null || true
              wait "$CHILD_PID" 2>/dev/null || true
            fi
            rm -f "$SERVICE_PID_FILE"
          }

          trap cleanup EXIT INT TERM

          ${startPostLaunchBody}
          set +e
          wait "$CHILD_PID"
          RC=$?
          set -e

          if [ "$RC" -eq 0 ]; then
            ${startExitSuccessBody}
          else
            ${startExitFailureBody}
          fi
          exit "$RC"
        '';
      };

      stop = mkWrappedScript {
        name = "${service}-stop";
        inherit
          loggingPrelude
          runtimePrelude
          ;
        body = ''
          if [ ! -f "$SERVICE_PID_FILE" ]; then
            ${stopMissingBody}
            exit 0
          fi

          PID=$(cat "$SERVICE_PID_FILE" 2>/dev/null || true)
          if [ -z "$PID" ] || ! kill -0 "$PID" 2>/dev/null; then
            rm -f "$SERVICE_PID_FILE"
            ${stopStaleBody}
            exit 0
          fi

          ${stopRequestBody}

          for _ in $(seq 1 ${toString stopWaitAttempts}); do
            if ! kill -0 "$PID" 2>/dev/null; then
              rm -f "$SERVICE_PID_FILE"
              ${stopStoppedBody}
              exit 0
            fi
            sleep ${stopWaitInterval}
          done

          kill -KILL "$PID" 2>/dev/null || true
          rm -f "$SERVICE_PID_FILE"
          ${stopForceKilledBody}
        '';
      };

      restart = pkgs.writeShellScript "${service}-restart" ''
        ${loggingPrelude}

        set -euo pipefail

        ${stop}
        exec ${start}
      '';

      status = mkObservedStatusScript {
        name = "${service}-status";
        inherit
          loggingPrelude
          runtimePrelude
          ;
        runningStateBody = ''
          if [ -f "$SERVICE_PID_FILE" ]; then
            PID=$(cat "$SERVICE_PID_FILE" 2>/dev/null || true)
            if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
              RUNNING=true
            fi
          fi
        '';
        inherit
          statusMergeBlock
          statusBody
          ;
      };

      checkConfig = mkWrappedScript {
        name = "${service}-check-config";
        inherit
          loggingPrelude
          runtimePrelude
          ;
        body = checkConfigBody;
      };

      health = mkWrappedScript {
        name = "${service}-health";
        inherit
          loggingPrelude
          runtimePrelude
          ;
        body = healthBody;
      };

      ready = mkWrappedScript {
        name = "${service}-ready";
        inherit
          loggingPrelude
          runtimePrelude
          ;
        body = readyBody;
      };

      fullStart = pkgs.writeShellScript "${service}-full-start" ''
        ${loggingPrelude}

        set -euo pipefail

        ${
          if fullStartBody != null then
            fullStartBody
          else
            ''
              ${init}
              ${checkConfig}
              exec ${start}
            ''
        }
      '';

      fullStartTest = pkgs.writeShellScript "${service}-full-start-test" ''
        ${loggingPrelude}

        set -euo pipefail

        ${
          if fullStartTestBody != null then
            fullStartTestBody
          else if fullStartBody != null then
            fullStartBody
          else
            ''
              ${init}
              ${checkConfig}
              exec ${start}
            ''
        }
      '';
    in
    {
      inherit
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
    };
in
{
  inherit
    mkProcessExitFailureBody
    mkReadyOutcomeBody
    mkSimpleProbeBody
    mkStartupReadinessBody
    mkWrappedScript
    mkObservedStatusScript
    mkPidFileManagedLifecycle
    ;
}
