# Run registry - durable run management with background mode
{
  pkgs,
  project ? { },
  loggingPrelude ? "",
}:

let
  commonRuntimeShell = import ../common-runtime.nix { inherit pkgs; };
  projectId = (project.project or { }).id or "project";
  runsRoot = (project.ci or { }).runsRoot or "/tmp/${projectId}-runs";
  id = import ./id.nix {
    inherit pkgs project;
  };

  runRegistryStart = pkgs.writeShellScript "run-registry-start" ''
    ${loggingPrelude}
    ${commonRuntimeShell}

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

    meta_fields_file() {
      printf '%s/meta.fields' "$RUN_DIR"
    }

    write_meta_fields() {
      local fields_file
      local tmp

      fields_file="$(meta_fields_file)"
      tmp="$RUN_DIR/meta.fields.tmp.$$"
      {
        printf 'META_RUN_ID=%q\n' "$META_RUN_ID"
        printf 'META_PLAN_ID=%q\n' "$META_PLAN_ID"
        printf 'META_TARGET=%q\n' "$META_TARGET"
        printf 'META_STATUS=%q\n' "$META_STATUS"
        printf 'META_STARTED_AT=%q\n' "$META_STARTED_AT"
        printf 'META_PID=%q\n' "$META_PID"
        printf 'META_SLOT=%q\n' "$META_SLOT"
        printf 'META_ENV=%q\n' "$META_ENV"
        printf 'META_EXIT_CODE=%q\n' "$META_EXIT_CODE"
        printf 'META_DURATION=%q\n' "$META_DURATION"
      } > "$tmp"
      mv "$tmp" "$fields_file"
    }

    load_meta_fields() {
      local fields_file

      fields_file="$(meta_fields_file)"
      if [ ! -f "$fields_file" ]; then
        echo "ERROR: missing run registry metadata for '$RUN_DIR'" >&2
        return 1
      fi

      unset \
        META_RUN_ID \
        META_PLAN_ID \
        META_TARGET \
        META_STATUS \
        META_STARTED_AT \
        META_PID \
        META_SLOT \
        META_ENV \
        META_EXIT_CODE \
        META_DURATION || true
      . "$fields_file"
    }

    write_meta_json() {
      local target_file="$1"
      local slot_json="null"

      if [[ "$META_SLOT" =~ ^[0-9]+$ ]]; then
        slot_json="$META_SLOT"
      else
        slot_json="$(json_string_or_null "$META_SLOT")"
      fi

      {
        printf '{'
        printf '"run_id":%s' "$(json_quote_string "$META_RUN_ID")"
        printf ',"plan_id":%s' "$(json_string_or_null "$META_PLAN_ID")"
        printf ',"target":%s' "$(json_quote_string "$META_TARGET")"
        printf ',"status":%s' "$(json_quote_string "$META_STATUS")"
        printf ',"started_at":%s' "$(json_quote_string "$META_STARTED_AT")"
        printf ',"pid":%s' "$(json_number_or_null "$META_PID")"
        printf ',"slot":%s' "$slot_json"
        printf ',"env":%s' "$(json_string_or_null "$META_ENV")"
        printf ',"exit_code":%s' "$(json_number_or_null "$META_EXIT_CODE")"
        printf ',"duration":%s' "$(json_number_or_null "$META_DURATION")"
        printf '}\n'
      } > "$target_file"
    }

    # Write initial meta.json
    SLOT_INFO=""
    if [ -n "''${SLOT:-}" ]; then SLOT_INFO="$SLOT"; fi
    ENV_INFO=""
    if [ -n "''${ENV:-}" ]; then ENV_INFO="$ENV"; fi

    META_RUN_ID="$RUN_ID"
    META_PLAN_ID="$PLAN_ID"
    META_TARGET="$NAME"
    META_STATUS="starting"
    META_STARTED_AT="$(${pkgs.coreutils}/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
    META_PID=""
    META_SLOT="$SLOT_INFO"
    META_ENV="$ENV_INFO"
    META_EXIT_CODE=""
    META_DURATION=""

    write_meta_json "$RUN_DIR/meta.json"
    write_meta_fields

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
        load_meta_fields
        META_STATUS="$status"
        META_PID="$$"
        if [ -n "$exit_code" ] && [ "$exit_code" != "null" ]; then
          META_EXIT_CODE="$exit_code"
        fi
        if [ -n "$duration" ] && [ "$duration" != "null" ]; then
          META_DURATION="$duration"
        fi
        write_meta_json "$TMP_META"
        mv "$TMP_META" "$RUN_DIR/meta.json"
        write_meta_fields
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
          log_error "Timeout after ''${TIMEOUT}s, killing run $RUN_ID"
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

      log_info "Run $RUN_ID started in background"
      log_info "Dir: $RUN_DIR"
      log_info "Log: $RUN_DIR/output.log"
      log_info "Meta: $RUN_DIR/meta.json"
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
