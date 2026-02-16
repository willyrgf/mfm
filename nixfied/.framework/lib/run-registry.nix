# Run registry - durable run management with background mode
{
  pkgs,
  project ? { },
}:

let
  projectId = (project.project or { }).id or "project";
  runsRoot = (project.ci or { }).runsRoot or "/tmp/${projectId}-runs";
  id = import ./id.nix {
    inherit pkgs project;
  };

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
    PLAN_ID="''${NIXFIED_PLAN_ID:-}"
    RUN_ID="$(${id.resolveId} "''${RUN_ID:-}" "$PLAN_ID")"
    RUN_DIR="$RUNS_ROOT/$RUN_ID"

    mkdir -p "$RUN_DIR"

    # Write runner script
    {
      printf '%s\n' '#!/usr/bin/env bash'
      printf '%s\n' 'set -euo pipefail'
      printf '%s\n' "$SCRIPT"
    } > "$RUN_DIR/runner.sh"
    chmod +x "$RUN_DIR/runner.sh"

    # Write initial meta.json
    SLOT_INFO=""
    if [ -n "''${SLOT:-}" ]; then SLOT_INFO="$SLOT"; fi
    ENV_INFO=""
    if [ -n "''${ENV:-}" ]; then ENV_INFO="$ENV"; fi

    ${pkgs.jq}/bin/jq -n \
      --arg run_id "$RUN_ID" \
      --arg target "$NAME" \
      --arg started_at "$(${pkgs.coreutils}/bin/date -u +%Y-%m-%dT%H:%M:%SZ)" \
      --arg plan_id "$PLAN_ID" \
      --arg slot "$SLOT_INFO" \
      --arg env "$ENV_INFO" \
      '
      {
        run_id: $run_id,
        plan_id: (if $plan_id == "" then null else $plan_id end),
        target: $target,
        status: "starting",
        started_at: $started_at,
        pid: null,
        slot: (if $slot == "" then null else (try ($slot | tonumber) catch $slot) end),
        env: (if $env == "" then null else $env end),
        exit_code: null,
        duration: null
      }
      ' > "$RUN_DIR/meta.json"

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
          echo "ERROR: Timeout after ''${TIMEOUT}s, killing run $RUN_ID" >&2
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

      echo "INFO: Run $RUN_ID started in background"
      echo "INFO: Dir: $RUN_DIR"
      echo "INFO: Log: $RUN_DIR/output.log"
      echo "INFO: Meta: $RUN_DIR/meta.json"
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
