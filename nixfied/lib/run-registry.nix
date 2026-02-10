# Run registry - durable run management with background mode
{
  pkgs,
  project ? { },
}:

let
  projectId = (project.project or { }).id or "project";
  runsRoot = (project.ci or { }).runsRoot or "/tmp/${projectId}-runs";

  runRegistryStart = pkgs.writeShellScript "run-registry-start" ''
    set -euo pipefail

    NAME=""
    SCRIPT=""
    BACKGROUND=false
    TIMEOUT=""

    while [ "$#" -gt 0 ]; do
      case "$1" in
        --name) NAME="$2"; shift 2 ;;
        --script) SCRIPT="$2"; shift 2 ;;
        --bg) BACKGROUND=true; shift ;;
        --timeout) TIMEOUT="$2"; shift 2 ;;
        *) break ;;
      esac
    done

    if [ -z "$NAME" ] || [ -z "$SCRIPT" ]; then
      echo "Usage: run-registry-start --name <name> --script <script> [--bg] [--timeout <seconds>]" >&2
      exit 1
    fi

    RUNS_ROOT="${runsRoot}"
    RUN_ID="$(date +%Y%m%d-%H%M%S)-$(head -c 4 /dev/urandom | od -An -tx1 | tr -d ' \n')"
    RUN_DIR="$RUNS_ROOT/$RUN_ID"

    mkdir -p "$RUN_DIR"

    # Write runner script
    cat > "$RUN_DIR/runner.sh" <<RUNNER_EOF
    #!/usr/bin/env bash
    set -euo pipefail
    $SCRIPT
    RUNNER_EOF
    chmod +x "$RUN_DIR/runner.sh"

    # Write initial meta.json
    SLOT_INFO=""
    if [ -n "''${SLOT:-}" ]; then SLOT_INFO="$SLOT"; fi
    ENV_INFO=""
    if [ -n "''${ENV:-}" ]; then ENV_INFO="$ENV"; fi

    cat > "$RUN_DIR/meta.json" <<META_EOF
    {
      "run_id": "$RUN_ID",
      "target": "$NAME",
      "status": "starting",
      "started_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
      "pid": null,
      "slot": $(if [ -n "$SLOT_INFO" ]; then echo "$SLOT_INFO"; else echo "null"; fi),
      "env": $(if [ -n "$ENV_INFO" ]; then echo "\"$ENV_INFO\""; else echo "null"; fi),
      "exit_code": null,
      "duration": null
    }
    META_EOF

    export RUN_DIR="$RUN_DIR"
    export RUN_ID="$RUN_ID"

    _update_meta() {
      local status="$1"
      local exit_code="''${2:-null}"
      local duration="''${3:-null}"

      LOCK="$RUN_DIR/meta.lock"
      TMP_META="$RUN_DIR/meta.json.tmp.$$"

      (
        exec 9>"$LOCK"
        ${pkgs.flock}/bin/flock -x 9
        ${pkgs.jq}/bin/jq \
          --arg s "$status" \
          --arg ec "$exit_code" \
          --arg d "$duration" \
          --arg pid "$$" \
          '.status = $s
           | .pid = ($pid | tonumber)
           | .exit_code = (if $ec == "null" then null else ($ec | tonumber) end)
           | .duration = (if $d == "null" then null else ($d | tonumber) end)' \
          "$RUN_DIR/meta.json" > "$TMP_META" && mv "$TMP_META" "$RUN_DIR/meta.json"
      )
    }

    _run_and_track() {
      _update_meta "running"

      local START_TIME=$(date +%s)
      local RC=0

      # Signal handling
      _on_signal() {
        local CHILD_PID="''${RUNNER_PID:-}"
        if [ -n "$CHILD_PID" ] && kill -0 "$CHILD_PID" 2>/dev/null; then
          kill -TERM "$CHILD_PID" 2>/dev/null || true
          wait "$CHILD_PID" 2>/dev/null || true
        fi
        local END_TIME=$(date +%s)
        local DUR=$((END_TIME - START_TIME))
        _update_meta "failed" "130" "$DUR"
        exit 130
      }
      trap _on_signal TERM INT HUP

      if [ -n "$TIMEOUT" ]; then
        (
          sleep "$TIMEOUT"
          echo "⏰ Timeout after ''${TIMEOUT}s, killing run $RUN_ID" >&2
          kill -TERM $$ 2>/dev/null || true
          sleep 5
          kill -KILL $$ 2>/dev/null || true
        ) &
        WATCHDOG_PID=$!
      fi

      bash "$RUN_DIR/runner.sh" > "$RUN_DIR/output.log" 2>&1 &
      RUNNER_PID=$!

      wait "$RUNNER_PID" || RC=$?

      if [ -n "''${WATCHDOG_PID:-}" ]; then
        kill "$WATCHDOG_PID" 2>/dev/null || true
      fi

      local END_TIME=$(date +%s)
      local DUR=$((END_TIME - START_TIME))

      if [ "$RC" -eq 0 ]; then
        _update_meta "passed" "0" "$DUR"
      elif [ "$RC" -eq 124 ]; then
        _update_meta "timed_out" "$RC" "$DUR"
      else
        _update_meta "failed" "$RC" "$DUR"
      fi

      return $RC
    }

    if [ "$BACKGROUND" = true ]; then
      # Background mode: detach
      (
        _run_and_track
      ) &
      disown

      echo "🚀 Run $RUN_ID started in background"
      echo "   Dir: $RUN_DIR"
      echo "   Log: $RUN_DIR/output.log"
      echo "   Meta: $RUN_DIR/meta.json"
      exit 0
    else
      # Foreground mode: stream output
      _run_and_track &
      TRACK_PID=$!

      # Stream output
      touch "$RUN_DIR/output.log"
      tail -f "$RUN_DIR/output.log" --pid="$TRACK_PID" 2>/dev/null &
      TAIL_PID=$!

      wait "$TRACK_PID"
      RC=$?

      kill "$TAIL_PID" 2>/dev/null || true

      exit $RC
    fi
  '';

in
{
  inherit runRegistryStart;
}
