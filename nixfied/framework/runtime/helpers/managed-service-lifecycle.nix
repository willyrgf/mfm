# Managed service lifecycle helpers and wrapped service-runtime scripts.
# Pid-file managed services expose SERVICE_DIR, SERVICE_PID_FILE, and
# SERVICE_LOG_FILE from their runtime prelude.
{ pkgs }:

let
  lib = pkgs.lib;
  runtimeDefaults = import ../../core/runtime-defaults.nix;

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
      if ${probeCommand}
      then
        ${successBody}
        log_ok "${successMessage}"
        exit 0
      fi

      ${failureBody}
      log_error "${failureMessage}"
      exit 1
    '';

  mkPlanProbeBody =
    {
      planBody,
      skipMessage,
      successBody ? "",
      wait ? null,
      timeoutMessage ? "probe timed out",
      progressBody ? "",
      preflightBody ? "",
    }:
    if planBody == "" then
      ''
        echo "${skipMessage}"
        exit 0
      ''
    else if wait == null || !(wait.enabled or false) then
      ''
        ${planBody}
        ${successBody}
        exit 0
      ''
    else
      ''
        ${preflightBody}
        run_probe_once() {
          ${planBody}
        }

        plan_probe_timeout_secs=${
          toString (wait.timeoutSeconds or runtimeDefaults.probes.wait.timeoutSeconds)
        }
        plan_probe_interval_secs=${
          toString (wait.intervalSeconds or runtimeDefaults.probes.wait.intervalSeconds)
        }
        ${lib.optionalString ((wait.timeoutEnvVar or null) != null) ''
          plan_probe_timeout_var=${lib.escapeShellArg wait.timeoutEnvVar}
          plan_probe_timeout_override="$(${pkgs.coreutils}/bin/printenv "$plan_probe_timeout_var" 2>/dev/null || true)"
          if [ -n "$plan_probe_timeout_override" ]; then
            plan_probe_timeout_secs="$plan_probe_timeout_override"
          fi
        ''}
        ${lib.optionalString ((wait.intervalEnvVar or null) != null) ''
          plan_probe_interval_var=${lib.escapeShellArg wait.intervalEnvVar}
          plan_probe_interval_override="$(${pkgs.coreutils}/bin/printenv "$plan_probe_interval_var" 2>/dev/null || true)"
          if [ -n "$plan_probe_interval_override" ]; then
            plan_probe_interval_secs="$plan_probe_interval_override"
          fi
        ''}

        case "$plan_probe_timeout_secs" in
          *[!0-9]*|"")
            log_error "probe timeout must be an integer seconds value (got '$plan_probe_timeout_secs')"
            exit 1
            ;;
        esac

        case "$plan_probe_interval_secs" in
          *[!0-9]*|"")
            log_error "probe interval must be an integer seconds value (got '$plan_probe_interval_secs')"
            exit 1
            ;;
        esac

        start_ts="$(${pkgs.coreutils}/bin/date +%s)"
        attempt=0

        while true; do
          attempt=$((attempt + 1))

          set +e
          ( run_probe_once ) >/dev/null 2>&1
          probe_rc=$?
          set -e

          if [ "$probe_rc" -eq 0 ]; then
            ( run_probe_once )
            ${successBody}
            exit 0
          fi

          ${progressBody}

          now_ts="$(${pkgs.coreutils}/bin/date +%s)"
          if [ $((now_ts - start_ts)) -ge "$plan_probe_timeout_secs" ]; then
            set +e
            ( run_probe_once )
            set -e
            log_error "${timeoutMessage}"
            exit 1
          fi

          ${pkgs.coreutils}/bin/sleep "$plan_probe_interval_secs"
        done
      '';

  mkStartupReadinessBody =
    {
      probeCommand,
      serviceLabel,
      successMessage,
      failureMessage,
      degradedWaitReason,
      degradedLastError,
      probeAttempts ? runtimeDefaults.probes.startupReadiness.attempts,
      probeInterval ? runtimeDefaults.probes.startupReadiness.intervalSeconds,
      tailLines ? runtimeDefaults.probes.startupReadiness.tailLines,
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
      startPreflightBody ? "",
      startPrepareBody ? "",
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
      stopWaitAttempts ? runtimeDefaults.probes.managedStop.waitAttempts,
      stopWaitInterval ? runtimeDefaults.probes.managedStop.waitIntervalSeconds,
      fullStartLeafBody ? null,
      fullStartTestLeafBody ? null,
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

      preflightStart = mkWrappedScript {
        name = "${service}-preflight-start";
        inherit
          loggingPrelude
          runtimePrelude
          ;
        body = if startPreflightBody == "" then ":" else startPreflightBody;
      };

      startLeaf = mkWrappedScript {
        name = "${service}-start-leaf";
        inherit
          loggingPrelude
          runtimePrelude
          ;
        body = ''
          LOG_FILE="$SERVICE_LOG_FILE"

          if [ -f "$SERVICE_PID_FILE" ]; then
            PID=$(cat "$SERVICE_PID_FILE" 2>/dev/null || true)
            if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
              ${startAlreadyRunningBody}
              exit 0
            fi
            rm -f "$SERVICE_PID_FILE"
          fi

          ${startPrepareBody}
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
          # Let composed full-start helpers reuse startup readiness without
          # keeping this supervisor wrapper alive.
          if [ "''${NIXFIED_START_RETURN_AFTER_READY:-0}" = "1" ]; then
            trap - EXIT INT TERM
            if command -v disown >/dev/null 2>&1; then
              disown "$CHILD_PID" 2>/dev/null || true
            fi
            exit 0
          fi

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

      start = pkgs.writeShellScript "${service}-start" ''
        ${loggingPrelude}

        set -euo pipefail

        ${init}
        ${checkConfig}
        ${preflightStart}
        exec ${startLeaf}
      '';

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

      defaultFullStartBody = ''
        NIXFIED_START_RETURN_AFTER_READY=1 exec ${startLeaf}
      '';

      fullStartLeaf = pkgs.writeShellScript "${service}-full-start-leaf" ''
        ${loggingPrelude}

        set -euo pipefail

        ${if fullStartLeafBody != null then fullStartLeafBody else defaultFullStartBody}
      '';

      fullStartTestLeaf = pkgs.writeShellScript "${service}-full-start-test-leaf" ''
        ${loggingPrelude}

        set -euo pipefail

        ${
          if fullStartTestLeafBody != null then
            fullStartTestLeafBody
          else if fullStartLeafBody != null then
            fullStartLeafBody
          else
            defaultFullStartBody
        }
      '';

      fullStart = pkgs.writeShellScript "${service}-full-start" ''
        ${loggingPrelude}

        set -euo pipefail

        ${init}
        ${checkConfig}
        ${preflightStart}
        exec ${fullStartLeaf}
      '';

      fullStartTest = pkgs.writeShellScript "${service}-full-start-test" ''
        ${loggingPrelude}

        set -euo pipefail

        ${init}
        ${checkConfig}
        ${preflightStart}
        exec ${fullStartTestLeaf}
      '';
    in
    {
      inherit
        init
        preflightStart
        startLeaf
        start
        stop
        restart
        status
        checkConfig
        health
        ready
        fullStartLeaf
        fullStartTestLeaf
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
    mkPlanProbeBody
    mkStartupReadinessBody
    mkWrappedScript
    mkObservedStatusScript
    mkPidFileManagedLifecycle
    ;
}
