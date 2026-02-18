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

        ORDER_FILE="$(mktemp "''${TMPDIR:-/tmp}/nixfied-plan-order.XXXXXX")"
        RESULTS_FILE="$(mktemp "''${TMPDIR:-/tmp}/nixfied-plan-results.XXXXXX")"
        trap 'rm -f "$ORDER_FILE" "$RESULTS_FILE"' EXIT
        printf '[]\n' > "$RESULTS_FILE"

        if ! ${pkgs.jq}/bin/jq -r '
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
                    error("execution_plan_cycle_or_unsatisfied_dependencies")
                  else
                    loop($done + [$ready[0]])
                  end
              end;
          loop([]) | .[]
        ' "$PLAN_FILE" > "$ORDER_FILE"; then
          log_error "execution plan dependency graph is not resolvable"
          exit 1
        fi

        TOTAL_UNITS="$(wc -l < "$ORDER_FILE" | tr -d '[:space:]')"
        case "$TOTAL_UNITS" in
          *[!0-9]*|"")
            log_error "failed to compute execution order"
            exit 1
            ;;
        esac
        if [ "$TOTAL_UNITS" -eq 0 ]; then
          log_error "computed execution order is empty"
          exit 1
        fi

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

        PLAN_START="$(${pkgs.coreutils}/bin/date +%s)"
        STEP_INDEX=0
        STEPS_DURATION=0
        EXIT_CODE=0

        while IFS= read -r UNIT_NAME; do
          [ -z "$UNIT_NAME" ] && continue
          STEP_INDEX=$((STEP_INDEX + 1))

          UNIT_JSON="$(${pkgs.jq}/bin/jq -c --arg name "$UNIT_NAME" '.units[] | select(.name == $name)' "$PLAN_FILE")"
          if [ -z "$UNIT_JSON" ]; then
            log_error "execution unit missing name=$UNIT_NAME"
            EXIT_CODE=1
            break
          fi

          UNIT_ID="$(printf '%s\n' "$UNIT_JSON" | ${pkgs.jq}/bin/jq -r '.id // .name')"
          UNIT_DESC="$(printf '%s\n' "$UNIT_JSON" | ${pkgs.jq}/bin/jq -r '.description // .name')"
          export NIXFIED_UNIT_ID="$UNIT_ID"
          export NIXFIED_UNIT_ATTEMPT="1"

          echo ""
          log_step "$STEP_INDEX" "$TOTAL_UNITS" "$UNIT_DESC"
          emit_progress "$UNIT_NAME" "$UNIT_ID" "$STEP_INDEX"

          UNIT_START="$(${pkgs.coreutils}/bin/date +%s)"
          UNIT_STATUS=""
          UNIT_RC=0
          SKIP_REASON=""

          UNIT_MISSING="$(printf '%s\n' "$UNIT_JSON" | ${pkgs.jq}/bin/jq -r '(.missing // false) | tostring')"
          if [ "$UNIT_MISSING" = "true" ]; then
            echo "Unknown step: $UNIT_NAME" >&2
            UNIT_STATUS="failed"
            UNIT_RC=1
          fi

          if [ -z "$UNIT_STATUS" ]; then
            while IFS= read -r MISSING_VAR; do
              [ -z "$MISSING_VAR" ] && continue
              if [ -z "''${!MISSING_VAR:-}" ]; then
                SKIP_REASON="missing $MISSING_VAR"
                UNIT_STATUS="skipped"
                UNIT_RC=42
                break
              fi
            done < <(printf '%s\n' "$UNIT_JSON" | ${pkgs.jq}/bin/jq -r '.skip_if_missing[]?')
          fi

          if [ -z "$UNIT_STATUS" ]; then
            WHEN_EXPR="$(printf '%s\n' "$UNIT_JSON" | ${pkgs.jq}/bin/jq -r '.when // ""')"
            if [ -n "$WHEN_EXPR" ]; then
              if ! ${pkgs.bash}/bin/bash -c "$WHEN_EXPR"; then
                SKIP_REASON="condition not met"
                UNIT_STATUS="skipped"
                UNIT_RC=42
              fi
            fi
          fi

          if [ -n "$SKIP_REASON" ]; then
            log_skip "$UNIT_DESC: $SKIP_REASON"
          fi

          if [ -z "$UNIT_STATUS" ]; then
            while IFS= read -r ENTRY_B64; do
              [ -z "$ENTRY_B64" ] && continue
              KEY="$(printf '%s' "$ENTRY_B64" | ${pkgs.coreutils}/bin/base64 -d | ${pkgs.jq}/bin/jq -r '.key')"
              VALUE="$(printf '%s' "$ENTRY_B64" | ${pkgs.coreutils}/bin/base64 -d | ${pkgs.jq}/bin/jq -r '.value | tostring')"
              export "$KEY=$VALUE"
            done < <(printf '%s\n' "$UNIT_JSON" | ${pkgs.jq}/bin/jq -r '.env // {} | to_entries[] | @base64')

            RUN_SCRIPT_FILE="$(mktemp "''${TMPDIR:-/tmp}/nixfied-unit-run.XXXXXX")"
            {
              printf '%s\n' '#!/usr/bin/env bash'
              printf '%s\n' 'set -euo pipefail'
              printf '%s\n' "$UNIT_JSON" | ${pkgs.jq}/bin/jq -r '.run // ""'
            } > "$RUN_SCRIPT_FILE"
            chmod +x "$RUN_SCRIPT_FILE"

            set +e
            (
              if [ -n "$CONTEXT_SCRIPT" ]; then
                source "$CONTEXT_SCRIPT"
              fi
              source "$RUN_SCRIPT_FILE"
            )
            UNIT_RC=$?
            set -e
            rm -f "$RUN_SCRIPT_FILE"

            if [ "$UNIT_RC" -eq 0 ]; then
              UNIT_STATUS="passed"
            else
              UNIT_STATUS="failed"
            fi
          fi

          CLEANUP_SCRIPT="$(printf '%s\n' "$UNIT_JSON" | ${pkgs.jq}/bin/jq -r '.cleanup // ""')"
          if [ -n "$CLEANUP_SCRIPT" ]; then
            CLEANUP_FILE="$(mktemp "''${TMPDIR:-/tmp}/nixfied-unit-cleanup.XXXXXX")"
            {
              printf '%s\n' '#!/usr/bin/env bash'
              printf '%s\n' 'set -euo pipefail'
              printf '%s\n' "$CLEANUP_SCRIPT"
            } > "$CLEANUP_FILE"
            chmod +x "$CLEANUP_FILE"
            set +e
            (
              if [ -n "$CONTEXT_SCRIPT" ]; then
                source "$CONTEXT_SCRIPT"
              fi
              source "$CLEANUP_FILE"
            ) >/dev/null 2>&1
            set -e
            rm -f "$CLEANUP_FILE"
          fi

          UNIT_END="$(${pkgs.coreutils}/bin/date +%s)"
          UNIT_DURATION=$((UNIT_END - UNIT_START))
          STEPS_DURATION=$((STEPS_DURATION + UNIT_DURATION))

          record_unit "$UNIT_ID" "$UNIT_NAME" "$UNIT_STATUS" "$UNIT_DURATION"

          if [ "$UNIT_STATUS" = "failed" ]; then
            if [ "$UNIT_RC" -eq 0 ] || [ "$UNIT_RC" -eq 42 ]; then
              EXIT_CODE=1
            else
              EXIT_CODE="$UNIT_RC"
            fi
            break
          fi
        done < "$ORDER_FILE"

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
