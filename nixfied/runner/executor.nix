{
  pkgs,
  model,
  registry,
  projectRoot,
}:
let
  modelFile = pkgs.writeText "nixfied-model.json" (builtins.toJSON model);
  registryShell = registry.events.mkShellLib { };
  workflowModesShell = import ./workflow-modes.nix {
    inherit pkgs;
  };
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
  ${workflowModesShell}
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
  LAST_WORKFLOW_SUMMARY_FILE=""

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

  task_has_hooks() {
    local task="$1"
    local hook_count

    hook_count="$(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r '((.runtime.preHooks // {}) | length) + ((.runtime.postHooks // {}) | length)')"
    if [ "$hook_count" -gt 0 ]; then
      return 0
    fi
    return 1
  }

  run_task_hooks() {
    local task_id="$1"
    local task="$2"
    local phase="$3"
    shift 3

    local hook_id
    local hook_command
    local hook_runtime_json
    local hook_exit_code

    while IFS= read -r hook_id; do
      if [ -z "$hook_id" ]; then
        continue
      fi

      echo "INFO: hook $phase $hook_id start"
      hook_command="$(
        printf '%s' "$task" | ${pkgs.jq}/bin/jq -r --arg phase "$phase" --arg hookId "$hook_id" '.runtime[($phase + "Hooks")][$hookId].command'
      )"
      hook_runtime_json="$(
        printf '%s' "$task" | ${pkgs.jq}/bin/jq -c --arg phase "$phase" --arg hookId "$hook_id" '
          .runtime as $taskRuntime
          | .runtime[($phase + "Hooks")][$hookId] as $hook
          | {
              slotEnv: $taskRuntime.slotEnv,
              workdir: (if ($hook.workdir // null) == null then $taskRuntime.workdir else $hook.workdir end),
              customWorkdir:
                (if ($hook.customWorkdir // null) != null then $hook.customWorkdir
                 elif ($hook.workdir // null) == null then ($taskRuntime.customWorkdir // null)
                 elif $hook.workdir == "custom" then ($taskRuntime.customWorkdir // null)
                 else null end),
              hermetic: $taskRuntime.hermetic,
              runtimeInputs: (($taskRuntime.runtimeInputs // []) + ($hook.runtimeInputs // [])),
              passThroughEnv: (($taskRuntime.passThroughEnv // []) + ($hook.passThroughEnv // [])),
              env: (($taskRuntime.env // {}) + ($hook.env // {})),
              umask: ($taskRuntime.umask // "022"),
              locale: ($taskRuntime.locale // "C.UTF-8"),
              timezone: ($taskRuntime.timezone // "UTC")
            }
        '
      )"

      run_in_sandbox_runtime "$hook_runtime_json" "$hook_command" "$@"
      hook_exit_code="$?"
      if [ "$hook_exit_code" -ne 0 ]; then
        echo "ERROR: hook $phase $hook_id failed exitCode=$hook_exit_code"
        return "$hook_exit_code"
      fi

      echo "OK: hook $phase $hook_id done"
    done < <(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r --arg phase "$phase" '.runtime[($phase + "Hooks")] // {} | keys[]')

    return 0
  }

  execute_task_body() {
    local task_id="$1"
    shift

    local task
    local runner_type
    local command
    local nested_workflow
    local exit_code
    local main_exit_code
    local post_exit_code

    task="$(task_json "$task_id")"
    if [ -z "$task" ]; then
      echo "ERROR: unknown task '$task_id'"
      return 2
    fi

    runner_type="$(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r '.runner.type')"
    if [ "$runner_type" != "shell" ] && task_has_hooks "$task"; then
      echo "ERROR: task '$task_id' defines runtime hooks but runner type '$runner_type' is unsupported"
      return 3
    fi

    set +e
    case "$runner_type" in
      shell)
        if run_task_hooks "$task_id" "$task" "pre" "$@"; then
          command="$(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r '.runner.command')"
          run_in_sandbox "$task" "$command" "$@"
          main_exit_code="$?"

          if run_task_hooks "$task_id" "$task" "post" "$@"; then
            post_exit_code=0
          else
            post_exit_code="$?"
          fi

          if [ "$post_exit_code" -ne 0 ]; then
            if [ "$main_exit_code" -ne 0 ]; then
              echo "ERROR: task '$task_id' main exitCode=$main_exit_code and post hook failed exitCode=$post_exit_code"
            fi
            exit_code="$post_exit_code"
          else
            exit_code="$main_exit_code"
          fi
        else
          exit_code="$?"
        fi
        ;;
      workflowRef)
        nested_workflow="$(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r '.runner.workflowId // empty')"
        if [ -z "$nested_workflow" ]; then
          echo "ERROR: task '$task_id' runner.workflowId is empty"
          exit_code=3
        else
          if [ -n "''${NIXFIED_PARENT_WORKFLOW_ID:-}" ]; then
            NIXFIED_WORKFLOW_NESTED=1 run_workflow "$nested_workflow" "$@"
          else
            run_workflow "$nested_workflow" "$@"
          fi
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

    if [ -n "$workflow_id" ]; then
      if NIXFIED_PARENT_WORKFLOW_ID="$workflow_id" execute_task_body "$task_id" "$@"; then
        exit_code=0
      else
        exit_code="$?"
      fi
    elif execute_task_body "$task_id" "$@"; then
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
    local managed_by_orchestrator=0

    if [ -n "''${NIXFIED_ORCHESTRATOR_RUN_ID:-}" ]; then
      run_id="$NIXFIED_ORCHESTRATOR_RUN_ID"
      RUN_SUFFIX_REASON="orchestrator"
      managed_by_orchestrator=1
    else
      args_payload="$(printf '%s\n' "$@")"
      run_id="$(compute_run_id "task" "" "$task_id" "$args_payload")"
      activate_run "$run_id"
      trap "deactivate_run '$run_id'" EXIT
    fi

    detail_json="$(${pkgs.jq}/bin/jq -cn --arg suffix "$RUN_SUFFIX_REASON" '{mode: "task", suffixReason: (if $suffix == "" then null else $suffix end)}')"
    append_event "$run_id" "" "$task_id" "queued" "$detail_json"

    set +e
    execute_task "$run_id" "" "$task_id" "$@"
    status="$?"
    set -e

    if [ "$managed_by_orchestrator" -eq 0 ]; then
      trap - EXIT
      deactivate_run "$run_id"
    fi

    return "$status"
  }

  resolve_workflow_mode() {
    local workflow_id="$1"
    local mode_override="$2"

    workflow_resolve_mode_id "$workflow_id" "$mode_override"
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
        echo "ERROR: $override_name must be an integer >= 1 (got '$override_value')" >&2
        return 2
      fi
    fi

    printf '%s' "$effective_workers"
  }

  resolve_parallel_mode() {
    local workflow="$1"
    local configured_parallel
    local env_override
    local run_parallel=0

    configured_parallel="$(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -r '.execution.parallel // false')"
    if [ "$configured_parallel" = "true" ]; then
      run_parallel=1
    fi

    env_override="''${NIXFIED_WORKFLOW_PARALLEL:-}"
    if [ -n "$env_override" ]; then
      if [ "$env_override" = "1" ]; then
        run_parallel=1
      elif [ "$env_override" = "0" ]; then
        run_parallel=0
      else
        echo "WARN: ignoring invalid NIXFIED_WORKFLOW_PARALLEL='$env_override' (expected 0 or 1)"
      fi
    fi

    printf '%s' "$run_parallel"
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

    max_workers="$(resolve_effective_max_workers "$workflow")" || return $?
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

  run_workflow_phase_tasks() {
    local run_id="$1"
    local workflow_id="$2"
    local workflow="$3"
    local phase_key="$4"
    shift 4
    local -a passthrough_args
    passthrough_args=("$@")

    local phase_task
    local phase_status=0

    while IFS= read -r phase_task; do
      if [ -z "$phase_task" ]; then
        continue
      fi

      if execute_task "$run_id" "$workflow_id" "$phase_task" "''${passthrough_args[@]}"; then
        phase_status=0
      else
        phase_status="$?"
        break
      fi
    done < <(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -r --arg phase "$phase_key" '.[$phase].tasks[]?')

    return "$phase_status"
  }

  is_nonneg_int() {
    case "''${1:-}" in
      ""|*[!0-9]*)
        return 1
        ;;
      *)
        return 0
        ;;
    esac
  }

  format_duration_seconds() {
    local seconds="$1"
    if ! is_nonneg_int "$seconds"; then
      printf '%s' "?"
      return 0
    fi

    if [ "$seconds" -lt 60 ]; then
      printf '%s' "''${seconds}s"
      return 0
    fi

    local mins
    local secs
    mins="$(( seconds / 60 ))"
    secs="$(( seconds % 60 ))"
    printf '%s' "''${mins}m ''${secs}s"
  }

  task_runner_type() {
    local task_id="$1"
    if [ -z "$task_id" ]; then
      printf '%s' "shell"
      return 0
    fi

    ${pkgs.jq}/bin/jq -r --arg taskId "$task_id" '.tasks[$taskId].runner.type // "shell"' "$MODEL_FILE"
  }

  workflow_step_records_tsv() {
    local run_id="$1"
    local events_file="$2"

    if [ ! -f "$events_file" ]; then
      return 0
    fi

    ${pkgs.jq}/bin/jq -r -s --arg runId "$run_id" '
      map(
        select(
          .runId == $runId
          and (.taskId // "") != ""
          and (
            .state == "queued"
            or .state == "running"
            or .state == "passed"
            or .state == "failed"
            or .state == "canceled"
          )
        )
      )
      | sort_by(.seq)
      | reduce .[] as $event (
          { active: {}, rows: [] };
          (((($event.workflowId // "") + "\u001f" + $event.taskId)) as $key
          | if ($event.state == "queued" or $event.state == "running") then
              .active[$key] = (
                (.active[$key] // {
                  task_id: $event.taskId,
                  workflow_id: ($event.workflowId // ""),
                  order_seq: $event.seq,
                  running_ts: null
                })
                | .order_seq = (if .order_seq > $event.seq then $event.seq else .order_seq end)
                | if $event.state == "running" then .running_ts = $event.ts else . end
              )
            elif ($event.state == "passed" or $event.state == "failed" or $event.state == "canceled") then
              (.active[$key] // {
                task_id: $event.taskId,
                workflow_id: ($event.workflowId // ""),
                order_seq: $event.seq,
                running_ts: null
              }) as $entry
              | .rows += [
                  {
                    task_id: $entry.task_id,
                    workflow_id: (if $entry.workflow_id == "" then ($event.workflowId // "") else $entry.workflow_id end),
                    order_seq: $entry.order_seq,
                    state: $event.state,
                    reason: ($event.detail.reason // ""),
                    exit_code: ($event.detail.exitCode // ""),
                    duration_seconds: (
                      if ($entry.running_ts != null)
                        and ($entry.running_ts | type == "string")
                        and ($event.ts | type == "string")
                      then
                        (((($event.ts | fromdateiso8601) - ($entry.running_ts | fromdateiso8601)) | floor) | if . < 0 then 0 else . end)
                      else
                        0
                      end
                    )
                  }
                ]
              | del(.active[$key])
            else
              .
            end)
        )
      | .rows
      | sort_by(.order_seq)
      | .[]
      | [
          .task_id,
          .workflow_id,
          (.order_seq | tostring),
          .state,
          (.duration_seconds | tostring),
          (.reason | tostring),
          (if .exit_code == null or .exit_code == "" then "" else (.exit_code | tostring) end)
        ]
      | @tsv
    ' "$events_file"
  }

  workflow_steps_json() {
    local run_id="$1"
    local events_file="$2"
    local steps_json="[]"
    local task_id
    local workflow_id
    local order_seq
    local state
    local duration
    local reason
    local exit_code
    local runner_type
    local status
    local duration_json
    local order_seq_json
    local exit_code_json

    if [ ! -f "$events_file" ]; then
      printf '%s' "$steps_json"
      return 0
    fi

    while IFS=$'\t' read -r task_id workflow_id order_seq state duration reason exit_code; do
      if [ -z "$task_id" ]; then
        continue
      fi

      runner_type="$(task_runner_type "$task_id")"
      if [ "$runner_type" = "workflowRef" ]; then
        continue
      fi

      status="$state"
      if [ "$state" = "canceled" ]; then
        case "$reason" in
          missing-env|when-false)
            status="skipped"
            ;;
          *)
            status="canceled"
            ;;
        esac
      fi

      if is_nonneg_int "$duration"; then
        duration_json="$duration"
      else
        duration_json=0
      fi

      if is_nonneg_int "$order_seq"; then
        order_seq_json="$order_seq"
      else
        order_seq_json=0
      fi

      if [ -n "$exit_code" ] && [[ "$exit_code" =~ ^-?[0-9]+$ ]]; then
        exit_code_json="$exit_code"
      else
        exit_code_json="null"
      fi

      steps_json="$(
        ${pkgs.jq}/bin/jq -cn \
          --argjson steps "$steps_json" \
          --arg name "$task_id" \
          --arg status "$status" \
          --arg state "$state" \
          --arg workflowId "$workflow_id" \
          --arg reason "$reason" \
          --argjson duration "$duration_json" \
          --argjson orderSeq "$order_seq_json" \
          --argjson exitCode "$exit_code_json" \
          '$steps + [{
            name: $name,
            status: $status,
            state: $state,
            duration: $duration,
            order: $orderSeq,
            workflow_id: (if $workflowId == "" then null else $workflowId end),
            reason: (if $reason == "" then null else $reason end),
            exit_code: $exitCode
          }]'
      )"
    done < <(workflow_step_records_tsv "$run_id" "$events_file")

    printf '%s' "$steps_json" | ${pkgs.jq}/bin/jq -c 'sort_by(.order)'
  }

  workflow_peak_workers() {
    local run_id="$1"
    local events_file="$2"
    local leaf_task_ids_json="$3"

    if [ ! -f "$events_file" ]; then
      printf '%s' "0"
      return 0
    fi

    ${pkgs.jq}/bin/jq -r -s --arg runId "$run_id" --argjson taskIds "$leaf_task_ids_json" '
      map(
        select(
          .runId == $runId
          and (.taskId // "") != ""
          and ((.taskId as $id | ($taskIds | index($id)) != null))
          and (
            .state == "running"
            or .state == "passed"
            or .state == "failed"
            or .state == "canceled"
          )
        )
      )
      | sort_by(.seq)
      | reduce .[] as $event (
          { running: 0, max: 0 };
          if $event.state == "running" then
            .running += 1
            | .max = (if .running > .max then .running else .max end)
          else
            .running = (if .running > 0 then .running - 1 else 0 end)
          end
        )
      | .max
    ' "$events_file"
  }

  print_workflow_summary_report() {
    local run_id="$1"
    local workflow_id="$2"
    local exit_code="$3"
    local duration_seconds="$4"
    local summary_file="$5"
    local events_file="$REGISTRY_ROOT/events.ndjson"
    local steps_json="[]"
    local timing_fields=""
    local summary_duration=""
    local timing_setup=""
    local timing_steps=""
    local timing_teardown=""
    local timing_accounted=""
    local timing_untracked=""
    local parallel_max_workers=""
    local parallel_peak_workers=""
    local parallel_canceled_count=""

    echo ""
    echo "------------------------------------------------------------"
    echo "Summary"
    echo "------------------------------------------------------------"

    if [ -n "$summary_file" ] && [ -f "$summary_file" ]; then
      echo "Source: $summary_file"
      steps_json="$(${pkgs.jq}/bin/jq -c '.steps // []' "$summary_file" 2>/dev/null || echo "[]")"
      summary_duration="$(${pkgs.jq}/bin/jq -r '.timing.total_duration // .duration_seconds // ""' "$summary_file" 2>/dev/null || true)"
      if is_nonneg_int "$summary_duration"; then
        duration_seconds="$summary_duration"
      fi
      timing_fields="$(
        ${pkgs.jq}/bin/jq -r '
          [
            (.timing.setup_duration // ""),
            (.timing.steps_duration // ""),
            (.timing.teardown_duration // ""),
            (.timing.accounted_duration // ""),
            (.timing.untracked_duration // ""),
            (.timing.parallelism.max_workers // ""),
            (.timing.parallelism.peak_workers // ""),
            (.timing.parallelism.canceled_count // "")
          ] | @tsv
        ' "$summary_file" 2>/dev/null || true
      )"
      if [ -n "$timing_fields" ]; then
        IFS=$'\t' read -r timing_setup timing_steps timing_teardown timing_accounted timing_untracked parallel_max_workers parallel_peak_workers parallel_canceled_count <<< "$timing_fields"
      fi
    elif [ -f "$events_file" ]; then
      steps_json="$(workflow_steps_json "$run_id" "$events_file" 2>/dev/null || echo "[]")"
    fi

    printf '%s' "$steps_json" | ${pkgs.jq}/bin/jq -r '
      .[] |
      "  [\(
        if .status == "passed" then
          "PASS"
        elif .status == "skipped" then
          "SKIP"
        elif .status == "failed" then
          "FAIL"
        elif .status == "canceled" then
          "FAIL"
        else
          "FAIL"
        end
      )] \(.name) (\((.duration // "?") | tostring)s)"
    ' 2>/dev/null || true

    if is_nonneg_int "$duration_seconds"; then
      echo "Total time: $(format_duration_seconds "$duration_seconds")"
    fi

    if is_nonneg_int "$timing_setup" \
      && is_nonneg_int "$timing_steps" \
      && is_nonneg_int "$timing_teardown" \
      && is_nonneg_int "$timing_accounted" \
      && is_nonneg_int "$timing_untracked"; then
      echo "INFO: Time breakdown setup=''${timing_setup}s steps=''${timing_steps}s teardown=''${timing_teardown}s accounted=''${timing_accounted}s untracked=''${timing_untracked}s"
    fi

    if is_nonneg_int "$parallel_max_workers" \
      && is_nonneg_int "$parallel_peak_workers" \
      && is_nonneg_int "$parallel_canceled_count"; then
      echo "INFO: Parallelism max_workers=$parallel_max_workers peak_workers=$parallel_peak_workers canceled_count=$parallel_canceled_count"
    fi

    if [ "$exit_code" -eq 0 ]; then
      echo "OK: Exit code: 0"
    else
      echo "ERROR: Exit code: $exit_code"
    fi

    if [ -n "$summary_file" ] && [ -f "$summary_file" ]; then
      echo ""
      echo "Artifacts: $(dirname "$summary_file")"
    fi

    echo "------------------------------------------------------------"
  }

  write_workflow_summary_json() {
    local run_id="$1"
    local workflow_id="$2"
    local workflow="$3"
    local exit_code="$4"
    local started_at="$5"
    local started_epoch="$6"

    local should_write
    local mode
    local artifacts_dir
    local summary_file
    local finished_at
    local duration_seconds
    local passed
    local failed
    local canceled
    local steps_json
    local steps_duration
    local setup_duration=0
    local teardown_duration=0
    local accounted_duration
    local untracked_duration
    local parallel_max_workers
    local parallel_peak_workers
    local parallel_canceled_count
    local parallel_max_workers_json="null"
    local parallel_peak_workers_json="null"
    local parallel_canceled_count_json="null"
    local leaf_task_ids_json="[]"
    local events_file="$REGISTRY_ROOT/events.ndjson"

    LAST_WORKFLOW_SUMMARY_FILE=""

    should_write="$(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -r '.artifacts.writeSummary // false')"
    if [ "$should_write" != "true" ]; then
      return 0
    fi

    mode="$(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -r '.mode // "custom"')"
    artifacts_dir="''${CI_ARTIFACTS_DIR:-$(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -r '.artifacts.root // "/tmp/ci-artifacts"')}"
    summary_file="$artifacts_dir/summary.json"

    if ! mkdir -p "$artifacts_dir"; then
      if [ -z "''${CI_ARTIFACTS_DIR:-}" ]; then
        artifacts_dir="$REGISTRY_ROOT/artifacts/$run_id"
        summary_file="$artifacts_dir/summary.json"
        if ! mkdir -p "$artifacts_dir"; then
          echo "ERROR: failed to create artifacts directory '$artifacts_dir'"
          return 1
        fi
        echo "WARN: artifacts root was not writable; using fallback '$artifacts_dir'"
      else
        echo "ERROR: failed to create artifacts directory '$artifacts_dir'"
        return 1
      fi
    fi

    finished_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
    duration_seconds="$(( $(date +%s) - started_epoch ))"
    if [ "$duration_seconds" -lt 0 ]; then
      duration_seconds=0
    fi

    if [ -f "$events_file" ]; then
      if steps_json="$(workflow_steps_json "$run_id" "$events_file")"; then
        :
      else
        echo "WARN: failed to collect step summary from '$events_file'; using empty step list"
        steps_json="[]"
      fi
    else
      steps_json="[]"
    fi

    passed="$(printf '%s' "$steps_json" | ${pkgs.jq}/bin/jq -r '[.[] | select(.state == "passed")] | length' 2>/dev/null || echo 0)"
    failed="$(printf '%s' "$steps_json" | ${pkgs.jq}/bin/jq -r '[.[] | select(.state == "failed")] | length' 2>/dev/null || echo 0)"
    canceled="$(printf '%s' "$steps_json" | ${pkgs.jq}/bin/jq -r '[.[] | select(.state == "canceled")] | length' 2>/dev/null || echo 0)"

    steps_duration="$(printf '%s' "$steps_json" | ${pkgs.jq}/bin/jq -r '[.[] | (.duration // 0)] | add // 0' 2>/dev/null || echo 0)"
    if ! is_nonneg_int "$steps_duration"; then
      steps_duration=0
    fi
    accounted_duration="$(( setup_duration + steps_duration + teardown_duration ))"
    untracked_duration="$(( duration_seconds - accounted_duration ))"
    if [ "$untracked_duration" -lt 0 ]; then
      untracked_duration=0
    fi

    if parallel_max_workers="$(resolve_effective_max_workers "$workflow" 2>/dev/null || true)"; then
      :
    fi
    if is_nonneg_int "$parallel_max_workers"; then
      parallel_max_workers_json="$parallel_max_workers"
    fi

    leaf_task_ids_json="$(printf '%s' "$steps_json" | ${pkgs.jq}/bin/jq -c '[.[] | .name] | unique' 2>/dev/null || echo '[]')"
    if [ -f "$events_file" ]; then
      parallel_peak_workers="$(workflow_peak_workers "$run_id" "$events_file" "$leaf_task_ids_json" 2>/dev/null || echo 0)"
    else
      parallel_peak_workers=0
    fi
    if is_nonneg_int "$parallel_peak_workers"; then
      parallel_peak_workers_json="$parallel_peak_workers"
    fi

    parallel_canceled_count="$canceled"
    if is_nonneg_int "$parallel_canceled_count"; then
      parallel_canceled_count_json="$parallel_canceled_count"
    fi

    write_summary_payload() {
      local target_file="$1"
      ${pkgs.jq}/bin/jq -n -S \
        --arg runId "$run_id" \
        --arg workflowId "$workflow_id" \
        --arg mode "$mode" \
        --argjson exitCode "$exit_code" \
        --arg startedAt "$started_at" \
        --arg finishedAt "$finished_at" \
        --argjson durationSeconds "$duration_seconds" \
        --argjson passed "$passed" \
        --argjson failed "$failed" \
        --argjson canceled "$canceled" \
        --argjson steps "$steps_json" \
        --argjson setupDuration "$setup_duration" \
        --argjson stepsDuration "$steps_duration" \
        --argjson teardownDuration "$teardown_duration" \
        --argjson accountedDuration "$accounted_duration" \
        --argjson untrackedDuration "$untracked_duration" \
        --argjson parallelMaxWorkers "$parallel_max_workers_json" \
        --argjson parallelPeakWorkers "$parallel_peak_workers_json" \
        --argjson parallelCanceledCount "$parallel_canceled_count_json" \
        '{
          run_id: $runId,
          workflow_id: $workflowId,
          mode: $mode,
          exit_code: $exitCode,
          started_at: $startedAt,
          finished_at: $finishedAt,
          duration_seconds: $durationSeconds,
          counts: {
            passed: $passed,
            failed: $failed,
            canceled: $canceled
          },
          steps: $steps,
          timing: {
            total_duration: $durationSeconds,
            setup_duration: $setupDuration,
            steps_duration: $stepsDuration,
            teardown_duration: $teardownDuration,
            accounted_duration: $accountedDuration,
            untracked_duration: $untrackedDuration,
            parallelism: {
              max_workers: $parallelMaxWorkers,
              peak_workers: $parallelPeakWorkers,
              canceled_count: $parallelCanceledCount
            }
          }
        }' > "$target_file"
    }

    if ! write_summary_payload "$summary_file"; then
      if [ -z "''${CI_ARTIFACTS_DIR:-}" ]; then
        artifacts_dir="$REGISTRY_ROOT/artifacts/$run_id"
        summary_file="$artifacts_dir/summary.json"
        if ! mkdir -p "$artifacts_dir"; then
          echo "ERROR: failed to create fallback artifacts directory '$artifacts_dir'"
          return 1
        fi
        if ! write_summary_payload "$summary_file"; then
          echo "ERROR: failed to write summary file '$summary_file'"
          return 1
        fi
        echo "WARN: artifacts root was not writable; using fallback '$artifacts_dir'"
      else
        echo "ERROR: failed to write summary file '$summary_file'"
        return 1
      fi
    fi

    LAST_WORKFLOW_SUMMARY_FILE="$summary_file"
    echo "INFO: summary_json=$summary_file"
    return 0
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
    local parse_options=1
    local -a passthrough_args
    passthrough_args=()
    local arg
    local shorthand_mode

    while [ "$#" -gt 0 ]; do
      arg="$1"
      shift

      if [ "$parse_options" -eq 0 ]; then
        passthrough_args+=("$arg")
        continue
      fi

      case "$arg" in
        --mode)
          if [ "$#" -lt 1 ]; then
            echo "ERROR: --mode requires a value"
            return 2
          fi
          mode_override="$1"
          shift
          ;;
        --mode=*)
          mode_override="''${arg#--mode=}"
          ;;
        --summary)
          print_summary=1
          ;;
        --)
          parse_options=0
          ;;
        --*)
          shorthand_mode="''${arg#--}"
          if workflow_simple_shorthand_exists_for_family "$workflow_id" "$shorthand_mode"; then
            mode_override="$shorthand_mode"
          else
            echo "ERROR: unknown option '$arg'"
            return 2
          fi
          ;;
        -*)
          echo "ERROR: unknown option '$arg'"
          return 2
          ;;
        *)
          passthrough_args+=("$arg")
          ;;
      esac
    done

    workflow_id="$(resolve_workflow_mode "$workflow_id" "$mode_override")" || return $?

    local workflow
    local args_payload
    local run_id
    local detail_json
    local fail_fast
    local run_parallel
    local status=0
    local post_status=0
    local post_always=true
    local started_at
    local started_epoch
    local duration_seconds
    local summary_file=""
    local nested_workflow_call=0
    local managed_by_orchestrator=0

    if [ "''${NIXFIED_WORKFLOW_NESTED:-0}" = "1" ]; then
      nested_workflow_call=1
    fi

    workflow="$(workflow_json "$workflow_id")"
    if [ -z "$workflow" ]; then
      echo "ERROR: unknown workflow '$workflow_id'"
      return 2
    fi

    started_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
    started_epoch="$(date +%s)"

    if [ -n "''${NIXFIED_ORCHESTRATOR_RUN_ID:-}" ]; then
      run_id="$NIXFIED_ORCHESTRATOR_RUN_ID"
      RUN_SUFFIX_REASON="orchestrator"
      managed_by_orchestrator=1
    else
      args_payload="$(printf '%s\n' "''${passthrough_args[@]}")"
      run_id="$(compute_run_id "workflow" "$workflow_id" "" "$args_payload")"
      activate_run "$run_id"
      trap "deactivate_run '$run_id'" EXIT
    fi

    detail_json="$(${pkgs.jq}/bin/jq -cn --arg suffix "$RUN_SUFFIX_REASON" --arg mode "workflow" '{mode: $mode, suffixReason: (if $suffix == "" then null else $suffix end)}')"
    append_event "$run_id" "$workflow_id" "" "queued" "$detail_json"

    fail_fast="$(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -r '.execution.failFast')"
    run_parallel="$(resolve_parallel_mode "$workflow")"

    if run_workflow_phase_tasks "$run_id" "$workflow_id" "$workflow" "preRun" "''${passthrough_args[@]}"; then
      status=0
    else
      status="$?"
    fi

    if [ "$status" -eq 0 ]; then
      if [ "$run_parallel" = "1" ]; then
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
    fi

    post_always="$(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -r 'if .postRun.alwaysRun == null then true else .postRun.alwaysRun end')"
    if [ "$post_always" = "true" ] || [ "$status" -eq 0 ]; then
      if run_workflow_phase_tasks "$run_id" "$workflow_id" "$workflow" "postRun" "''${passthrough_args[@]}"; then
        post_status=0
      else
        post_status="$?"
      fi

      if [ "$post_status" -ne 0 ] && [ "$status" -eq 0 ]; then
        status="$post_status"
      fi
    fi

    if [ "$status" -eq 0 ]; then
      append_event "$run_id" "$workflow_id" "" "passed" '{}'
    else
      detail_json="$(${pkgs.jq}/bin/jq -cn --argjson exitCode "$status" '{exitCode: $exitCode}')"
      append_event "$run_id" "$workflow_id" "" "failed" "$detail_json"
    fi

    duration_seconds="$(( $(date +%s) - started_epoch ))"
    if [ "$duration_seconds" -lt 0 ]; then
      duration_seconds=0
    fi

    if [ "$nested_workflow_call" -eq 0 ]; then
      if ! write_workflow_summary_json "$run_id" "$workflow_id" "$workflow" "$status" "$started_at" "$started_epoch"; then
        if [ "$status" -eq 0 ]; then
          status=1
          detail_json="$(${pkgs.jq}/bin/jq -cn --arg reason "summary-write-failed" '{reason: $reason, exitCode: 1}')"
          append_event "$run_id" "$workflow_id" "" "failed" "$detail_json"
        fi
      fi
      summary_file="$LAST_WORKFLOW_SUMMARY_FILE"
    fi

    if [ "$print_summary" -eq 1 ] && [ "$nested_workflow_call" -eq 0 ]; then
      local events_file="$REGISTRY_ROOT/events.ndjson"
      local passed failed canceled
      print_workflow_summary_report "$run_id" "$workflow_id" "$status" "$duration_seconds" "$summary_file"

      if [ -n "$summary_file" ] && [ -f "$summary_file" ]; then
        passed="$(${pkgs.jq}/bin/jq -r '.counts.passed // 0' "$summary_file" 2>/dev/null || echo 0)"
        failed="$(${pkgs.jq}/bin/jq -r '.counts.failed // 0' "$summary_file" 2>/dev/null || echo 0)"
        canceled="$(${pkgs.jq}/bin/jq -r '.counts.canceled // 0' "$summary_file" 2>/dev/null || echo 0)"
      elif [ -f "$events_file" ]; then
        passed="$(${pkgs.jq}/bin/jq -r --arg runId "$run_id" 'select(.runId == $runId and .taskId != "" and .state == "passed") | .state' "$events_file" | ${pkgs.gnugrep}/bin/grep -c '^passed$' || true)"
        failed="$(${pkgs.jq}/bin/jq -r --arg runId "$run_id" 'select(.runId == $runId and .taskId != "" and .state == "failed") | .state' "$events_file" | ${pkgs.gnugrep}/bin/grep -c '^failed$' || true)"
        canceled="$(${pkgs.jq}/bin/jq -r --arg runId "$run_id" 'select(.runId == $runId and .taskId != "" and .state == "canceled") | .state' "$events_file" | ${pkgs.gnugrep}/bin/grep -c '^canceled$' || true)"
      else
        passed=0
        failed=0
        canceled=0
      fi
      echo "INFO: runId=$run_id passed=$passed failed=$failed canceled=$canceled"
    fi

    if [ "$managed_by_orchestrator" -eq 0 ]; then
      trap - EXIT
      deactivate_run "$run_id"
    fi

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
