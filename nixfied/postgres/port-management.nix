# PostgreSQL port management utilities
{ pkgs }:

let
  checkPort = pkgs.writeShellScript "postgres-check-port" ''
    set -euo pipefail
    PORT="''${1:-}"
    if [ -z "$PORT" ]; then
      echo "usage: postgres-check-port <port>" >&2
      exit 1
    fi

    if command -v lsof >/dev/null 2>&1; then
      PIDS=$(lsof -ti:"$PORT" 2>/dev/null || true)
      if [ -n "$PIDS" ]; then
        echo "Port $PORT is in use by PID(s): $PIDS"
        lsof -i:"$PORT" 2>/dev/null || true
        exit 1
      fi
    fi

    echo "✅ Port $PORT is available"
  '';

  getPortPids = pkgs.writeShellScript "postgres-get-port-pids" ''
    PORT="''${1:-}"
    if [ -z "$PORT" ]; then
      exit 0
    fi
    if command -v lsof >/dev/null 2>&1; then
      lsof -ti:"$PORT" 2>/dev/null || true
    fi
  '';

  getPortInfo = pkgs.writeShellScript "postgres-get-port-info" ''
    PORT="''${1:-}"
    if [ -z "$PORT" ]; then
      echo "usage: postgres-get-port-info <port>" >&2
      exit 1
    fi
    if command -v lsof >/dev/null 2>&1; then
      lsof -i:"$PORT" 2>/dev/null || echo "No processes on port $PORT"
    fi
  '';

  killPort = pkgs.writeShellScript "postgres-kill-port" ''
    set -euo pipefail
    PORT="''${1:-}"
    if [ -z "$PORT" ]; then
      echo "usage: postgres-kill-port <port>" >&2
      exit 1
    fi

    PIDS=$(lsof -ti:"$PORT" 2>/dev/null || true)
    if [ -z "$PIDS" ]; then
      echo "No processes on port $PORT"
      exit 0
    fi

    echo "🛑 Killing processes on port $PORT: $PIDS"
    echo "$PIDS" | xargs kill -TERM 2>/dev/null || true
    sleep 2

    REMAINING=$(lsof -ti:"$PORT" 2>/dev/null || true)
    if [ -n "$REMAINING" ]; then
      echo "   Force killing: $REMAINING"
      echo "$REMAINING" | xargs kill -KILL 2>/dev/null || true
    fi

    echo "✅ Port $PORT cleared"
  '';

  assertPortsFree = pkgs.writeShellScript "postgres-assert-ports-free" ''
    set -euo pipefail
    FAILED=0
    for PORT in "$@"; do
      if command -v lsof >/dev/null 2>&1; then
        if lsof -ti:"$PORT" >/dev/null 2>&1; then
          echo "❌ Port $PORT is in use"
          FAILED=1
        fi
      fi
    done
    if [ "$FAILED" -eq 1 ]; then
      exit 1
    fi
    echo "✅ All ports are available"
  '';

in
{
  inherit
    checkPort
    getPortPids
    getPortInfo
    killPort
    assertPortsFree
    ;
}
