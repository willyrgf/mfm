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
    SLOT_INFO_OUT="$(${slots.getSlotInfo})" || exit 1
    eval "$SLOT_INFO_OUT"
    SOCKET_HASH=$(printf '%s' "$RUN_DIR" | cksum | cut -d ' ' -f1)
    export PC_SOCKET_PATH="/tmp/nixfied-pc-$SOCKET_HASH.sock"
    CONFIG_FILE=$(${config.generateConfig})
    export PC_CONFIG_FILES="$CONFIG_FILE"
    export PC_DISABLE_TUI=1
    exec ${pc}/bin/process-compose -f "$CONFIG_FILE" -t=false --keep-project up
  '';

  stop = pkgs.writeShellScript "supervisor-stop" ''
    set -euo pipefail
    SLOT_INFO_OUT="$(${slots.getSlotInfo})" || exit 1
    eval "$SLOT_INFO_OUT"
    SOCKET_HASH=$(printf '%s' "$RUN_DIR" | cksum | cut -d ' ' -f1)
    export PC_SOCKET_PATH="/tmp/nixfied-pc-$SOCKET_HASH.sock"

    # First, stop via process-compose server (connected over socket).
    ${pc}/bin/process-compose down 2>/dev/null || true

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

    # Remove runtime state files
    rm -f "$PC_SOCKET_PATH" 2>/dev/null || true
    PID_FILE="$RUN_DIR/supervisor.pid"
    rm -f "$PID_FILE" 2>/dev/null || true

    echo "OK: Supervisor stopped"
  '';

  startDaemon = pkgs.writeShellScript "supervisor-start-daemon" ''
    set -euo pipefail
    SLOT_INFO_OUT="$(${slots.getSlotInfo})" || exit 1
    eval "$SLOT_INFO_OUT"
    SOCKET_HASH=$(printf '%s' "$RUN_DIR" | cksum | cut -d ' ' -f1)
    export PC_SOCKET_PATH="/tmp/nixfied-pc-$SOCKET_HASH.sock"
    CONFIG_FILE=$(${config.generateConfig})
    export PC_CONFIG_FILES="$CONFIG_FILE"
    export PC_DISABLE_TUI=1

    PID_FILE="$RUN_DIR/supervisor.pid"

    # Check if already running via socket.
    if ${pc}/bin/process-compose process list -o json >/dev/null 2>&1; then
      echo "OK: Supervisor already running socket=$PC_SOCKET_PATH"
      exit 0
    fi

    # Clear stale state from failed previous runs.
    rm -f "$PC_SOCKET_PATH" "$PID_FILE" 2>/dev/null || true

    echo "INFO: Starting supervisor in background"
    nohup ${pc}/bin/process-compose -f "$CONFIG_FILE" -t=false --keep-project up \
      > "$LOG_DIR/supervisor-daemon.log" 2>&1 &
    DAEMON_PID=$!
    echo "$DAEMON_PID" > "$PID_FILE"

    # Fail fast if the daemon exits immediately (common config/startup error case).
    sleep 1
    if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
      echo "ERROR: Supervisor failed to start (PID $DAEMON_PID exited). See: $LOG_DIR/supervisor-daemon.log" >&2
      rm -f "$PID_FILE" 2>/dev/null || true
      exit 1
    fi

    READY=0
    for _ in $(seq 1 40); do
      if ${pc}/bin/process-compose process list -o json >/dev/null 2>&1; then
        READY=1
        break
      fi
      sleep 0.25
    done

    if [ "$READY" -ne 1 ]; then
      echo "ERROR: Supervisor did not expose process API socket=$PC_SOCKET_PATH" >&2
      kill -TERM "$DAEMON_PID" 2>/dev/null || true
      rm -f "$PID_FILE" 2>/dev/null || true
      exit 1
    fi

    echo "OK: Supervisor started pid=$DAEMON_PID socket=$PC_SOCKET_PATH"
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
