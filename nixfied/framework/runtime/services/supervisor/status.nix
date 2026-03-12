# Supervisor status and log viewing
{
  pkgs,
  slots,
  config,
  runtime,
  loggingPrelude,
}:

let
  pc = pkgs.process-compose;
  jq = pkgs.jq;

  status = runtime.mkSupervisorScript {
    name = "supervisor-status";
    body = ''
      if ! ${isRunning} >/dev/null 2>&1; then
        echo "service=supervisor slot=$SLOT env=$ENV running=false"
        exit 1
      fi

      exec ${pc}/bin/process-compose process list -o wide
    '';
  };

  isRunning = runtime.mkSupervisorScript {
    name = "supervisor-is-running";
    useLoggingPrelude = false;
    body = ''
      if supervisor_process_api_ready; then
        echo "running (socket $PC_SOCKET_PATH)"
        exit 0
      fi

      echo "stopped"
      exit 1
    '';
  };

  logs = runtime.mkSupervisorScript {
    name = "supervisor-logs";
    includeLogDir = true;
    useLoggingPrelude = false;
    body = ''
      SERVICE="''${1:-}"
      LINES="''${2:-50}"

      if [ -n "$SERVICE" ]; then
        LOG_FILE="$LOG_DIR/$SERVICE.log"
        if [ -f "$LOG_FILE" ]; then
          tail -n "$LINES" -f "$LOG_FILE"
        else
          echo "No log file for service: $SERVICE" >&2
          echo "Looking in $LOG_DIR..." >&2
          ls "$LOG_DIR"/*.log 2>/dev/null || echo "No logs found"
          exit 1
        fi
      else
        LOG_FILE="$LOG_DIR/supervisor.log"
        if [ -f "$LOG_FILE" ]; then
          tail -n "$LINES" -f "$LOG_FILE"
        else
          echo "No supervisor log found at $LOG_FILE" >&2
          exit 1
        fi
      fi
    '';
  };

  health = runtime.mkSupervisorScript {
    name = "supervisor-health";
    body = ''
      if ! supervisor_process_api_ready; then
        log_error "supervisor unhealthy reason=daemon_not_running slot=$SLOT env=$ENV"
        exit 1
      fi

      if ! supervisor_fetch_process_json; then
        log_error "supervisor unhealthy reason=process_query_failed socket=$PC_SOCKET_PATH"
        exit 1
      fi

      TOTAL=$(${jq}/bin/jq -r 'length' <<<"$SUPERVISOR_PROCESS_JSON")
      if [ "$TOTAL" -eq 0 ]; then
        log_ok "supervisor healthy services=0 slot=$SLOT env=$ENV"
        exit 0
      fi

      UNHEALTHY=$(
        ${jq}/bin/jq -r '
          [
            .[]
            | select((.status != "Running") or (.is_running != true))
            | (
                .name
                + ":status=" + (.status | tostring)
                + ",running=" + (.is_running | tostring)
                + ",ready=" + (.is_ready | tostring)
              )
          ] | join("; ")
        ' <<<"$SUPERVISOR_PROCESS_JSON"
      )

      if [ -n "$UNHEALTHY" ]; then
        log_error "supervisor unhealthy slot=$SLOT env=$ENV services=$UNHEALTHY"
        exit 1
      fi

      log_ok "supervisor healthy services=$TOTAL slot=$SLOT env=$ENV"
    '';
  };

in
{
  inherit
    status
    isRunning
    health
    logs
    ;
}
