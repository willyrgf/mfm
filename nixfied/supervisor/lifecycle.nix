# Supervisor lifecycle - start, stop, startDaemon with orphan cleanup
{
  pkgs,
  project,
  slots,
  config,
}:

let
  pc = pkgs.process-compose;
  ports = project.ports or { };
  portNames = builtins.attrNames ports;

  start = pkgs.writeShellScript "supervisor-start" ''
    set -euo pipefail
    CONFIG_FILE=$(${config.generateConfig})
    exec ${pc}/bin/process-compose -f "$CONFIG_FILE" up
  '';

  stop = pkgs.writeShellScript "supervisor-stop" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"
    CONFIG_FILE=$(${config.generateConfig})

    # First, stop via process-compose
    ${pc}/bin/process-compose -f "$CONFIG_FILE" down 2>/dev/null || true

    # Clean up orphan processes on configured ports
    ${pkgs.lib.concatMapStringsSep "\n" (
      name:
      let
        portVar = slots.portVarName name;
      in
      ''
        PORT="''${${portVar}:-}"
        if [ -n "$PORT" ] && command -v lsof >/dev/null 2>&1; then
          ORPHANS=$(lsof -ti:"$PORT" 2>/dev/null || true)
          if [ -n "$ORPHANS" ]; then
            echo "INFO: Cleaning orphan processes on port $PORT (${name}): $ORPHANS"
            echo "$ORPHANS" | xargs kill -TERM 2>/dev/null || true
          fi
        fi
      ''
    ) portNames}

    # Remove PID file
    PID_FILE="$RUN_DIR/supervisor.pid"
    rm -f "$PID_FILE" 2>/dev/null || true

    echo "OK: Supervisor stopped"
  '';

  startDaemon = pkgs.writeShellScript "supervisor-start-daemon" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"
    CONFIG_FILE=$(${config.generateConfig})

    PID_FILE="$RUN_DIR/supervisor.pid"

    # Check if already running
    if [ -f "$PID_FILE" ]; then
      PID=$(cat "$PID_FILE" 2>/dev/null || true)
      if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        echo "OK: Supervisor already running (PID $PID)"
        exit 0
      fi
      rm -f "$PID_FILE"
    fi

    echo "INFO: Starting supervisor in background"
    nohup ${pc}/bin/process-compose -f "$CONFIG_FILE" up \
      > "$LOG_DIR/supervisor-daemon.log" 2>&1 &

    DAEMON_PID=$!
    echo "$DAEMON_PID" > "$PID_FILE"

    # Fail fast if the daemon exits immediately (common config error case).
    sleep 1
    if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
      echo "ERROR: Supervisor failed to start (PID $DAEMON_PID exited). See: $LOG_DIR/supervisor-daemon.log" >&2
      rm -f "$PID_FILE" 2>/dev/null || true
      exit 1
    fi

    echo "OK: Supervisor started (PID $DAEMON_PID)"
  '';

in
{
  inherit
    pc
    start
    stop
    startDaemon
    ;
}
