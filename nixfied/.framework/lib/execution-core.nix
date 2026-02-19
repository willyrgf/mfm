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

        PLAN_ID="$(${pkgs.jq}/bin/jq -r '.plan_id // ""' "$PLAN_FILE")"
        if [ -z "$PLAN_ID" ]; then
          PLAN_CANONICAL="$(${pkgs.jq}/bin/jq -cS 'del(.plan_id)' "$PLAN_FILE")"
          PLAN_ID="$(printf '%s' "$PLAN_CANONICAL" | ${id.mkPlanId} --from-stdin)"
        fi
        export NIXFIED_PLAN_ID="$PLAN_ID"
        export RUN_ID="$(${id.resolveId} "''${RUN_ID:-}" "$PLAN_ID")"

        MODE_VALUE="$(${pkgs.jq}/bin/jq -r '.mode // ""' "$PLAN_FILE")"

        RESULTS_FILE="$(mktemp "''${TMPDIR:-/tmp}/nixfied-plan-results.XXXXXX")"
        UNIT_RESULT_DIR="$(mktemp -d "''${TMPDIR:-/tmp}/nixfied-plan-unit-results.XXXXXX")"
        DONE_FILE="$(mktemp "''${TMPDIR:-/tmp}/nixfied-plan-done.XXXXXX")"
        STARTED_FILE="$(mktemp "''${TMPDIR:-/tmp}/nixfied-plan-started.XXXXXX")"
        trap 'rm -f "$RESULTS_FILE" "$DONE_FILE" "$STARTED_FILE"; rm -rf "$UNIT_RESULT_DIR"' EXIT
        printf '[]\n' > "$RESULTS_FILE"
        : > "$DONE_FILE"
        : > "$STARTED_FILE"
        TOTAL_UNITS="$UNIT_COUNT"

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

        json_array_from_file() {
          local list_file="$1"
          if [ ! -s "$list_file" ]; then
            printf '[]\n'
            return 0
          fi
          ${pkgs.jq}/bin/jq -Rsc 'split("\n") | map(select(length > 0))' "$list_file"
        }

        resolve_ready_units() {
          local done_json="$1"
          local started_json="$2"

          ${pkgs.jq}/bin/jq -r \
            --argjson done "$done_json" \
            --argjson started "$started_json" \
            '
            def has_name($names; $name):
              ($names | index($name)) != null;

            def deps_satisfied($done; $deps):
              (($deps // []) | all(has_name($done; .)));

            [
              .units[]
              | select(
                  (has_name($started; .name) | not)
                  and (has_name($done; .name) | not)
                  and deps_satisfied($done; .depends_on)
                )
              | .name
            ]
            | sort
            | .[]?
            ' "$PLAN_FILE"
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

        export_unit_env() {
          local unit_json="$1"
          while IFS= read -r ENTRY_B64; do
            [ -z "$ENTRY_B64" ] && continue
            KEY="$(printf '%s' "$ENTRY_B64" | ${pkgs.coreutils}/bin/base64 -d | ${pkgs.jq}/bin/jq -r '.key')"
            VALUE="$(printf '%s' "$ENTRY_B64" | ${pkgs.coreutils}/bin/base64 -d | ${pkgs.jq}/bin/jq -r '.value | tostring')"
            export "$KEY=$VALUE"
          done < <(printf '%s\n' "$unit_json" | ${pkgs.jq}/bin/jq -r '.env // {} | to_entries[] | @base64')
        }

        write_unit_result() {
          local unit_result_file="$1"
          local unit_id="$2"
          local unit_name="$3"
          local unit_status="$4"
          local unit_duration="$5"
          local unit_rc="$6"

          ${pkgs.jq}/bin/jq -n \
            --arg id "$unit_id" \
            --arg name "$unit_name" \
            --arg status "$unit_status" \
            --arg duration "$unit_duration" \
            --arg rc "$unit_rc" \
            '{
              id: $id,
              name: $name,
              status: $status,
              duration: ($duration | tonumber),
              rc: ($rc | tonumber)
            }' > "$unit_result_file"
        }

        execute_unit() {
          local unit_name="$1"
          local unit_index="$2"
          local unit_result_file="$3"

          local unit_json=""
          local unit_id=""
          local unit_desc=""
          local unit_start=0
          local unit_end=0
          local unit_duration=0
          local unit_status=""
          local unit_rc=0
          local skip_reason=""
          local unit_missing=""
          local when_expr=""
          local run_script_file=""
          local cleanup_script=""
          local cleanup_file=""

          unit_json="$(${pkgs.jq}/bin/jq -c --arg name "$unit_name" '.units[] | select(.name == $name)' "$PLAN_FILE")"
          if [ -z "$unit_json" ]; then
            log_error "execution unit missing name=$unit_name"
            write_unit_result "$unit_result_file" "$unit_name" "$unit_name" "failed" "0" "1"
            return 0
          fi

          unit_id="$(printf '%s\n' "$unit_json" | ${pkgs.jq}/bin/jq -r '.id // .name')"
          unit_desc="$(printf '%s\n' "$unit_json" | ${pkgs.jq}/bin/jq -r '.description // .name')"

          export NIXFIED_UNIT_ID="$unit_id"
          export NIXFIED_UNIT_ATTEMPT="1"

          echo ""
          log_step "$unit_index" "$TOTAL_UNITS" "$unit_desc"
          emit_progress "$unit_name" "$unit_id" "$unit_index"

          unit_start="$(${pkgs.coreutils}/bin/date +%s)"
          unit_status=""
          unit_rc=0
          skip_reason=""

          unit_missing="$(printf '%s\n' "$unit_json" | ${pkgs.jq}/bin/jq -r '(.missing // false) | tostring')"
          if [ "$unit_missing" = "true" ]; then
            echo "Unknown step: $unit_name" >&2
            unit_status="failed"
            unit_rc=1
          fi

          if [ -z "$unit_status" ]; then
            while IFS= read -r MISSING_VAR; do
              [ -z "$MISSING_VAR" ] && continue
              if [ -z "''${!MISSING_VAR:-}" ]; then
                skip_reason="missing $MISSING_VAR"
                unit_status="skipped"
                unit_rc=42
                break
              fi
            done < <(printf '%s\n' "$unit_json" | ${pkgs.jq}/bin/jq -r '.skip_if_missing[]?')
          fi

          if [ -z "$unit_status" ]; then
            when_expr="$(printf '%s\n' "$unit_json" | ${pkgs.jq}/bin/jq -r '.when // ""')"
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

          if [ -z "$unit_status" ]; then
            run_script_file="$(mktemp "''${TMPDIR:-/tmp}/nixfied-unit-run.XXXXXX")"
            {
              printf '%s\n' '#!/usr/bin/env bash'
              printf '%s\n' 'set -euo pipefail'
              printf '%s\n' "$unit_json" | ${pkgs.jq}/bin/jq -r '.run // ""'
            } > "$run_script_file"
            chmod +x "$run_script_file"

            set +e
            (
              if [ -n "$CONTEXT_SCRIPT" ]; then
                source "$CONTEXT_SCRIPT"
              fi
              export_unit_env "$unit_json"
              source "$run_script_file"
            )
            unit_rc=$?
            set -e
            rm -f "$run_script_file"

            if [ "$unit_rc" -eq 0 ]; then
              unit_status="passed"
            else
              unit_status="failed"
            fi
          fi

          cleanup_script="$(printf '%s\n' "$unit_json" | ${pkgs.jq}/bin/jq -r '.cleanup // ""')"
          if [ -n "$cleanup_script" ]; then
            cleanup_file="$(mktemp "''${TMPDIR:-/tmp}/nixfied-unit-cleanup.XXXXXX")"
            {
              printf '%s\n' '#!/usr/bin/env bash'
              printf '%s\n' 'set -euo pipefail'
              printf '%s\n' "$cleanup_script"
            } > "$cleanup_file"
            chmod +x "$cleanup_file"
            set +e
            (
              if [ -n "$CONTEXT_SCRIPT" ]; then
                source "$CONTEXT_SCRIPT"
              fi
              export_unit_env "$unit_json"
              source "$cleanup_file"
            ) >/dev/null 2>&1
            set -e
            rm -f "$cleanup_file"
          fi

          unit_end="$(${pkgs.coreutils}/bin/date +%s)"
          unit_duration=$((unit_end - unit_start))

          write_unit_result "$unit_result_file" "$unit_id" "$unit_name" "$unit_status" "$unit_duration" "$unit_rc"
          return 0
        }

        PLAN_START="$(${pkgs.coreutils}/bin/date +%s)"
        STEP_INDEX=0
        STEPS_DURATION=0
        EXIT_CODE=0

        while true; do
          DONE_COUNT="$(wc -l < "$DONE_FILE" | tr -d '[:space:]')"
          case "$DONE_COUNT" in
            *[!0-9]*|"")
              log_error "failed to resolve completed-unit count"
              EXIT_CODE=1
              break
              ;;
          esac
          if [ "$DONE_COUNT" -ge "$TOTAL_UNITS" ]; then
            break
          fi

          DONE_JSON="$(json_array_from_file "$DONE_FILE")"
          STARTED_JSON="$(json_array_from_file "$STARTED_FILE")"
          READY_UNITS="$(resolve_ready_units "$DONE_JSON" "$STARTED_JSON")"
          if [ -z "$READY_UNITS" ]; then
            log_error "execution plan dependency graph is not resolvable"
            EXIT_CODE=1
            break
          fi

          WAVE_NAMES_FILE="$(mktemp "''${TMPDIR:-/tmp}/nixfied-wave-names.XXXXXX")"
          WAVE_PIDS_FILE="$(mktemp "''${TMPDIR:-/tmp}/nixfied-wave-pids.XXXXXX")"
          WAVE_RESULTS_FILE="$(mktemp "''${TMPDIR:-/tmp}/nixfied-wave-results.XXXXXX")"
          : > "$WAVE_NAMES_FILE"
          : > "$WAVE_PIDS_FILE"
          : > "$WAVE_RESULTS_FILE"

          while IFS= read -r UNIT_NAME; do
            [ -z "$UNIT_NAME" ] && continue
            STEP_INDEX=$((STEP_INDEX + 1))

            printf '%s\n' "$UNIT_NAME" >> "$STARTED_FILE"
            UNIT_RESULT_FILE="$(mktemp "$UNIT_RESULT_DIR/unit.XXXXXX.json")"
            (
              execute_unit "$UNIT_NAME" "$STEP_INDEX" "$UNIT_RESULT_FILE"
            ) &
            UNIT_PID=$!

            printf '%s\n' "$UNIT_NAME" >> "$WAVE_NAMES_FILE"
            printf '%s\n' "$UNIT_PID" >> "$WAVE_PIDS_FILE"
            printf '%s\n' "$UNIT_RESULT_FILE" >> "$WAVE_RESULTS_FILE"
          done <<< "$READY_UNITS"

          WAVE_FAILED=0
          exec 7<"$WAVE_NAMES_FILE"
          exec 8<"$WAVE_PIDS_FILE"
          exec 9<"$WAVE_RESULTS_FILE"
          while true; do
            IFS= read -r UNIT_NAME <&7 || break
            IFS= read -r UNIT_PID <&8 || UNIT_PID=""
            IFS= read -r UNIT_RESULT_FILE <&9 || UNIT_RESULT_FILE=""

            if [ -z "$UNIT_PID" ]; then
              WAIT_RC=1
            else
              set +e
              wait "$UNIT_PID"
              WAIT_RC=$?
              set -e
            fi

            UNIT_ID="$UNIT_NAME"
            UNIT_STATUS="failed"
            UNIT_DURATION="0"
            UNIT_RC="$WAIT_RC"
            if [ -n "$UNIT_RESULT_FILE" ] && [ -f "$UNIT_RESULT_FILE" ]; then
              UNIT_ID="$(${pkgs.jq}/bin/jq -r '.id // ""' "$UNIT_RESULT_FILE")"
              if [ -z "$UNIT_ID" ]; then
                UNIT_ID="$UNIT_NAME"
              fi
              UNIT_STATUS="$(${pkgs.jq}/bin/jq -r '.status // "failed"' "$UNIT_RESULT_FILE")"
              UNIT_DURATION="$(${pkgs.jq}/bin/jq -r '.duration // 0' "$UNIT_RESULT_FILE")"
              UNIT_RC="$(${pkgs.jq}/bin/jq -r '.rc // 1' "$UNIT_RESULT_FILE")"
            fi

            case "$UNIT_DURATION" in
              *[!0-9]*|"")
                UNIT_DURATION=0
                ;;
            esac
            case "$UNIT_RC" in
              *[!0-9-]*|"")
                UNIT_RC=1
                ;;
            esac

            STEPS_DURATION=$((STEPS_DURATION + UNIT_DURATION))
            record_unit "$UNIT_ID" "$UNIT_NAME" "$UNIT_STATUS" "$UNIT_DURATION"
            printf '%s\n' "$UNIT_NAME" >> "$DONE_FILE"

            if [ "$UNIT_STATUS" = "failed" ] || [ "$WAIT_RC" -ne 0 ]; then
              WAVE_FAILED=1
              if [ "$EXIT_CODE" -eq 0 ]; then
                NORMALIZED_RC="$UNIT_RC"
                if [ "$NORMALIZED_RC" -eq 0 ] && [ "$WAIT_RC" -ne 0 ]; then
                  NORMALIZED_RC="$WAIT_RC"
                fi
                if [ "$NORMALIZED_RC" -eq 0 ] || [ "$NORMALIZED_RC" -eq 42 ]; then
                  EXIT_CODE=1
                else
                  EXIT_CODE="$NORMALIZED_RC"
                fi
              fi
            fi
          done
          exec 7<&-
          exec 8<&-
          exec 9<&-
          rm -f "$WAVE_NAMES_FILE" "$WAVE_PIDS_FILE" "$WAVE_RESULTS_FILE"

          if [ "$WAVE_FAILED" -ne 0 ]; then
            break
          fi
        done

        PLAN_END="$(${pkgs.coreutils}/bin/date +%s)"
        PLAN_DURATION=$((PLAN_END - PLAN_START))

        ${pkgs.jq}/bin/jq -n \
          --argjson schema_version 2 \
          --arg plan_id "$PLAN_ID" \
          --arg mode "$MODE_VALUE" \
          --arg run_id "$RUN_ID" \
          --arg exit_code "$EXIT_CODE" \
          --arg steps_duration "$STEPS_DURATION" \
          --arg total_duration "$PLAN_DURATION" \
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
            steps: $steps[0]
          }
          ' > "$RESULT_FILE"

        exit "$EXIT_CODE"
  '';
in
{
  inherit runPlan;
}
