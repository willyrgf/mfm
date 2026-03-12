# Supervisor management - restart, log rotation
{
  pkgs,
  slots,
  config,
  runtime,
  loggingPrelude,
}:

let
  pc = pkgs.process-compose;

  restart = runtime.mkSupervisorScript {
    name = "supervisor-restart";
    body = ''
      SERVICE="''${1:-}"
      if [ -z "$SERVICE" ]; then
        echo "Usage: supervisor-restart <service-name>" >&2
        exit 1
      fi

      log_info "Restarting $SERVICE"
      ${pc}/bin/process-compose process restart "$SERVICE"
      log_ok "$SERVICE restarted"
    '';
  };

  rotateLogs = runtime.mkSupervisorScript {
    name = "supervisor-rotate-logs";
    includeLogDir = true;
    body = ''
      MAX_SIZE="''${1:-10485760}"  # 10MB default
      KEEP_COUNT="''${2:-5}"

      for logfile in "$LOG_DIR"/*.log; do
        [ -f "$logfile" ] || continue

        SIZE=$(stat -f%z "$logfile" 2>/dev/null || stat -c%s "$logfile" 2>/dev/null || echo 0)
        if [ "$SIZE" -gt "$MAX_SIZE" ]; then
          log_info "Rotating $(basename "$logfile") ($SIZE bytes)"

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
          log_ok "Rotated $(basename "$logfile")"
        fi
      done
    '';
  };

in
{
  inherit restart rotateLogs;
}
