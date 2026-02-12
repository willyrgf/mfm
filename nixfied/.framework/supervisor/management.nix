# Supervisor management - restart, log rotation
{
  pkgs,
  slots,
  config,
}:

let
  pc = pkgs.process-compose;

  restart = pkgs.writeShellScript "supervisor-restart" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    SERVICE="''${1:-}"
    if [ -z "$SERVICE" ]; then
      echo "Usage: supervisor-restart <service-name>" >&2
      exit 1
    fi

    SOCKET_HASH=$(printf '%s' "$RUN_DIR" | cksum | cut -d ' ' -f1)
    export PC_SOCKET_PATH="/tmp/nixfied-pc-$SOCKET_HASH.sock"

    echo "INFO: Restarting $SERVICE"
    ${pc}/bin/process-compose process restart "$SERVICE"
    echo "OK: $SERVICE restarted"
  '';

  rotateLogs = pkgs.writeShellScript "supervisor-rotate-logs" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    MAX_SIZE="''${1:-10485760}"  # 10MB default
    KEEP_COUNT="''${2:-5}"

    for logfile in "$LOG_DIR"/*.log; do
      [ -f "$logfile" ] || continue

      SIZE=$(stat -f%z "$logfile" 2>/dev/null || stat -c%s "$logfile" 2>/dev/null || echo 0)
      if [ "$SIZE" -gt "$MAX_SIZE" ]; then
        echo "INFO: Rotating $(basename "$logfile") ($SIZE bytes)"

        # Shift existing rotated logs
        for i in $(seq "$KEEP_COUNT" -1 1); do
          PREV=$((i - 1))
          if [ "$PREV" -eq 0 ]; then
            SRC="$logfile"
          else
            SRC="$logfile.$PREV.gz"
          fi
          DST="$logfile.$i.gz"
          if [ -f "$SRC" ] && [ "$PREV" -ne 0 ]; then
            mv "$SRC" "$DST"
          fi
        done

        # Compress current log and start fresh
        gzip -c "$logfile" > "$logfile.1.gz"
        : > "$logfile"
        echo "OK: Rotated $(basename "$logfile")"
      fi
    done
  '';

in
{
  inherit restart rotateLogs;
}
