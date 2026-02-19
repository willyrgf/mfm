# Execution core for typed plan execution.
{
  pkgs,
  project ? { },
  loggingPrelude,
}:

let
  id = import ./id.nix {
    inherit pkgs project;
  };

  runPlan = pkgs.writeShellScript "execution-core-run-plan" ''
        ${loggingPrelude}

        set -euo pipefail

        PLAN_FILE=""
        RESULT_FILE=""
        EMIT_EVENT_CMD=""
        CONTEXT_SCRIPT=""

        usage() {
          cat <<'EOF'
    Usage: execution-core-run-plan --plan-file <path> --result-file <path> [--emit-event <path>] [--context-script <path>]
    EOF
        }

        while [ "$#" -gt 0 ]; do
          case "$1" in
            --plan-file)
              PLAN_FILE="''${2:-}"
              shift 2
              ;;
            --result-file)
              RESULT_FILE="''${2:-}"
              shift 2
              ;;
            --emit-event)
              EMIT_EVENT_CMD="''${2:-}"
              shift 2
              ;;
            --context-script)
              CONTEXT_SCRIPT="''${2:-}"
              shift 2
              ;;
            --help|-h)
              usage
              exit 0
              ;;
            *)
              log_error "unknown argument: $1"
              usage >&2
              exit 1
              ;;
          esac
        done

        if [ -z "$PLAN_FILE" ] || [ -z "$RESULT_FILE" ]; then
          log_error "--plan-file and --result-file are required"
          usage >&2
          exit 1
        fi

        if [ ! -f "$PLAN_FILE" ]; then
          log_error "plan file not found path=$PLAN_FILE"
          exit 1
        fi

        if [ -n "$CONTEXT_SCRIPT" ] && [ ! -f "$CONTEXT_SCRIPT" ]; then
          log_error "context script not found path=$CONTEXT_SCRIPT"
          exit 1
        fi

        UNIT_COUNT="$(${pkgs.jq}/bin/jq -r '(.units // []) | length' "$PLAN_FILE")"
        case "$UNIT_COUNT" in
          *[!0-9]*|"")
            log_error "invalid plan file; .units must be an array"
            exit 1
            ;;
        esac
        if [ "$UNIT_COUNT" -eq 0 ]; then
          log_error "execution plan must include at least one unit"
          exit 1
        fi

        MAX_WORKERS="$(${pkgs.jq}/bin/jq -r '.max_workers // 1' "$PLAN_FILE")"
        case "$MAX_WORKERS" in
          *[!0-9]*|"")
            log_error "invalid plan file; .max_workers must be an integer >= 1"
            exit 1
            ;;
        esac
        if [ "$MAX_WORKERS" -lt 1 ]; then
          log_error "invalid plan file; .max_workers must be >= 1"
          exit 1
        fi

        DUP_NAMES="$(${pkgs.jq}/bin/jq -r '
          [(.units // [])[].name]
          | group_by(.)
          | map(select(length > 1) | .[0])
          | .[]?
        ' "$PLAN_FILE")"
        if [ -n "$DUP_NAMES" ]; then
          log_error "execution plan has duplicate unit names:"
          echo "$DUP_NAMES" >&2
          exit 1
        fi

        MISSING_DEPS="$(${pkgs.jq}/bin/jq -r '
          [(.units // [])[].name] as $names
          | [(.units // [])[].depends_on[]? | select(($names | index(.)) == null)] | unique | .[]?
        ' "$PLAN_FILE")"
        if [ -n "$MISSING_DEPS" ]; then
          log_error "execution plan has unresolved dependencies:"
          echo "$MISSING_DEPS" >&2
          exit 1
        fi

        BAD_LOCK_ARRAYS="$(${pkgs.jq}/bin/jq -r '
          [(.units // [])[] | select((.locks // []) | type != "array") | .name] | .[]?
        ' "$PLAN_FILE")"
        if [ -n "$BAD_LOCK_ARRAYS" ]; then
          log_error "execution plan has invalid lock definitions (locks must be arrays):"
          echo "$BAD_LOCK_ARRAYS" >&2
          exit 1
        fi

        BAD_LOCK_VALUES="$(${pkgs.jq}/bin/jq -r '
          [
            (.units // [])[] as $unit
            | (($unit.locks // [])[]? | select(type != "string") | $unit.name)
          ] | .[]?
        ' "$PLAN_FILE")"
        if [ -n "$BAD_LOCK_VALUES" ]; then
          log_error "execution plan has non-string lock tokens:"
          echo "$BAD_LOCK_VALUES" >&2
          exit 1
        fi

        if ! ${pkgs.jq}/bin/jq -e '
          def done_has($done; $name):
            ($done | index($name)) != null;

          def deps_satisfied($done; $deps):
            (($deps // []) | all(done_has($done; .)));

          .units as $units
          | def loop($done):
              if ($done | length) == ($units | length) then
                $done
              else
                ([ $units[] | select((done_has($done; .name) | not) and deps_satisfied($done; .depends_on)) | .name ] | sort) as $ready
                | if ($ready | length) == 0 then
                    false
                  else
                    loop($done + [$ready[0]])
                  end
              end;
          (loop([]) | type) == "array"
        ' "$PLAN_FILE" >/dev/null; then
          log_error "execution plan dependency graph is not resolvable"
          exit 1
        fi

        PLAN_ID="$(${pkgs.jq}/bin/jq -r '.plan_id // ""' "$PLAN_FILE")"
        if [ -z "$PLAN_ID" ]; then
          PLAN_CANONICAL="$(${pkgs.jq}/bin/jq -cS 'del(.plan_id)' "$PLAN_FILE")"
          PLAN_ID="$(printf '%s' "$PLAN_CANONICAL" | ${id.mkPlanId} --from-stdin)"
        fi
        export NIXFIED_PLAN_ID="$PLAN_ID"
        export RUN_ID="$(${id.resolveId} "''${RUN_ID:-}" "$PLAN_ID")"

        MODE_VALUE="$(${pkgs.jq}/bin/jq -r '.mode // ""' "$PLAN_FILE")"

        UNIT_NAMES_FILE="$(mktemp "''${TMPDIR:-/tmp}/nixfied-plan-units.XXXXXX")"
        RESULTS_FILE="$(mktemp "''${TMPDIR:-/tmp}/nixfied-plan-results.XXXXXX")"
        trap 'rm -f "$UNIT_NAMES_FILE" "$RESULTS_FILE"' EXIT
        ${pkgs.jq}/bin/jq -r '.units[] | .name' "$PLAN_FILE" | sort > "$UNIT_NAMES_FILE"
        printf '[]\n' > "$RESULTS_FILE"

        TOTAL_UNITS="$(wc -l < "$UNIT_NAMES_FILE" | tr -d '[:space:]')"
        case "$TOTAL_UNITS" in
          *[!0-9]*|"")
            log_error "failed to compute execution units list"
            exit 1
            ;;
        esac
        if [ "$TOTAL_UNITS" -eq 0 ]; then
          log_error "execution plan unit list is empty"
          exit 1
        fi

        unit_field() {
          local unit_name="$1"
          local filter="$2"
          ${pkgs.jq}/bin/jq -r --arg name "$unit_name" ".units[] | select(.name == \$name) | $filter" "$PLAN_FILE"
        }

        unit_depends() {
          local unit_name="$1"
          unit_field "$unit_name" '.depends_on[]?'
        }

        unit_locks() {
          local unit_name="$1"
          unit_field "$unit_name" '.locks[]?'
        }

        unit_env_entries() {
          local unit_name="$1"
          unit_field "$unit_name" '.env // {} | to_entries[] | @base64'
        }

        write_unit_script() {
          local unit_name="$1"
          local mode="$2"
          local script_file="$3"
          local body_filter='.run // ""'

          if [ "$mode" = "cleanup" ]; then
            body_filter='.cleanup // ""'
          fi

          {
            printf '%s\n' '#!/usr/bin/env bash'
            printf '%s\n' 'set -euo pipefail'
            if [ -n "$CONTEXT_SCRIPT" ]; then
              printf 'source %q\n' "$CONTEXT_SCRIPT"
            fi
            while IFS= read -r ENTRY_B64; do
              [ -z "$ENTRY_B64" ] && continue
              KEY="$(printf '%s' "$ENTRY_B64" | ${pkgs.coreutils}/bin/base64 -d | ${pkgs.jq}/bin/jq -r '.key')"
              VALUE="$(printf '%s' "$ENTRY_B64" | ${pkgs.coreutils}/bin/base64 -d | ${pkgs.jq}/bin/jq -r '.value | tostring')"
              printf 'export %s=%q\n' "$KEY" "$VALUE"
            done < <(unit_env_entries "$unit_name")
            unit_field "$unit_name" "$body_filter"
          } > "$script_file"
          chmod +x "$script_file"
        }

        run_unit_process() {
          local unit_name="$1"
          local run_script_file=""
          local rc=0

          run_script_file="$(mktemp "''${TMPDIR:-/tmp}/nixfied-unit-run.XXXXXX")"
          write_unit_script "$unit_name" "run" "$run_script_file"

          set +e
          ${pkgs.bash}/bin/bash "$run_script_file"
          rc=$?
          set -e

          rm -f "$run_script_file"
          return "$rc"
        }

        run_cleanup_for_unit() {
          local unit_name="$1"
          local cleanup_script=""
          local cleanup_file=""

          cleanup_script="$(unit_field "$unit_name" '.cleanup // ""')"
          if [ -z "$cleanup_script" ]; then
            return 0
          fi

          cleanup_file="$(mktemp "''${TMPDIR:-/tmp}/nixfied-unit-cleanup.XXXXXX")"
          write_unit_script "$unit_name" "cleanup" "$cleanup_file"

          set +e
          ${pkgs.bash}/bin/bash "$cleanup_file" >/dev/null 2>&1
          set -e

          rm -f "$cleanup_file"
          return 0
        }

        normalize_failure_code() {
          local rc="$1"
          if [ "$rc" -eq 0 ] || [ "$rc" -eq 42 ]; then
            printf '1\n'
          else
            printf '%s\n' "$rc"
          fi
        }

        record_unit() {
          local unit_id="$1"
          local unit_name="$2"
          local unit_status="$3"
          local unit_duration="$4"
          local tmp_results="$RESULTS_FILE.tmp.$$"

          ${pkgs.jq}/bin/jq \
            --arg id "$unit_id" \
            --arg name "$unit_name" \
            --arg status "$unit_status" \
            --arg duration "$unit_duration" \
            '. + [{
              id: $id,
              name: $name,
              status: $status,
              duration: ($duration | tonumber)
            }]' \
            "$RESULTS_FILE" > "$tmp_results"
          mv "$tmp_results" "$RESULTS_FILE"
        }

        emit_progress() {
          local unit_name="$1"
          local unit_id="$2"
          local index="$3"
          if [ -z "$EMIT_EVENT_CMD" ]; then
            return 0
          fi
          "$EMIT_EVENT_CMD" \
            --event-type readiness_progress \
            --state waiting \
            --run-id "$RUN_ID" \
            --plan-id "$PLAN_ID" \
            --unit-id "$unit_id" \
            --attempt "1" \
            --command "''${COMMAND_NAME:-unknown}" \
            --wait-reason "unit=$unit_name unit_id=$unit_id index=$index total=$TOTAL_UNITS plan_id=$PLAN_ID attempt=1" >/dev/null 2>&1 || true
          return 0
        }

        declare -A UNIT_STARTED=()
        declare -A UNIT_STATUS=()
        declare -A UNIT_START=()
        declare -A UNIT_ID=()
        declare -A UNIT_DESC=()
        declare -A PID_TO_UNIT=()
        declare -A LOCK_OWNER=()
        declare -A CANCEL_REQUESTED=()

        STEP_INDEX=0
        RUNNING_COUNT=0
        COMPLETED_COUNT=0
        STEPS_DURATION=0
        PEAK_WORKERS=0
        CANCELED_COUNT=0
        FAILED_FLAG=0
        EXIT_CODE=0

        deps_satisfied() {
          local unit_name="$1"
          local dep=""
          local dep_status=""

          while IFS= read -r dep; do
            [ -z "$dep" ] && continue
            dep_status="''${UNIT_STATUS[$dep]:-}"
            case "$dep_status" in
              passed|skipped) ;;
              *) return 1 ;;
            esac
          done < <(unit_depends "$unit_name")
          return 0
        }

        has_lock_conflict() {
          local unit_name="$1"
          local lock=""
          local owner=""

          while IFS= read -r lock; do
            [ -z "$lock" ] && continue
            owner="''${LOCK_OWNER[$lock]:-}"
            if [ -n "$owner" ]; then
              return 0
            fi
          done < <(unit_locks "$unit_name")
          return 1
        }

        assign_unit_locks() {
          local unit_name="$1"
          local lock=""
          while IFS= read -r lock; do
            [ -z "$lock" ] && continue
            LOCK_OWNER["$lock"]="$unit_name"
          done < <(unit_locks "$unit_name")
        }

        release_unit_locks() {
          local unit_name="$1"
          local lock=""
          while IFS= read -r lock; do
            [ -z "$lock" ] && continue
            if [ "''${LOCK_OWNER[$lock]:-}" = "$unit_name" ]; then
              unset "LOCK_OWNER[$lock]"
            fi
          done < <(unit_locks "$unit_name")
        }

        finalize_unit() {
          local unit_name="$1"
          local unit_status="$2"
          local unit_rc="$3"
          local unit_start="$4"
          local unit_end="$5"
          local unit_duration=0

          unit_duration=$((unit_end - unit_start))
          if [ "$unit_duration" -lt 0 ]; then
            unit_duration=0
          fi

          UNIT_STARTED["$unit_name"]=1
          UNIT_STATUS["$unit_name"]="$unit_status"

          run_cleanup_for_unit "$unit_name"
          record_unit "''${UNIT_ID[$unit_name]}" "$unit_name" "$unit_status" "$unit_duration"

          COMPLETED_COUNT=$((COMPLETED_COUNT + 1))
          STEPS_DURATION=$((STEPS_DURATION + unit_duration))
          if [ "$unit_status" = "canceled" ]; then
            CANCELED_COUNT=$((CANCELED_COUNT + 1))
          fi
          return 0
        }

        cancel_running_units() {
          local pid=""
          local unit_name=""
          local has_running=0

          for pid in "''${!PID_TO_UNIT[@]}"; do
            unit_name="''${PID_TO_UNIT[$pid]:-}"
            [ -z "$unit_name" ] && continue
            has_running=1
            CANCEL_REQUESTED["$unit_name"]=1
            kill -TERM "$pid" 2>/dev/null || true
          done

          if [ "$has_running" -eq 0 ]; then
            return 0
          fi

          sleep 5
          for pid in "''${!PID_TO_UNIT[@]}"; do
            if kill -0 "$pid" 2>/dev/null; then
              kill -KILL "$pid" 2>/dev/null || true
            fi
          done
          return 0
        }

        mark_not_started_as_canceled() {
          local unit_name=""
          local ts=0
          while IFS= read -r unit_name; do
            [ -z "$unit_name" ] && continue
            if [ "''${UNIT_STARTED[$unit_name]:-0}" = "1" ]; then
              continue
            fi
            UNIT_ID["$unit_name"]="$(unit_field "$unit_name" '.id // .name')"
            UNIT_DESC["$unit_name"]="$(unit_field "$unit_name" '.description // .name')"
            ts="$(${pkgs.coreutils}/bin/date +%s)"
            finalize_unit "$unit_name" "canceled" 0 "$ts" "$ts"
          done < "$UNIT_NAMES_FILE"
        }

        next_ready_unit() {
          local unit_name=""
          while IFS= read -r unit_name; do
            [ -z "$unit_name" ] && continue
            if [ "''${UNIT_STARTED[$unit_name]:-0}" = "1" ]; then
              continue
            fi
            if ! deps_satisfied "$unit_name"; then
              continue
            fi
            if has_lock_conflict "$unit_name"; then
              continue
            fi
            printf '%s\n' "$unit_name"
            return 0
          done < "$UNIT_NAMES_FILE"
          return 1
        }

        start_unit() {
          local unit_name="$1"
          local unit_start=0
          local unit_end=0
          local unit_id=""
          local unit_desc=""
          local unit_missing=""
          local unit_status=""
          local unit_rc=0
          local skip_reason=""
          local when_expr=""
          local missing_var=""
          local unit_pid=0

          unit_start="$(${pkgs.coreutils}/bin/date +%s)"
          unit_id="$(unit_field "$unit_name" '.id // .name')"
          unit_desc="$(unit_field "$unit_name" '.description // .name')"

          UNIT_ID["$unit_name"]="$unit_id"
          UNIT_DESC["$unit_name"]="$unit_desc"
          CANCEL_REQUESTED["$unit_name"]=0

          STEP_INDEX=$((STEP_INDEX + 1))
          export NIXFIED_UNIT_ID="$unit_id"
          export NIXFIED_UNIT_ATTEMPT="1"

          echo ""
          log_step "$STEP_INDEX" "$TOTAL_UNITS" "$unit_desc"
          emit_progress "$unit_name" "$unit_id" "$STEP_INDEX"

          unit_missing="$(unit_field "$unit_name" '(.missing // false) | tostring')"
          if [ "$unit_missing" = "true" ]; then
            echo "Unknown step: $unit_name" >&2
            unit_status="failed"
            unit_rc=1
          fi

          if [ -z "$unit_status" ]; then
            while IFS= read -r missing_var; do
              [ -z "$missing_var" ] && continue
              if [ -z "''${!missing_var:-}" ]; then
                skip_reason="missing $missing_var"
                unit_status="skipped"
                unit_rc=42
                break
              fi
            done < <(unit_field "$unit_name" '.skip_if_missing[]?')
          fi

          if [ -z "$unit_status" ]; then
            when_expr="$(unit_field "$unit_name" '.when // ""')"
            if [ -n "$when_expr" ]; then
              if ! ${pkgs.bash}/bin/bash -c "$when_expr"; then
                skip_reason="condition not met"
                unit_status="skipped"
                unit_rc=42
              fi
            fi
          fi

          if [ -n "$skip_reason" ]; then
            log_skip "$unit_desc: $skip_reason"
          fi

          if [ -n "$unit_status" ]; then
            unit_end="$(${pkgs.coreutils}/bin/date +%s)"
            finalize_unit "$unit_name" "$unit_status" "$unit_rc" "$unit_start" "$unit_end"
            if [ "$unit_status" = "failed" ] && [ "$FAILED_FLAG" -eq 0 ]; then
              FAILED_FLAG=1
              EXIT_CODE="$(normalize_failure_code "$unit_rc")"
              cancel_running_units
            fi
            return 0
          fi

          (
            run_unit_process "$unit_name"
          ) &
          unit_pid=$!

          PID_TO_UNIT["$unit_pid"]="$unit_name"
          UNIT_STARTED["$unit_name"]=1
          UNIT_START["$unit_name"]="$unit_start"
          assign_unit_locks "$unit_name"

          RUNNING_COUNT=$((RUNNING_COUNT + 1))
          if [ "$RUNNING_COUNT" -gt "$PEAK_WORKERS" ]; then
            PEAK_WORKERS="$RUNNING_COUNT"
          fi
          return 0
        }

        PLAN_START="$(${pkgs.coreutils}/bin/date +%s)"

        while [ "$COMPLETED_COUNT" -lt "$TOTAL_UNITS" ]; do
          if [ "$FAILED_FLAG" -eq 0 ]; then
            while [ "$RUNNING_COUNT" -lt "$MAX_WORKERS" ]; do
              READY_UNIT="$(next_ready_unit || true)"
              if [ -z "$READY_UNIT" ]; then
                break
              fi
              start_unit "$READY_UNIT"
              if [ "$FAILED_FLAG" -ne 0 ]; then
                break
              fi
            done
          fi

          if [ "$RUNNING_COUNT" -eq 0 ]; then
            if [ "$FAILED_FLAG" -ne 0 ]; then
              mark_not_started_as_canceled
              break
            fi
            if [ "$COMPLETED_COUNT" -lt "$TOTAL_UNITS" ]; then
              log_error "execution plan is blocked with no runnable units"
              EXIT_CODE=1
              FAILED_FLAG=1
              mark_not_started_as_canceled
            fi
            break
          fi

          DONE_PID=""
          set +e
          wait -n -p DONE_PID
          WAIT_RC=$?
          set -e

          DONE_UNIT="''${PID_TO_UNIT[$DONE_PID]:-}"
          if [ -z "$DONE_UNIT" ]; then
            continue
          fi
          unset "PID_TO_UNIT[$DONE_PID]"

          RUNNING_COUNT=$((RUNNING_COUNT - 1))
          release_unit_locks "$DONE_UNIT"

          UNIT_END="$(${pkgs.coreutils}/bin/date +%s)"
          UNIT_BEGIN="''${UNIT_START[$DONE_UNIT]:-$UNIT_END}"
          unset "UNIT_START[$DONE_UNIT]"

          UNIT_STATUS_VALUE=""
          UNIT_RC_VALUE=0
          if [ "''${CANCEL_REQUESTED[$DONE_UNIT]:-0}" = "1" ]; then
            UNIT_STATUS_VALUE="canceled"
            UNIT_RC_VALUE=0
          elif [ "$WAIT_RC" -eq 0 ]; then
            UNIT_STATUS_VALUE="passed"
            UNIT_RC_VALUE=0
          else
            UNIT_STATUS_VALUE="failed"
            UNIT_RC_VALUE="$WAIT_RC"
          fi

          finalize_unit "$DONE_UNIT" "$UNIT_STATUS_VALUE" "$UNIT_RC_VALUE" "$UNIT_BEGIN" "$UNIT_END"

          if [ "$UNIT_STATUS_VALUE" = "failed" ] && [ "$FAILED_FLAG" -eq 0 ]; then
            FAILED_FLAG=1
            EXIT_CODE="$(normalize_failure_code "$UNIT_RC_VALUE")"
            cancel_running_units
          fi
        done

        PLAN_END="$(${pkgs.coreutils}/bin/date +%s)"
        PLAN_DURATION=$((PLAN_END - PLAN_START))
        if [ "$PLAN_DURATION" -lt 0 ]; then
          PLAN_DURATION=0
        fi

        ${pkgs.jq}/bin/jq -n \
          --argjson schema_version 2 \
          --arg plan_id "$PLAN_ID" \
          --arg mode "$MODE_VALUE" \
          --arg run_id "$RUN_ID" \
          --arg exit_code "$EXIT_CODE" \
          --arg steps_duration "$STEPS_DURATION" \
          --arg total_duration "$PLAN_DURATION" \
          --arg max_workers "$MAX_WORKERS" \
          --arg peak_workers "$PEAK_WORKERS" \
          --arg canceled_count "$CANCELED_COUNT" \
          --slurpfile steps "$RESULTS_FILE" \
          '
          {
            schema_version: $schema_version,
            plan_id: $plan_id,
            mode: (if $mode == "" then null else $mode end),
            run_id: $run_id,
            exit_code: ($exit_code | tonumber),
            steps_duration: ($steps_duration | tonumber),
            total_duration: ($total_duration | tonumber),
            timing: {
              parallelism: {
                max_workers: ($max_workers | tonumber),
                peak_workers: ($peak_workers | tonumber),
                canceled_count: ($canceled_count | tonumber)
              }
            },
            steps: $steps[0]
          }
          ' > "$RESULT_FILE"

        exit "$EXIT_CODE"
  '';
in
{
  inherit runPlan;
}
