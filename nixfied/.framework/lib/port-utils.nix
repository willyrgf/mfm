# Port management utilities - cleanup and conflict checking
{
  pkgs,
  loggingPrelude,
}:

let
  mkPortCleanup = pkgs.writeShellScript "port-cleanup" ''
    ${loggingPrelude}

    cleanup_port() {
      local port=$1
      local name=$2

      log_info "Cleaning $name processes on port $port"

      if command -v lsof >/dev/null 2>&1; then
        PIDS=$(lsof -ti:$port 2>/dev/null || true)
        if [ -n "$PIDS" ]; then
          echo "   Found processes: $PIDS"
          echo "$PIDS" | xargs kill -TERM 2>/dev/null || true
          sleep 2
          echo "$PIDS" | xargs kill -KILL 2>/dev/null || true
        fi
      else
        ${pkgs.procps}/bin/netstat -tlnp 2>/dev/null | grep ":$port " | awk '/LISTEN/ {print $7}' | cut -d'/' -f1 | xargs kill -TERM 2>/dev/null || true
        sleep 2
        ${pkgs.procps}/bin/netstat -tlnp 2>/dev/null | grep ":$port " | awk '/LISTEN/ {print $7}' | cut -d'/' -f1 | xargs kill -KILL 2>/dev/null || true
      fi
    }

    for port in "$@"; do
      cleanup_port "$port" "service"
    done
  '';

  mkPortConflictChecker = pkgs.writeShellScript "port-conflict-checker" ''
    ${loggingPrelude}

    check_port() {
      local port=$1
      local name=$2

      if command -v lsof >/dev/null 2>&1; then
        if lsof -i:$port >/dev/null 2>&1; then
          log_error "Port $port ($name) is already in use"
          return 1
        fi
      else
        if ${pkgs.procps}/bin/netstat -tln 2>/dev/null | grep ":$port " >/dev/null; then
          log_error "Port $port ($name) is already in use"
          return 1
        fi
      fi
      log_ok "Port $port ($name) is available"
      return 0
    }

    CONFLICT=false
    for port in "$@"; do
      if ! check_port "$port" "service"; then
        CONFLICT=true
      fi
    done

    if [ "$CONFLICT" = "true" ]; then
      exit 1
    fi

    exit 0
  '';
in
{
  inherit mkPortCleanup mkPortConflictChecker;
}
