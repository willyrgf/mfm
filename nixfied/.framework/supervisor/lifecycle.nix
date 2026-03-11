# Supervisor lifecycle - start, stop, startDaemon with orphan cleanup
{
  pkgs,
  project,
  slots,
  config,
  runtime,
  loggingPrelude,
}:

let
  start = runtime.mkSupervisorScript {
    name = "supervisor-start";
    includeConfig = true;
    body = ''
      supervisor_exec_up
    '';
  };

  stop = runtime.mkSupervisorScript {
    name = "supervisor-stop";
    includePorts = true;
    body = ''
      # First, stop via process-compose server (connected over socket).
      supervisor_stop_server

      # Clean up orphan processes on configured ports
      supervisor_cleanup_orphans

      # Remove runtime state files
      supervisor_clear_state

      log_ok "Supervisor stopped"
    '';
  };

  startDaemon = runtime.mkSupervisorScript {
    name = "supervisor-start-daemon";
    includeLogDir = true;
    includeConfig = true;
    body = ''
      PID_FILE="$(supervisor_pid_file)"

      # Check if already running via socket.
      if supervisor_process_api_ready; then
        log_ok "Supervisor already running socket=$PC_SOCKET_PATH"
        exit 0
      fi

      # Clear stale state from failed previous runs.
      supervisor_clear_state

      log_info "Starting supervisor in background"
      DAEMON_LOG_FILE="$LOG_DIR/supervisor-daemon.log"
      supervisor_spawn_daemon "$DAEMON_LOG_FILE"
      echo "$DAEMON_PID" > "$PID_FILE"

      # Fail fast if the daemon exits immediately (common config/startup error case).
      sleep 1
      if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
        log_error "Supervisor failed to start (PID $DAEMON_PID exited). See: $DAEMON_LOG_FILE"
        rm -f "$PID_FILE" 2>/dev/null || true
        exit 1
      fi

      if ! supervisor_wait_process_api_ready 40 0.25; then
        log_error "Supervisor did not expose process API socket=$PC_SOCKET_PATH"
        kill -TERM "$DAEMON_PID" 2>/dev/null || true
        rm -f "$PID_FILE" 2>/dev/null || true
        exit 1
      fi

      log_ok "Supervisor started pid=$DAEMON_PID socket=$PC_SOCKET_PATH"
    '';
  };

in
{
  inherit
    start
    stop
    startDaemon
    ;
}
