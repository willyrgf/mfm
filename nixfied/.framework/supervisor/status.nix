# Supervisor status and log viewing
{
  pkgs,
  slots,
  config,
  loggingPrelude,
}:

let
  pc = pkgs.process-compose;
  jq = pkgs.jq;

  status = pkgs.writeShellScript "supervisor-status" ''
    ${loggingPrelude}

    set -euo pipefail
    source <(${slots.getSlotInfo})
    SOCKET_HASH=$(printf '%s' "$RUN_DIR" | cksum | cut -d ' ' -f1)
    export PC_SOCKET_PATH="/tmp/nixfied-pc-$SOCKET_HASH.sock"

    if ! ${isRunning} >/dev/null 2>&1; then
      echo "service=supervisor slot=$SLOT env=$ENV running=false"
      exit 1
    fi

    exec ${pc}/bin/process-compose process list -o wide
  '';

  isRunning = pkgs.writeShellScript "supervisor-is-running" ''
    set -euo pipefail
    source <(${slots.getSlotInfo})

    SOCKET_HASH=$(printf '%s' "$RUN_DIR" | cksum | cut -d ' ' -f1)
    export PC_SOCKET_PATH="/tmp/nixfied-pc-$SOCKET_HASH.sock"

    if ${pc}/bin/process-compose process list -o json >/dev/null 2>&1; then
      echo "running (socket $PC_SOCKET_PATH)"
      exit 0
    fi

    echo "stopped"
    exit 1
  '';

  logs = pkgs.writeShellScript "supervisor-logs" ''
    set -euo pipefail
    source <(${slots.getSlotInfo})

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

  health = pkgs.writeShellScript "supervisor-health" ''
    ${loggingPrelude}

    set -euo pipefail
    source <(${slots.getSlotInfo})

    SOCKET_HASH=$(printf '%s' "$RUN_DIR" | cksum | cut -d ' ' -f1)
    export PC_SOCKET_PATH="/tmp/nixfied-pc-$SOCKET_HASH.sock"

    if ! ${isRunning} >/dev/null 2>&1; then
      log_error "supervisor unhealthy reason=daemon_not_running slot=$SLOT env=$ENV"
      exit 1
    fi

    set +e
    PROC_JSON=$(${pc}/bin/process-compose process list -o json 2>/dev/null)
    RC=$?
    set -e
    if [ "$RC" -ne 0 ] || [ -z "$PROC_JSON" ]; then
      log_error "supervisor unhealthy reason=process_query_failed socket=$PC_SOCKET_PATH"
      exit 1
    fi

    TOTAL=$(${jq}/bin/jq -r 'length' <<<"$PROC_JSON")
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
      ' <<<"$PROC_JSON"
    )

    if [ -n "$UNHEALTHY" ]; then
      log_error "supervisor unhealthy slot=$SLOT env=$ENV services=$UNHEALTHY"
      exit 1
    fi

    log_ok "supervisor healthy services=$TOTAL slot=$SLOT env=$ENV"
  '';

in
{
  inherit
    status
    isRunning
    health
    logs
    ;
}
