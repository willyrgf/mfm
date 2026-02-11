# Supervisor status and log viewing
{
  pkgs,
  slots,
  config,
}:

let
  pc = pkgs.process-compose;

  status = pkgs.writeShellScript "supervisor-status" ''
    set -euo pipefail
    CONFIG_FILE=$(${config.generateConfig})
    exec ${pc}/bin/process-compose -f "$CONFIG_FILE" status
  '';

  isRunning = pkgs.writeShellScript "supervisor-is-running" ''
    eval "$(${slots.getSlotInfo})"
    PID_FILE="$RUN_DIR/supervisor.pid"
    if [ -f "$PID_FILE" ]; then
      PID=$(cat "$PID_FILE" 2>/dev/null || true)
      if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        echo "running (PID $PID)"
        exit 0
      fi
    fi
    echo "stopped"
    exit 1
  '';

  logs = pkgs.writeShellScript "supervisor-logs" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

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

in
{
  inherit status isRunning logs;
}
