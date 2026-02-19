{
  pkgs,
  model,
  registry,
  projectRoot,
}:
let
  modelFile = pkgs.writeText "nixfied-model.json" (builtins.toJSON model);
  registryShell = registry.events.mkShellLib { };
  envSandboxShell = import ./env-sandbox.nix {
    inherit
      pkgs
      projectRoot
      ;
  };
in
pkgs.writeShellScriptBin "nixfied-executor" ''
  set -euo pipefail

  MODEL_FILE=${pkgs.lib.escapeShellArg (builtins.toString modelFile)}
  PROJECT_ROOT=${pkgs.lib.escapeShellArg (builtins.toString projectRoot)}
  REGISTRY_ROOT_DEFAULT=${pkgs.lib.escapeShellArg model.state.registry.root}
  REGISTRY_ROOT="''${REGISTRY_ROOT:-$REGISTRY_ROOT_DEFAULT}"

  ${registryShell}
  ${envSandboxShell}

  sha256_text() {
    printf '%s' "$1" | ${pkgs.coreutils}/bin/sha256sum | ${pkgs.gawk}/bin/awk '{print $1}'
  }

  normalize_args_hash() {
    local args_payload="$1"
    sha256_text "$args_payload"
  }

  normalize_env_hash() {
    ${pkgs.coreutils}/bin/env | ${pkgs.coreutils}/bin/sort | ${pkgs.coreutils}/bin/sha256sum | ${pkgs.gawk}/bin/awk '{print $1}'
  }

  RUN_SUFFIX_REASON=""

  compute_run_id() {
    local mode="$1"
    local workflow_id="$2"
    local task_id="$3"
    local args_payload="$4"

    local slot_var=${pkgs.lib.escapeShellArg model.runtime.slot.var}
    local env_var=${pkgs.lib.escapeShellArg model.runtime.env.var}
    local slot_default=${toString model.runtime.slot.default}
    local env_default=${pkgs.lib.escapeShellArg model.runtime.env.default}

    local slot_value
    local env_value
    local args_hash
    local env_hash
    local run_input
    local run_base
    local run_id

    slot_value="''${!slot_var:-$slot_default}"
    env_value="''${!env_var:-$env_default}"

    args_hash="$(normalize_args_hash "$args_payload")"
    env_hash="$(normalize_env_hash)"

    run_input="run-id|${model.identity.evalHash}|$workflow_id|$task_id|$mode|$slot_value|$env_value|$args_hash|$env_hash"
    run_base="$(sha256_text "$run_input")"
    run_id="run-''${run_base:0:24}"
    RUN_SUFFIX_REASON=""

    mkdir -p "$REGISTRY_ROOT/active" "$REGISTRY_ROOT/counters"

    if [ -e "$REGISTRY_ROOT/active/$run_id" ]; then
      local lock_dir="$REGISTRY_ROOT/counters/.lock-$run_base"
      local counter_file="$REGISTRY_ROOT/counters/$run_base"
      local counter="0"

      registry_lock_acquire "$lock_dir"
      if [ -f "$counter_file" ]; then
        counter="$(cat "$counter_file")"
      fi
      counter="$(( counter + 1 ))"
      printf '%s' "$counter" > "$counter_file"
      registry_lock_release "$lock_dir"

      run_id="$run_id-$(printf 'c%03d' "$counter")"
      RUN_SUFFIX_REASON="active-collision"
    fi

    printf '%s' "$run_id"
  }

  activate_run() {
    local run_id="$1"
    mkdir -p "$REGISTRY_ROOT/active"
    : > "$REGISTRY_ROOT/active/$run_id"
  }

  deactivate_run() {
    local run_id="$1"
    rm -f "$REGISTRY_ROOT/active/$run_id"
  }

  task_json() {
    local task_id="$1"
    ${pkgs.jq}/bin/jq -c --arg taskId "$task_id" '.tasks[$taskId] // empty' "$MODEL_FILE"
  }

  workflow_json() {
    local workflow_id="$1"
    ${pkgs.jq}/bin/jq -c --arg workflowId "$workflow_id" '.workflows[$workflowId] // empty' "$MODEL_FILE"
  }

  append_event() {
    local run_id="$1"
    local workflow_id="$2"
    local task_id="$3"
    local state="$4"
    local detail_json="$5"

    registry_append_event "$REGISTRY_ROOT" "$run_id" "$workflow_id" "$task_id" "$state" "$detail_json"
  }

  execute_task_body() {
    local task_id="$1"
    shift

    local task
    local runner_type
    local command
    local nested_workflow
    local exit_code

    task="$(task_json "$task_id")"
    if [ -z "$task" ]; then
      echo "ERROR: unknown task '$task_id'"
      return 2
    fi

    runner_type="$(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r '.runner.type')"

    set +e
    case "$runner_type" in
      shell)
        command="$(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r '.runner.command')"
        run_in_sandbox "$task" "$command" "$@"
        exit_code="$?"
        ;;
      workflowRef)
        nested_workflow="$(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r '.runner.workflowId // empty')"
        if [ -z "$nested_workflow" ]; then
          echo "ERROR: task '$task_id' runner.workflowId is empty"
          exit_code=3
        else
          run_workflow "$nested_workflow" "$@"
          exit_code="$?"
        fi
        ;;
      derivation)
        echo "ERROR: derivation runner is not implemented for task '$task_id'"
        exit_code=3
        ;;
      *)
        echo "ERROR: unsupported runner type '$runner_type' for task '$task_id'"
        exit_code=3
        ;;
    esac
    set -e

    return "$exit_code"
  }

  execute_task() {
    local run_id="$1"
    local workflow_id="$2"
    local task_id="$3"
    shift 3

    local detail_json
    local exit_code

    detail_json="$(${pkgs.jq}/bin/jq -cn --arg mode "task" '{mode: $mode}')"
    append_event "$run_id" "$workflow_id" "$task_id" "queued" "$detail_json"
    append_event "$run_id" "$workflow_id" "$task_id" "running" '{}'

    if execute_task_body "$task_id" "$@"; then
      exit_code=0
    else
      exit_code="$?"
    fi

    if [ "$exit_code" -eq 0 ]; then
      append_event "$run_id" "$workflow_id" "$task_id" "passed" '{}'
    else
      detail_json="$(${pkgs.jq}/bin/jq -cn --argjson exitCode "$exit_code" '{exitCode: $exitCode}')"
      append_event "$run_id" "$workflow_id" "$task_id" "failed" "$detail_json"
      return "$exit_code"
    fi
  }

  run_task() {
    if [ "$#" -lt 1 ]; then
      echo "ERROR: usage: run-task <task-id> [-- ...]"
      return 2
    fi

    local task_id="$1"
    shift

    local args_payload=""
    local run_id
    local detail_json
    local status

    args_payload="$(printf '%s\n' "$@")"
    run_id="$(compute_run_id "task" "" "$task_id" "$args_payload")"

    activate_run "$run_id"
    trap "deactivate_run '$run_id'" EXIT

    detail_json="$(${pkgs.jq}/bin/jq -cn --arg suffix "$RUN_SUFFIX_REASON" '{mode: "task", suffixReason: (if $suffix == "" then null else $suffix end)}')"
    append_event "$run_id" "" "$task_id" "queued" "$detail_json"

    set +e
    execute_task "$run_id" "" "$task_id" "$@"
    status="$?"
    set -e

    trap - EXIT
    deactivate_run "$run_id"

    return "$status"
  }

  resolve_workflow_mode() {
    local workflow_id="$1"
    local mode_override="$2"

    if [ -z "$mode_override" ]; then
      printf '%s' "$workflow_id"
      return
    fi

    if [[ "$workflow_id" == workflow.ci.* ]]; then
      local candidate="workflow.ci.$mode_override"
      if ${pkgs.jq}/bin/jq -e --arg workflowId "$candidate" '.workflows[$workflowId] != null' "$MODEL_FILE" >/dev/null; then
        printf '%s' "$candidate"
        return
      fi
    fi

    printf '%s' "$workflow_id"
  }

  resolve_effective_max_workers() {
    local workflow="$1"
    local workflow_max_workers
    local effective_workers
    local override_name=""
    local override_value=""

    workflow_max_workers="$(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -r '.maxWorkers // 1')"
    if ! [[ "$workflow_max_workers" =~ ^[0-9]+$ ]] || [ "$workflow_max_workers" -lt 1 ]; then
      workflow_max_workers=1
    fi
    effective_workers="$workflow_max_workers"

    if [ -n "''${NIXFIED_CI_MAX_WORKERS:-}" ]; then
      override_name="NIXFIED_CI_MAX_WORKERS"
      override_value="$NIXFIED_CI_MAX_WORKERS"
    elif [ -n "''${CI_MAX_WORKERS:-}" ]; then
      override_name="CI_MAX_WORKERS"
      override_value="$CI_MAX_WORKERS"
    fi

    if [ -n "$override_name" ]; then
      if [[ "$override_value" =~ ^[0-9]+$ ]] && [ "$override_value" -ge 1 ]; then
        if [ "$override_value" -lt "$effective_workers" ]; then
          effective_workers="$override_value"
        fi
      else
        echo "WARN: ignoring invalid $override_name='$override_value' (expected integer >= 1)"
      fi
    fi

    printf '%s' "$effective_workers"
  }

  run_workflow_serial_impl() {
    local run_id="$1"
    local workflow_id="$2"
    local workflow="$3"
    local fail_fast="$4"
    shift 4
    local -a passthrough_args
    passthrough_args=("$@")

    local status=0
    local unit_json

    while IFS= read -r unit_json; do
      local unit_task
      local skip=0
      local missing=""

      unit_task="$(printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '.taskId')"

      while IFS= read -r required_env; do
        if [ -n "$required_env" ] && [ -z "''${!required_env:-}" ]; then
          skip=1
          if [ -z "$missing" ]; then
            missing="$required_env"
          else
            missing="$missing,$required_env"
          fi
        fi
      done < <(printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '.skipIfMissingEnv[]?')

      if [ "$skip" -eq 1 ]; then
        local detail_json
        detail_json="$(${pkgs.jq}/bin/jq -cn --arg reason "missing-env" --arg missing "$missing" '{reason: $reason, missing: $missing}')"
        append_event "$run_id" "$workflow_id" "$unit_task" "canceled" "$detail_json"
        continue
      fi

      if execute_task "$run_id" "$workflow_id" "$unit_task" "''${passthrough_args[@]}"; then
        status=0
      else
        status="$?"
      fi

      if [ "$status" -ne 0 ] && [ "$fail_fast" = "true" ]; then
        break
      fi
    done < <(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -c '.plan[]')

    return "$status"
  }

  run_workflow_parallel_impl() {
    local run_id="$1"
    local workflow_id="$2"
    local workflow="$3"
    local fail_fast="$4"
    shift 4
    local -a passthrough_args
    passthrough_args=("$@")

    local max_workers
    local lock_policy
    local workflow_status=0
    local stop_scheduling=0
    local completed_count=0
    local running_count=0

    local -a unit_names
    unit_names=()

    local -A UNIT_JSON
    local -A UNIT_TASK
    local -A UNIT_NEEDS_LEFT
    local -A UNIT_STATE
    local -A UNIT_DEPENDENTS
    local -A UNIT_LOCKS
    local -A UNIT_PID
    local -A PID_UNIT
    local -A LOCK_OWNER
    local -A CANCEL_REQUESTED

    mark_unit_canceled() {
      local unit_name="$1"
      local reason="$2"
      local extra_key="''${3:-}"
      local extra_value="''${4:-}"
      local current_state
      local detail_json

      current_state="''${UNIT_STATE[$unit_name]:-pending}"
      if [ "$current_state" != "pending" ] && [ "$current_state" != "ready" ]; then
        return 0
      fi

      if [ -n "$extra_key" ]; then
        detail_json="$(${pkgs.jq}/bin/jq -cn --arg reason "$reason" --arg extraKey "$extra_key" --arg extraValue "$extra_value" '{reason: $reason} + {($extraKey): $extraValue}')"
      else
        detail_json="$(${pkgs.jq}/bin/jq -cn --arg reason "$reason" '{reason: $reason}')"
      fi

      append_event "$run_id" "$workflow_id" "''${UNIT_TASK[$unit_name]}" "canceled" "$detail_json"
      UNIT_STATE[$unit_name]="canceled"
      completed_count=$((completed_count + 1))
    }

    cancel_pending_dependents() {
      local source_unit="$1"
      local reason="$2"
      local -a queue
      local current
      local dependent
      queue=("$source_unit")

      while [ "''${#queue[@]}" -gt 0 ]; do
        current="''${queue[0]}"
        queue=("''${queue[@]:1}")
        for dependent in ''${UNIT_DEPENDENTS[$current]:-}; do
          local before_state
          before_state="''${UNIT_STATE[$dependent]:-pending}"
          mark_unit_canceled "$dependent" "$reason" "dependency" "$current"
          if [ "$before_state" = "pending" ] || [ "$before_state" = "ready" ]; then
            queue+=("$dependent")
          fi
        done
      done
    }

    cancel_pending_units() {
      local reason="$1"
      local unit_name
      for unit_name in "''${unit_names[@]}"; do
        mark_unit_canceled "$unit_name" "$reason"
      done
    }

    unit_has_lock_conflict() {
      local unit_name="$1"
      local lock
      local owner
      for lock in ''${UNIT_LOCKS[$unit_name]:-}; do
        owner="''${LOCK_OWNER[$lock]:-}"
        if [ -n "$owner" ] && [ "$owner" != "$unit_name" ]; then
          return 0
        fi
      done
      return 1
    }

    assign_unit_locks() {
      local unit_name="$1"
      local lock
      for lock in ''${UNIT_LOCKS[$unit_name]:-}; do
        LOCK_OWNER[$lock]="$unit_name"
      done
    }

    release_unit_locks() {
      local unit_name="$1"
      local lock
      for lock in ''${UNIT_LOCKS[$unit_name]:-}; do
        if [ "''${LOCK_OWNER[$lock]:-}" = "$unit_name" ]; then
          unset "LOCK_OWNER[$lock]"
        fi
      done
    }

    next_ready_unit() {
      local unit_name
      for unit_name in "''${unit_names[@]}"; do
        if [ "''${UNIT_STATE[$unit_name]:-pending}" != "ready" ]; then
          continue
        fi
        if unit_has_lock_conflict "$unit_name"; then
          continue
        fi
        printf '%s' "$unit_name"
        return 0
      done
      return 1
    }

    start_unit() {
      local unit_name="$1"
      local unit_task
      local detail_json
      local pid

      unit_task="''${UNIT_TASK[$unit_name]}"
      detail_json="$(${pkgs.jq}/bin/jq -cn --arg mode "task" '{mode: $mode}')"
      append_event "$run_id" "$workflow_id" "$unit_task" "queued" "$detail_json"
      append_event "$run_id" "$workflow_id" "$unit_task" "running" '{}'

      (
        execute_task_body "$unit_task" "''${passthrough_args[@]}"
      ) &
      pid="$!"

      UNIT_STATE[$unit_name]="running"
      UNIT_PID[$unit_name]="$pid"
      PID_UNIT[$pid]="$unit_name"
      CANCEL_REQUESTED[$unit_name]=0
      assign_unit_locks "$unit_name"
      running_count=$((running_count + 1))
    }

    cancel_running_units() {
      local pid
      local unit_name

      for pid in "''${!PID_UNIT[@]}"; do
        unit_name="''${PID_UNIT[$pid]:-}"
        if [ -z "$unit_name" ]; then
          continue
        fi
        CANCEL_REQUESTED[$unit_name]=1
        kill -TERM "$pid" 2>/dev/null || true
      done

      sleep 5
      for pid in "''${!PID_UNIT[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then
          kill -KILL "$pid" 2>/dev/null || true
        fi
      done
    }

    max_workers="$(resolve_effective_max_workers "$workflow")"
    lock_policy="$(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -r '.execution.lockPolicy // "exclusive"')"
    if [ "$lock_policy" = "shared-aware" ]; then
      echo "WARN: lockPolicy=shared-aware uses exclusive semantics in workflow parallel runner"
    fi

    while IFS= read -r unit_json; do
      local unit_name
      local unit_task
      local needs_count
      local lock_list

      unit_name="$(printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '.name')"
      unit_task="$(printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '.taskId')"
      needs_count="$(printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '(.needs // []) | length')"
      lock_list="$(printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '.locks[]?' | ${pkgs.gawk}/bin/awk 'NF {printf "%s ", $0}')"

      unit_names+=("$unit_name")
      UNIT_JSON[$unit_name]="$unit_json"
      UNIT_TASK[$unit_name]="$unit_task"
      UNIT_NEEDS_LEFT[$unit_name]="$needs_count"
      UNIT_STATE[$unit_name]="pending"
      UNIT_DEPENDENTS[$unit_name]=""
      UNIT_LOCKS[$unit_name]="$lock_list"
    done < <(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -c '.plan[]')

    local total_units="''${#unit_names[@]}"
    if [ "$total_units" -eq 0 ]; then
      return 0
    fi

    local unit_name
    for unit_name in "''${unit_names[@]}"; do
      while IFS= read -r dependency; do
        if [ -n "$dependency" ]; then
          UNIT_DEPENDENTS[$dependency]="''${UNIT_DEPENDENTS[$dependency]:-} $unit_name"
        fi
      done < <(printf '%s' "''${UNIT_JSON[$unit_name]}" | ${pkgs.jq}/bin/jq -r '.needs[]?')
    done

    for unit_name in "''${unit_names[@]}"; do
      local unit_json
      local missing=""
      local skip=0
      local when_failed=0
      local required_env
      local env_name
      local expected_value
      local actual_value

      unit_json="''${UNIT_JSON[$unit_name]}"

      while IFS= read -r required_env; do
        if [ -n "$required_env" ] && [ -z "''${!required_env:-}" ]; then
          skip=1
          if [ -z "$missing" ]; then
            missing="$required_env"
          else
            missing="$missing,$required_env"
          fi
        fi
      done < <(printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '.skipIfMissingEnv[]?')

      if [ "$skip" -eq 1 ]; then
        mark_unit_canceled "$unit_name" "missing-env" "missing" "$missing"
        cancel_pending_dependents "$unit_name" "dependency-not-passed"
        continue
      fi

      while IFS= read -r required_env; do
        if [ -n "$required_env" ] && [ -z "''${!required_env:-}" ]; then
          when_failed=1
          break
        fi
      done < <(printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '.when.envPresent[]?')

      if [ "$when_failed" -eq 0 ]; then
        while IFS=$'\t' read -r env_name expected_value; do
          if [ -z "$env_name" ]; then
            continue
          fi
          actual_value="''${!env_name:-}"
          if [ "$actual_value" != "$expected_value" ]; then
            when_failed=1
            break
          fi
        done < <(printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '.when.envEquals // {} | to_entries[]? | [.key, (.value | tostring)] | @tsv')
      fi

      if [ "$when_failed" -eq 1 ]; then
        mark_unit_canceled "$unit_name" "when-false"
        cancel_pending_dependents "$unit_name" "dependency-not-passed"
        continue
      fi

      if [ "''${UNIT_NEEDS_LEFT[$unit_name]}" -eq 0 ]; then
        UNIT_STATE[$unit_name]="ready"
      fi
    done

    while [ "$completed_count" -lt "$total_units" ]; do
      if [ "$stop_scheduling" -eq 0 ]; then
        while [ "$running_count" -lt "$max_workers" ]; do
          local ready_unit
          ready_unit="$(next_ready_unit || true)"
          if [ -z "$ready_unit" ]; then
            break
          fi
          start_unit "$ready_unit"
        done
      fi

      if [ "$running_count" -eq 0 ]; then
        if [ "$completed_count" -lt "$total_units" ] && [ "$stop_scheduling" -eq 0 ]; then
          cancel_pending_units "blocked"
          if [ "$workflow_status" -eq 0 ]; then
            workflow_status=1
          fi
        fi
        break
      fi

      local done_pid=""
      local wait_rc
      local done_unit
      local detail_json

      if wait -n -p done_pid; then
        wait_rc=0
      else
        wait_rc="$?"
      fi

      done_unit="''${PID_UNIT[$done_pid]:-}"
      if [ -z "$done_unit" ]; then
        continue
      fi

      unset "PID_UNIT[$done_pid]"
      unset "UNIT_PID[$done_unit]"
      running_count=$((running_count - 1))
      release_unit_locks "$done_unit"

      if [ "''${CANCEL_REQUESTED[$done_unit]:-0}" = "1" ]; then
        detail_json="$(${pkgs.jq}/bin/jq -cn --arg reason "fail-fast-running" '{reason: $reason}')"
        append_event "$run_id" "$workflow_id" "''${UNIT_TASK[$done_unit]}" "canceled" "$detail_json"
        UNIT_STATE[$done_unit]="canceled"
        completed_count=$((completed_count + 1))
        continue
      fi

      if [ "$wait_rc" -eq 0 ]; then
        append_event "$run_id" "$workflow_id" "''${UNIT_TASK[$done_unit]}" "passed" '{}'
        UNIT_STATE[$done_unit]="passed"
        completed_count=$((completed_count + 1))

        local dependent
        for dependent in ''${UNIT_DEPENDENTS[$done_unit]:-}; do
          if [ "''${UNIT_STATE[$dependent]:-pending}" = "pending" ]; then
            UNIT_NEEDS_LEFT[$dependent]="$(( ''${UNIT_NEEDS_LEFT[$dependent]} - 1 ))"
            if [ "''${UNIT_NEEDS_LEFT[$dependent]}" -eq 0 ]; then
              UNIT_STATE[$dependent]="ready"
            fi
          fi
        done
      else
        detail_json="$(${pkgs.jq}/bin/jq -cn --argjson exitCode "$wait_rc" '{exitCode: $exitCode}')"
        append_event "$run_id" "$workflow_id" "''${UNIT_TASK[$done_unit]}" "failed" "$detail_json"
        UNIT_STATE[$done_unit]="failed"
        completed_count=$((completed_count + 1))

        if [ "$workflow_status" -eq 0 ]; then
          workflow_status="$wait_rc"
        fi

        if [ "$fail_fast" = "true" ] && [ "$stop_scheduling" -eq 0 ]; then
          stop_scheduling=1
          cancel_running_units
          cancel_pending_units "fail-fast"
        else
          cancel_pending_dependents "$done_unit" "dependency-not-passed"
        fi
      fi
    done

    return "$workflow_status"
  }

  run_workflow() {
    if [ "$#" -lt 1 ]; then
      echo "ERROR: usage: run-workflow <workflow-id> [-- ...]"
      return 2
    fi

    local workflow_id="$1"
    shift

    local mode_override=""
    local print_summary=0
    local -a passthrough_args
    passthrough_args=()

    while [ "$#" -gt 0 ]; do
      case "$1" in
        --mode)
          if [ "$#" -lt 2 ]; then
            echo "ERROR: --mode requires a value"
            return 2
          fi
          mode_override="$2"
          shift 2
          ;;
        --basic|--app|--env|--full)
          mode_override="''${1#--}"
          shift
          ;;
        --summary)
          print_summary=1
          shift
          ;;
        --)
          shift
          while [ "$#" -gt 0 ]; do
            passthrough_args+=("$1")
            shift
          done
          ;;
        *)
          passthrough_args+=("$1")
          shift
          ;;
      esac
    done

    workflow_id="$(resolve_workflow_mode "$workflow_id" "$mode_override")"

    local workflow
    local args_payload
    local run_id
    local detail_json
    local fail_fast
    local status=0

    workflow="$(workflow_json "$workflow_id")"
    if [ -z "$workflow" ]; then
      echo "ERROR: unknown workflow '$workflow_id'"
      return 2
    fi

    args_payload="$(printf '%s\n' "''${passthrough_args[@]}")"
    run_id="$(compute_run_id "workflow" "$workflow_id" "" "$args_payload")"

    activate_run "$run_id"
    trap "deactivate_run '$run_id'" EXIT

    detail_json="$(${pkgs.jq}/bin/jq -cn --arg suffix "$RUN_SUFFIX_REASON" --arg mode "workflow" '{mode: $mode, suffixReason: (if $suffix == "" then null else $suffix end)}')"
    append_event "$run_id" "$workflow_id" "" "queued" "$detail_json"

    fail_fast="$(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -r '.execution.failFast')"

    if [ "''${NIXFIED_WORKFLOW_PARALLEL:-0}" = "1" ]; then
      if run_workflow_parallel_impl "$run_id" "$workflow_id" "$workflow" "$fail_fast" "''${passthrough_args[@]}"; then
        status=0
      else
        status="$?"
      fi
    else
      if run_workflow_serial_impl "$run_id" "$workflow_id" "$workflow" "$fail_fast" "''${passthrough_args[@]}"; then
        status=0
      else
        status="$?"
      fi
    fi

    if [ "$status" -eq 0 ]; then
      append_event "$run_id" "$workflow_id" "" "passed" '{}'
    else
      detail_json="$(${pkgs.jq}/bin/jq -cn --argjson exitCode "$status" '{exitCode: $exitCode}')"
      append_event "$run_id" "$workflow_id" "" "failed" "$detail_json"
    fi

    if [ "$print_summary" -eq 1 ]; then
      local events_file="$REGISTRY_ROOT/events.ndjson"
      local passed failed canceled
      if [ -f "$events_file" ]; then
        passed="$(${pkgs.jq}/bin/jq -r --arg runId "$run_id" 'select(.runId == $runId) | .state' "$events_file" | ${pkgs.gnugrep}/bin/grep -c '^passed$' || true)"
        failed="$(${pkgs.jq}/bin/jq -r --arg runId "$run_id" 'select(.runId == $runId) | .state' "$events_file" | ${pkgs.gnugrep}/bin/grep -c '^failed$' || true)"
        canceled="$(${pkgs.jq}/bin/jq -r --arg runId "$run_id" 'select(.runId == $runId) | .state' "$events_file" | ${pkgs.gnugrep}/bin/grep -c '^canceled$' || true)"
      else
        passed=0
        failed=0
        canceled=0
      fi
      echo "INFO: runId=$run_id passed=$passed failed=$failed canceled=$canceled"
    fi

    trap - EXIT
    deactivate_run "$run_id"
    return "$status"
  }

  main() {
    if [ "$#" -lt 1 ]; then
      echo "ERROR: usage: nixfied-executor <run-task|run-workflow> ..."
      exit 2
    fi

    local subcommand="$1"
    shift

    case "$subcommand" in
      run-task)
        run_task "$@"
        ;;
      run-workflow)
        run_workflow "$@"
        ;;
      *)
        echo "ERROR: unknown subcommand '$subcommand'"
        exit 2
        ;;
    esac
  }

  main "$@"
''
