{
  pkgs,
  model,
  selectionIndex ? null,
  services,
  runtimeHash ? model.identity.evalHash,
  registry,
  projectRoot,
  serviceHookEnv ? { },
}:
let
  lib = pkgs.lib;
  resolvedSelectionIndex =
    if selectionIndex != null then
      selectionIndex
    else
      import ../../compiler/compile-selection-index.nix
        {
          inherit (pkgs) lib;
        }
        {
          tasks = model.tasks or { };
          workflows = model.workflows or { };
          serviceCatalog = model.serviceCatalog or { };
        };
  shellCommon = import ../core/shell-common.nix { inherit pkgs; };
  registryShell = registry.events.mkShellLib { };
  workflowModesShell = import ./workflow-modes.nix {
    inherit
      pkgs
      model
      ;
    selectionIndex = resolvedSelectionIndex;
  };
  orchestratorRuntimeShell = import ./orchestrator-runtime.nix { inherit pkgs; };
  executor = import ./executor.nix {
    inherit
      pkgs
      model
      services
      registry
      projectRoot
      serviceHookEnv
      ;
    selectionIndex = resolvedSelectionIndex;
  };
  frameworkEphemeral = import ./ephemeral.nix {
    inherit
      pkgs
      projectRoot
      ;
    project = {
      project = {
        id = model.identity.projectId;
        slotVar = model.runtime.slot.var;
        envVar = model.runtime.env.var;
      };
      state = {
        registry = {
          root = model.state.registry.root;
        };
      };
      slots = {
        max = model.runtime.slot.max;
      };
      install = {
        deps = "";
      };
      tooling = {
        runtimePackages = [
          pkgs.coreutils
          pkgs.findutils
          pkgs.gnused
          pkgs.gnugrep
          pkgs.gawk
          pkgs.jq
          pkgs.git
        ];
      };
      ephemeral = model.runtime.ephemeral or { };
    };
  };
  ephemeralExecutorWrapper = frameworkEphemeral.mkEphemeralWrapper {
    name = "orchestrator-executor";
    installDeps = false;
    script = ''
      ${shellCommon}
      if [ "$#" -lt 1 ]; then
        nixfied_exit_usage "missing command for ephemeral wrapper"
      fi
      export NIXFIED_CALLER_PWD="$(pwd -P)"
      exec "$@"
    '';
  };
  setsidBin = if pkgs ? util-linux then "${pkgs.util-linux}/bin/setsid" else "";
in
pkgs.writeShellScriptBin "nixfied-orchestrator" ''
  set -euo pipefail
  ${shellCommon}
  export NIXFIED_ORCHESTRATOR_BIN="$0"
  export NIXFIED_ORCHESTRATOR_SELF="$0"

  EXECUTOR_PROGRAM=${lib.escapeShellArg "${executor}/bin/nixfied-executor"}
  export NIXFIED_EXECUTOR_BIN="$EXECUTOR_PROGRAM"
  EPHEMERAL_EXECUTOR_WRAPPER=${lib.escapeShellArg (builtins.toString ephemeralExecutorWrapper)}
  PROJECT_ROOT=${lib.escapeShellArg (builtins.toString projectRoot)}
  REGISTRY_ROOT_DEFAULT=${lib.escapeShellArg model.state.registry.root}
  ARTIFACTS_ROOT_DEFAULT=${lib.escapeShellArg model.state.artifacts.root}
  if [ -n "''${REGISTRY_ROOT+x}" ]; then
    REGISTRY_ROOT_EXPLICIT=1
  else
    REGISTRY_ROOT_EXPLICIT=0
  fi
  REGISTRY_ROOT="''${REGISTRY_ROOT:-$REGISTRY_ROOT_DEFAULT}"

  RUNS_DIR="$REGISTRY_ROOT/orchestrator/runs"
  RUN_LOCKS_DIR="$REGISTRY_ROOT/orchestrator/locks"
  RUN_LOG_DIR="$REGISTRY_ROOT/orchestrator/logs"
  RUN_ID_ACTIVE_ROOT="$REGISTRY_ROOT/active"
  RUN_ID_COUNTER_ROOT="$REGISTRY_ROOT/counters"
  SETSID_BIN=${lib.escapeShellArg setsidBin}
  ORCHESTRATOR_STOP_TIMEOUT_SEC_DEFAULT=${lib.escapeShellArg (toString model.runtime.orchestrator.stopTimeoutSec)}

  ${registryShell}
  ${workflowModesShell}
  ${orchestratorRuntimeShell}

  mkdir -p "$RUNS_DIR" "$RUN_LOCKS_DIR" "$RUN_LOG_DIR"

  FOREGROUND_RUN_ACTIVE=0
  FOREGROUND_RUN_ID=""
  FOREGROUND_RUN_PID=""
  FOREGROUND_RUN_PGID=""
  FOREGROUND_RUN_WORKFLOW_ID=""
  FOREGROUND_RUN_TASK_ID=""
  FOREGROUND_SIGNAL_FILE=""
  FOREGROUND_SIGNAL_NAME=""
  RUN_SUFFIX_REASON=""

  sha256_text() {
    printf '%s' "$1" | ${pkgs.coreutils}/bin/sha256sum | ${pkgs.gawk}/bin/awk '{print $1}'
  }

  compute_attempt_id() {
    local attempt_dir
    local attempt_name

    attempt_dir="$(mktemp -d "''${TMPDIR:-/tmp}/nixfied-attempt.XXXXXX")" || return 1
    attempt_name="$(basename "$attempt_dir")"
    rmdir "$attempt_dir"
    printf '%s' "attempt-''${attempt_name#nixfied-attempt.}"
  }

  canonical_run_id_envelope() {
    local run_kind="$1"
    local workflow_id="$2"
    local task_id="$3"
    local slot_value="$4"
    local env_value="$5"
    local pass_through_env_json="$6"
    local argv_json
    shift 6

    argv_json="$(jq_positional_args_json "$@")" || return 1

    ${pkgs.jq}/bin/jq -cnS \
      --arg modelEvalHash "${model.identity.evalHash}" \
      --arg runtimeHash "${runtimeHash}" \
      --arg runKind "$run_kind" \
      --arg workflowId "$workflow_id" \
      --arg taskId "$task_id" \
      --arg slot "$slot_value" \
      --arg env "$env_value" \
      --argjson passThroughEnv "$pass_through_env_json" \
      --argjson argv "$argv_json" \
      '{
        model_eval_hash: $modelEvalHash,
        runtime_hash: $runtimeHash,
        run_kind: $runKind,
        workflow_id: (if $workflowId == "" then null else $workflowId end),
        task_id: (if $taskId == "" then null else $taskId end),
        slot: $slot,
        env: $env,
        pass_through_env: $passThroughEnv,
        argv: $argv
      }'
  }

  filter_run_id_args() {
    local parse_options=1
    local arg=""
    local -a filtered_args=()

    while [ "$#" -gt 0 ]; do
      arg="$1"
      shift

      if [ "$parse_options" -eq 0 ]; then
        filtered_args+=("$arg")
        continue
      fi

      case "$arg" in
        --summary)
          ;;
        --log-level)
          if [ "$#" -lt 1 ]; then
            echo "ERROR: --log-level requires a value"
            return 2
          fi
          shift
          ;;
        --log-level=*)
          ;;
        --output-mode)
          if [ "$#" -lt 1 ]; then
            echo "ERROR: --output-mode requires a value"
            return 2
          fi
          shift
          ;;
        --output-mode=*)
          ;;
        --)
          parse_options=0
          filtered_args+=("--")
          ;;
        *)
          filtered_args+=("$arg")
          ;;
      esac
    done

    printf '%s\n' "''${filtered_args[@]}"
  }

  emit_runtime_pass_through_env_names() {
    local runtime_json="$1"
    printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '(.passThroughEnv // [])[]?'
  }

  run_id_pass_through_env_json() {
    local run_kind="$1"
    local workflow_id="$2"
    local task_id="$3"
    local env_name=""
    local dep_task_id=""
    local phase=""
    local hook_id=""
    local runner_type=""
    local nested_workflow_id=""
    local unit_json=""
    local unit_task_id=""
    local -A seen_tasks
    local -A seen_workflows

    collect_task_env_names() {
      local current_task_id="$1"

      if [ -z "$current_task_id" ] || [ -n "''${seen_tasks[$current_task_id]:-}" ]; then
        return 0
      fi
      seen_tasks[$current_task_id]=1

      emit_runtime_pass_through_env_names "$(task_runtime_json "$current_task_id")"

      for phase in pre post; do
        while IFS= read -r hook_id; do
          [ -n "$hook_id" ] || continue
          emit_runtime_pass_through_env_names "$(task_hook_runtime_json "$current_task_id" "$phase" "$hook_id")"
        done < <(task_hook_ids "$current_task_id" "$phase" 2>/dev/null || true)
      done

      while IFS= read -r dep_task_id; do
        [ -n "$dep_task_id" ] || continue
        collect_task_env_names "$dep_task_id"
      done < <(task_needs "$current_task_id" 2>/dev/null || true)

      while IFS= read -r dep_task_id; do
        [ -n "$dep_task_id" ] || continue
        collect_task_env_names "$dep_task_id"
      done < <(task_soft_needs "$current_task_id" 2>/dev/null || true)

      runner_type="$(task_runner_type "$current_task_id")"
      if [ "$runner_type" = "workflowRef" ]; then
        nested_workflow_id="$(task_runner_workflow_id "$current_task_id")"
        if [ -n "$nested_workflow_id" ]; then
          collect_workflow_env_names "$nested_workflow_id"
        fi
      fi
    }

    collect_workflow_env_names() {
      local current_workflow_id="$1"

      if [ -z "$current_workflow_id" ] || [ -n "''${seen_workflows[$current_workflow_id]:-}" ]; then
        return 0
      fi
      seen_workflows[$current_workflow_id]=1

      while IFS= read -r dep_task_id; do
        [ -n "$dep_task_id" ] || continue
        collect_task_env_names "$dep_task_id"
      done < <(workflow_phase_tasks "$current_workflow_id" preRun 2>/dev/null || true)

      while IFS= read -r unit_json; do
        [ -n "$unit_json" ] || continue
        unit_task_id="$(workflow_unit_task_id "$unit_json")"
        if [ -n "$unit_task_id" ] && [ "$unit_task_id" != "null" ]; then
          collect_task_env_names "$unit_task_id"
        fi
      done < <(workflow_plan_records "$current_workflow_id" 2>/dev/null || true)

      while IFS= read -r dep_task_id; do
        [ -n "$dep_task_id" ] || continue
        collect_task_env_names "$dep_task_id"
      done < <(workflow_phase_tasks "$current_workflow_id" postRun 2>/dev/null || true)
    }

    {
      while IFS= read -r env_name; do
        [ -n "$env_name" ] || continue
        if [ -n "''${!env_name+x}" ]; then
          ${pkgs.jq}/bin/jq -cn --arg key "$env_name" --arg value "''${!env_name}" '{key: $key, value: $value}'
        fi
      done < <(
        case "$run_kind" in
          task)
            collect_task_env_names "$task_id"
            ;;
          workflow)
            collect_workflow_env_names "$workflow_id"
            ;;
        esac | ${pkgs.coreutils}/bin/sort -u
      )
    } | ${pkgs.jq}/bin/jq -cs 'from_entries'
  }

  compute_run_id() {
    local run_kind="$1"
    local workflow_id="$2"
    local task_id="$3"
    shift 3

    local slot_var=${pkgs.lib.escapeShellArg model.runtime.slot.var}
    local env_var=${pkgs.lib.escapeShellArg model.runtime.env.var}
    local slot_default=${toString model.runtime.slot.default}
    local env_default=${pkgs.lib.escapeShellArg model.runtime.env.default}

    local slot_value
    local env_value
    local pass_through_env_json
    local run_input
    local run_base
    local run_id
    local lock_file
    local counter_file
    local lock_fd
    local counter="0"

    slot_value="''${!slot_var:-$slot_default}"
    env_value="''${!env_var:-$env_default}"
    pass_through_env_json="$(run_id_pass_through_env_json "$run_kind" "$workflow_id" "$task_id")"

    run_input="$(canonical_run_id_envelope "$run_kind" "$workflow_id" "$task_id" "$slot_value" "$env_value" "$pass_through_env_json" "$@")"
    run_base="$(sha256_text "$run_input")"
    run_id="run-''${run_base:0:24}"
    RUN_SUFFIX_REASON=""

    mkdir -p "$RUN_ID_ACTIVE_ROOT" "$RUN_ID_COUNTER_ROOT"
    lock_file="$RUN_ID_COUNTER_ROOT/$run_base.lock"
    counter_file="$RUN_ID_COUNTER_ROOT/$run_base"
    lock_fd="$(registry_lock_acquire "$lock_file" "orchestrator-run-counter:$run_base" 30)" || return 1

    if [ -e "$RUN_ID_ACTIVE_ROOT/$run_id" ]; then
      if [ -f "$counter_file" ]; then
        counter="$(cat "$counter_file")"
      fi
      counter="$(( counter + 1 ))"
      printf '%s' "$counter" > "$counter_file"

      run_id="$run_id-$(printf 'c%03d' "$counter")"
      RUN_SUFFIX_REASON="active-collision"
    fi

    : > "$RUN_ID_ACTIVE_ROOT/$run_id"
    registry_lock_release "$lock_fd" "$lock_file"

    printf '%s' "$run_id"
  }

  activate_run_id() {
    local run_id="$1"
    mkdir -p "$RUN_ID_ACTIVE_ROOT"
    : > "$RUN_ID_ACTIVE_ROOT/$run_id"
  }

  deactivate_run_id() {
    local run_id="$1"
    rm -f "$RUN_ID_ACTIVE_ROOT/$run_id"
  }

  iso_now() {
    date -u +"%Y-%m-%dT%H:%M:%SZ"
  }

  run_file_for() {
    local run_id="$1"
    printf '%s/%s.json' "$RUNS_DIR" "$run_id"
  }

  run_lock_for() {
    local run_id="$1"
    printf '%s/.lock-%s' "$RUN_LOCKS_DIR" "$run_id"
  }

  pgid_of_pid() {
    local pid="$1"
    local parent_pgid
    local pgid

    parent_pgid="$(${pkgs.procps}/bin/ps -o pgid= -p $$ 2>/dev/null | tr -d '[:space:]' || true)"
    pgid="$(${pkgs.procps}/bin/ps -o pgid= -p "$pid" 2>/dev/null | tr -d '[:space:]' || true)"

    if [ -z "$pgid" ] || [ "$pgid" = "$parent_pgid" ]; then
      printf '0'
      return
    fi

    printf '%s' "$pgid"
  }

  is_pid_running() {
    local pid="$1"
    if [ -z "$pid" ] || [ "$pid" = "null" ]; then
      return 1
    fi
    kill -0 "$pid" 2>/dev/null
  }

  map_terminal_state() {
    local state="$1"
    case "$state" in
      passed|failed|canceled)
        return 0
        ;;
      *)
        return 1
        ;;
    esac
  }

  stop_timeout_seconds() {
    local value="''${NIXFIED_ORCHESTRATOR_STOP_TIMEOUT_SECONDS:-$ORCHESTRATOR_STOP_TIMEOUT_SEC_DEFAULT}"

    if [[ "$value" =~ ^[0-9]+$ ]]; then
      printf '%s' "$value"
      return 0
    fi

    echo "WARN: ignoring invalid NIXFIED_ORCHESTRATOR_STOP_TIMEOUT_SECONDS='$value'; using default $ORCHESTRATOR_STOP_TIMEOUT_SEC_DEFAULT"
    printf '%s' "$ORCHESTRATOR_STOP_TIMEOUT_SEC_DEFAULT"
  }

  run_state_is_terminal() {
    local state="$1"
    case "$state" in
      passed|failed|canceled)
        return 0
        ;;
      *)
        return 1
        ;;
    esac
  }

  signal_run_process() {
    local signal_name="$1"
    local pid="$2"
    local pgid="$3"

    if [ -n "$pgid" ] && [ "$pgid" != "0" ]; then
      kill -"$signal_name" -- "-$pgid" 2>/dev/null || true
    elif [ -n "$pid" ] && [ "$pid" != "null" ]; then
      kill -"$signal_name" "$pid" 2>/dev/null || true
    fi
  }

  wait_for_run_process_exit() {
    local pid="$1"
    local timeout_seconds="$2"
    local waited=0

    if ! is_pid_running "$pid"; then
      return 0
    fi

    while [ "$waited" -lt "$timeout_seconds" ]; do
      if ! is_pid_running "$pid"; then
        return 0
      fi
      sleep "$NIXFIED_POLL_INTERVAL_DEFAULT"
      waited=$((waited + 1))
    done

    ! is_pid_running "$pid"
  }

  terminate_run_process() {
    local pid="$1"
    local pgid="$2"
    local timeout_seconds="$3"

    signal_run_process TERM "$pid" "$pgid"
    wait_for_run_process_exit "$pid" "$timeout_seconds" || true

    if is_pid_running "$pid"; then
      signal_run_process KILL "$pid" "$pgid"
    fi
  }

  append_run_canceled_event() {
    local run_id="$1"
    local workflow_id="$2"
    local task_id="$3"
    local signal_name="$4"
    local attempt_id=""
    local detail_json
    local run_file=""

    if [ -z "$workflow_id" ] && [ -z "$task_id" ]; then
      return 0
    fi

    run_file="$(run_file_for "$run_id")"
    if [ -f "$run_file" ]; then
      attempt_id="$(run_file_attempt_id "$run_file")"
    fi

    detail_json="$(${pkgs.jq}/bin/jq -cn --arg reason "orchestrator-interrupted" --arg signal "$signal_name" '{reason: $reason, signal: $signal}')"
    if [ -n "$workflow_id" ]; then
      registry_append_event "$REGISTRY_ROOT" "$run_id" "$attempt_id" "$workflow_id" "" "canceled" "$detail_json"
    else
      registry_append_event "$REGISTRY_ROOT" "$run_id" "$attempt_id" "" "$task_id" "canceled" "$detail_json"
    fi
  }

  clear_foreground_run_context() {
    FOREGROUND_RUN_ACTIVE=0
    FOREGROUND_RUN_ID=""
    FOREGROUND_RUN_PID=""
    FOREGROUND_RUN_PGID=""
    FOREGROUND_RUN_WORKFLOW_ID=""
    FOREGROUND_RUN_TASK_ID=""
    FOREGROUND_SIGNAL_FILE=""
    FOREGROUND_SIGNAL_NAME=""
  }

  set_foreground_run_context() {
    FOREGROUND_RUN_ACTIVE=1
    FOREGROUND_RUN_ID="$1"
    FOREGROUND_RUN_PID="$2"
    FOREGROUND_RUN_PGID="$3"
    FOREGROUND_RUN_WORKFLOW_ID="$4"
    FOREGROUND_RUN_TASK_ID="$5"
  }

  create_run_record() {
    local run_id="$1"
    local attempt_id="$2"
    local command_name="$3"
    local workflow_id="$4"
    local task_id="$5"
    local execution_mode="$6"
    local process_mode="$7"
    local ephemeral_enabled="$8"
    local args_json="$9"

    local run_file
    local now
    local lock_file
    local lock_fd
    local tmp

    run_file="$(run_file_for "$run_id")"
    now="$(iso_now)"
    lock_file="$(run_lock_for "$run_id")"
    lock_fd="$(registry_lock_acquire "$lock_file" "orchestrator-create-run-record:$run_id" 30)" || return 1
    tmp="$(mktemp "$run_file.tmp.XXXXXX")"

    if ! ${pkgs.jq}/bin/jq -cnS \
      --arg runId "$run_id" \
      --arg attemptId "$attempt_id" \
      --arg command "$command_name" \
      --arg workflowId "$workflow_id" \
      --arg taskId "$task_id" \
      --arg executionMode "$execution_mode" \
      --arg processMode "$process_mode" \
      --argjson ephemeralEnabled "$( [ "$ephemeral_enabled" = "1" ] && printf true || printf false )" \
      --argjson args "$args_json" \
      --arg ts "$now" \
      '{
        run_id: $runId,
        attempt_id: $attemptId,
        command: $command,
        workflow_id: $workflowId,
        task_id: $taskId,
        execution_mode: $executionMode,
        process_mode: $processMode,
        ephemeral_enabled: $ephemeralEnabled,
        state: "queued",
        pid: null,
        pgid: null,
        exit_code: null,
        stop_reason: null,
        created_at: $ts,
        started_at: null,
        finished_at: null,
        updated_at: $ts,
        args: $args,
        history: [
          {
            state: "queued",
            at: $ts
          }
        ]
      }' > "$tmp"; then
      rm -f "$tmp"
      registry_lock_release "$lock_fd" "$lock_file"
      return 1
    fi

    mv "$tmp" "$run_file"
    registry_lock_release "$lock_fd" "$lock_file"

  }

  update_run_state() {
    local run_id="$1"
    local state="$2"
    local exit_code_json="$3"
    local stop_reason="$4"
    local pid="$5"
    local pgid="$6"

    local run_file
    local lock_file
    local lock_fd
    local tmp
    local now

    run_file="$(run_file_for "$run_id")"
    lock_file="$(run_lock_for "$run_id")"
    now="$(iso_now)"

    if [ ! -f "$run_file" ]; then
      return 1
    fi

    lock_fd="$(registry_lock_acquire "$lock_file" "orchestrator-update-run-state:$run_id" 30)" || return 1
    tmp="$(mktemp "$run_file.tmp.XXXXXX")"

    if ! ${pkgs.jq}/bin/jq -cS \
      --arg state "$state" \
      --arg ts "$now" \
      --argjson exitCode "$exit_code_json" \
      --arg stopReason "$stop_reason" \
      --arg pid "$pid" \
      --arg pgid "$pgid" \
      '
      .state = $state
      | .updated_at = $ts
      | .history += [{state: $state, at: $ts}]
      | .pid = (if $pid == "" then .pid else ($pid | tonumber) end)
      | .pgid = (if $pgid == "" then .pgid else ($pgid | tonumber) end)
      | .started_at = (if (.started_at == null and $state == "running") then $ts else .started_at end)
      | .finished_at = (if ($state == "passed" or $state == "failed" or $state == "canceled") then $ts else .finished_at end)
      | .exit_code = (if $exitCode == null then .exit_code else $exitCode end)
      | .stop_reason = (if $stopReason == "" then .stop_reason else $stopReason end)
      ' "$run_file" > "$tmp"; then
      rm -f "$tmp"
      registry_lock_release "$lock_fd" "$lock_file"
      return 1
    fi

    mv "$tmp" "$run_file"
    registry_lock_release "$lock_fd" "$lock_file"

    case "$state" in
      passed|failed|canceled)
        deactivate_run_id "$run_id"
        ;;
    esac
    return 0
  }

  terminal_from_events() {
    local run_id="$1"
    local attempt_id="$2"
    local events_file
    local terminal_state
    local exit_code

    events_file="$(registry_events_snapshot "$REGISTRY_ROOT")" || {
      echo "unknown 1"
      return
    }

    if [ -z "$events_file" ] || [ ! -f "$events_file" ]; then
      echo "unknown 1"
      return
    fi

    terminal_state="$(${pkgs.jq}/bin/jq -r --arg runId "$run_id" --arg attemptId "$attempt_id" '
      select(.runId == $runId and ($attemptId == "" or (.attemptId // "") == $attemptId) and (.state == "passed" or .state == "failed" or .state == "canceled"))
      | .state
    ' "$events_file" | ${pkgs.coreutils}/bin/tail -n 1)"

    if [ -z "$terminal_state" ]; then
      registry_snapshot_cleanup "$events_file"
      echo "unknown 1"
      return
    fi

    case "$terminal_state" in
      passed)
        registry_snapshot_cleanup "$events_file"
        echo "passed 0"
        ;;
      canceled)
        registry_snapshot_cleanup "$events_file"
        echo "canceled 130"
        ;;
      failed)
        exit_code="$(${pkgs.jq}/bin/jq -r --arg runId "$run_id" --arg attemptId "$attempt_id" '
          select(.runId == $runId and ($attemptId == "" or (.attemptId // "") == $attemptId) and .state == "failed")
          | .detail.exitCode // empty
        ' "$events_file" | ${pkgs.coreutils}/bin/tail -n 1)"
        if [ -z "$exit_code" ]; then
          exit_code=1
        fi
        registry_snapshot_cleanup "$events_file"
        echo "failed $exit_code"
        ;;
      *)
        registry_snapshot_cleanup "$events_file"
        echo "unknown 1"
        ;;
    esac
  }

  refresh_one_run() {
    local run_id="$1"
    local run_file
    local state
    local pid
    local attempt_id=""
    local terminal_state
    local terminal_code

    run_file="$(run_file_for "$run_id")"
    if [ ! -f "$run_file" ]; then
      return 1
    fi

    state="$(run_file_state "$run_file")"
    if [ "$state" != "running" ]; then
      return 0
    fi

    pid="$(run_file_pid "$run_file")"
    if is_pid_running "$pid"; then
      return 0
    fi

    attempt_id="$(run_file_attempt_id "$run_file")"
    read -r terminal_state terminal_code <<<"$(terminal_from_events "$run_id" "$attempt_id")"
    if ! map_terminal_state "$terminal_state"; then
      terminal_state="failed"
      terminal_code="1"
    fi

    update_run_state "$run_id" "$terminal_state" "$terminal_code" "" "" ""
  }

  refresh_all_runs() {
    local run_file
    local run_id

    if [ ! -d "$RUNS_DIR" ]; then
      return 0
    fi

    while IFS= read -r run_file; do
      run_id="$(basename "$run_file" .json)"
      refresh_one_run "$run_id" || true
    done < <(${pkgs.findutils}/bin/find "$RUNS_DIR" -type f -name '*.json' | ${pkgs.coreutils}/bin/sort)
  }

  cleanup_interrupted_run() {
    local run_id="$1"
    local pid="$2"
    local pgid="$3"
    local workflow_id="$4"
    local task_id="$5"
    local signal_name="$6"
    local timeout_seconds
    local run_file
    local current_state
    local terminal_state
    local terminal_code
    local stop_reason
    local attempt_id=""

    timeout_seconds="$(stop_timeout_seconds)"
    terminate_run_process "$pid" "$pgid" "$timeout_seconds"

    run_file="$(run_file_for "$run_id")"
    if [ -f "$run_file" ]; then
      attempt_id="$(run_file_attempt_id "$run_file")"
      current_state="$(run_file_state "$run_file")"
      if ! run_state_is_terminal "$current_state"; then
        read -r terminal_state terminal_code <<<"$(terminal_from_events "$run_id" "$attempt_id")"
        if map_terminal_state "$terminal_state"; then
          update_run_state "$run_id" "$terminal_state" "$terminal_code" "" "$pid" "$pgid" || true
        else
          if [ -z "$signal_name" ]; then
            signal_name="EXIT"
          fi
          append_run_canceled_event "$run_id" "$workflow_id" "$task_id" "$signal_name" || {
            echo "WARN: failed to append cancellation event run_id=$run_id signal=$signal_name"
          }
          stop_reason="signal-$(printf '%s' "$signal_name" | ${pkgs.coreutils}/bin/tr '[:upper:]' '[:lower:]')"
          update_run_state "$run_id" "canceled" "130" "$stop_reason" "$pid" "$pgid" || true
        fi
      fi
    fi
    return 0
  }

  start_foreground_janitor() {
    local parent_pid="$1"
    local run_id="$2"
    local pid="$3"
    local pgid="$4"
    local workflow_id="$5"
    local task_id="$6"
    local control_file="$7"
    local signal_file="$8"

    if [ -n "$SETSID_BIN" ] && [ -x "$SETSID_BIN" ]; then
      "$SETSID_BIN" "$NIXFIED_ORCHESTRATOR_SELF" janitor-run "$parent_pid" "$run_id" "$pid" "$pgid" "$workflow_id" "$task_id" "$control_file" "$signal_file" >/dev/null 2>&1 < /dev/null &
    elif command -v setsid >/dev/null 2>&1; then
      setsid "$NIXFIED_ORCHESTRATOR_SELF" janitor-run "$parent_pid" "$run_id" "$pid" "$pgid" "$workflow_id" "$task_id" "$control_file" "$signal_file" >/dev/null 2>&1 < /dev/null &
    else
      ${pkgs.coreutils}/bin/nohup "$NIXFIED_ORCHESTRATOR_SELF" janitor-run "$parent_pid" "$run_id" "$pid" "$pgid" "$workflow_id" "$task_id" "$control_file" "$signal_file" >/dev/null 2>&1 < /dev/null &
    fi
    printf '%s' "$!"
  }

  janitor_run_loop() {
    local parent_pid="$1"
    local run_id="$2"
    local pid="$3"
    local pgid="$4"
    local workflow_id="$5"
    local task_id="$6"
    local control_file="$7"
    local signal_file="$8"
    local signal_name

    while [ -f "$control_file" ]; do
      if ! kill -0 "$parent_pid" 2>/dev/null; then
        signal_name="EXIT"
        if [ -f "$signal_file" ]; then
          signal_name="$(tr -d '\n' < "$signal_file")"
          if [ -z "$signal_name" ]; then
            signal_name="EXIT"
          fi
        fi
        cleanup_interrupted_run "$run_id" "$pid" "$pgid" "$workflow_id" "$task_id" "$signal_name"
        rm -f "$control_file" "$signal_file"
        return 0
      fi
      sleep "$NIXFIED_POLL_INTERVAL_FAST"
    done

    return 0
  }

  handle_foreground_signal() {
    local signal_name="$1"

    FOREGROUND_SIGNAL_NAME="$signal_name"
    if [ -n "$FOREGROUND_SIGNAL_FILE" ]; then
      printf '%s' "$signal_name" > "$FOREGROUND_SIGNAL_FILE" || true
    fi
    return 0
  }

  launch_command() {
    local run_id="$1"
    local process_mode="$2"
    local terminal_workflow_id="$3"
    local terminal_task_id="$4"
    shift 4

    local -a cmd
    local pid
    local pgid
    local rc
    local log_file
    local run_file
    local current_state
    local janitor_dir
    local control_file
    local signal_file
    local janitor_pid
    local interrupted_signal=""

    cmd=("$@")

    if [ "$process_mode" = "bg" ]; then
      log_file="$RUN_LOG_DIR/$run_id.log"
      if [ -n "$SETSID_BIN" ] && [ -x "$SETSID_BIN" ]; then
        "$SETSID_BIN" "''${cmd[@]}" >> "$log_file" 2>&1 < /dev/null &
      elif command -v setsid >/dev/null 2>&1; then
        setsid "''${cmd[@]}" >> "$log_file" 2>&1 < /dev/null &
      else
        "''${cmd[@]}" >> "$log_file" 2>&1 < /dev/null &
      fi
      pid="$!"
      pgid="$(pgid_of_pid "$pid")"
      update_run_state "$run_id" "running" "null" "" "$pid" "$pgid"
      echo "OK: detached run_id=$run_id pid=$pid pgid=$pgid log=$log_file"
      return 0
    fi

    trap 'handle_foreground_signal INT' INT
    trap 'handle_foreground_signal TERM' TERM

    if [ -n "$SETSID_BIN" ] && [ -x "$SETSID_BIN" ]; then
      "$SETSID_BIN" "''${cmd[@]}" &
    elif command -v setsid >/dev/null 2>&1; then
      setsid "''${cmd[@]}" &
    else
      "''${cmd[@]}" &
    fi

    pid="$!"
    pgid="$(pgid_of_pid "$pid")"
    set_foreground_run_context "$run_id" "$pid" "$pgid" "$terminal_workflow_id" "$terminal_task_id"
    janitor_dir="$(mktemp -d "$RUN_LOCKS_DIR/fg-$run_id.XXXXXX")"
    control_file="$janitor_dir/alive"
    signal_file="$janitor_dir/signal"
    : > "$control_file"
    FOREGROUND_SIGNAL_FILE="$signal_file"
    janitor_pid="$(start_foreground_janitor "$$" "$run_id" "$pid" "$pgid" "$terminal_workflow_id" "$terminal_task_id" "$control_file" "$signal_file")"
    update_run_state "$run_id" "running" "null" "" "$pid" "$pgid"

    set +e
    wait "$pid"
    rc="$?"
    set -e
    trap - INT TERM

    if [ -n "$FOREGROUND_SIGNAL_NAME" ]; then
      interrupted_signal="$FOREGROUND_SIGNAL_NAME"
    elif [ -f "$signal_file" ]; then
      interrupted_signal="$(tr -d '\n' < "$signal_file")"
    fi

    if [ -n "$interrupted_signal" ]; then
      cleanup_interrupted_run "$run_id" "$pid" "$pgid" "$terminal_workflow_id" "$terminal_task_id" "$interrupted_signal"
    fi

    rm -f "$control_file" "$signal_file"
    wait "$janitor_pid" 2>/dev/null || true
    rmdir "$janitor_dir" 2>/dev/null || true
    clear_foreground_run_context

    run_file="$(run_file_for "$run_id")"
    if [ -f "$run_file" ]; then
      current_state="$(run_file_state "$run_file")"
      if run_state_is_terminal "$current_state"; then
        case "$current_state" in
          passed)
            return 0
            ;;
          canceled)
            return 130
            ;;
          failed)
            if [ -n "$interrupted_signal" ]; then
              return 130
            fi
            if [ "$rc" -eq 0 ]; then
              return 1
            fi
            return "$rc"
            ;;
        esac
      fi
    fi

    if [ -n "$interrupted_signal" ]; then
      return 130
    fi

    if [ "$rc" -eq 0 ]; then
      update_run_state "$run_id" "passed" "0" "" "$pid" "$pgid"
    else
      update_run_state "$run_id" "failed" "$rc" "" "$pid" "$pgid"
    fi

    return "$rc"
  }

  list_runs() {
    refresh_all_runs

    local run_file
    local run_id
    local state
    local command_name
    local mode
    local pid
    local pgid
    local listed=0

    if [ ! -d "$RUNS_DIR" ]; then
      echo "INFO: no runs"
      return 0
    fi

    while IFS= read -r run_file; do
      run_id="$(basename "$run_file" .json)"
      command_name="$(run_file_command "$run_file")"
      state="$(run_file_state "$run_file")"
      mode="$(run_file_process_mode "$run_file")"
      pid="$(run_file_pid "$run_file")"
      pgid="$(run_file_pgid "$run_file")"

      echo "INFO: run_id=$run_id state=$state command=$command_name mode=$mode pid=$pid pgid=$pgid"
      listed=$((listed + 1))
    done < <(${pkgs.findutils}/bin/find "$RUNS_DIR" -type f -name '*.json' | ${pkgs.coreutils}/bin/sort)

    if [ "$listed" -eq 0 ]; then
      echo "INFO: no runs"
    fi
  }

  show_run() {
    local run_id="$1"
    local run_file

    run_file="$(run_file_for "$run_id")"
    if [ ! -f "$run_file" ]; then
      echo "ERROR: unknown run '$run_id'"
      return "$NIXFIED_EXIT_USAGE"
    fi

    refresh_one_run "$run_id" || true
    ${pkgs.jq}/bin/jq -cS '.' "$run_file"
  }

  stop_single_run() {
    local run_id="$1"
    local run_file
    local state
    local pid
    local pgid
    local timeout_seconds

    run_file="$(run_file_for "$run_id")"
    if [ ! -f "$run_file" ]; then
      echo "ERROR: unknown run '$run_id'"
      return "$NIXFIED_EXIT_USAGE"
    fi

    refresh_one_run "$run_id" || true
    state="$(run_file_state "$run_file")"

    case "$state" in
      passed|failed|canceled)
        echo "SKIP: run already terminal run_id=$run_id state=$state"
        return 0
        ;;
    esac

    pid="$(run_file_pid "$run_file")"
    pgid="$(run_file_pgid "$run_file")"
    timeout_seconds="$(stop_timeout_seconds)"
    terminate_run_process "$pid" "$pgid" "$timeout_seconds"

    update_run_state "$run_id" "canceled" "130" "stop-requested" "$pid" "$pgid"
    echo "OK: stopped run_id=$run_id"
  }

  stop_all_runs() {
    refresh_all_runs

    local run_file
    local run_id
    local status=0
    local state

    while IFS= read -r run_file; do
      run_id="$(basename "$run_file" .json)"
      state="$(run_file_state "$run_file")"
      if [ "$state" = "running" ] || [ "$state" = "queued" ]; then
        if ! stop_single_run "$run_id"; then
          status=1
        fi
      fi
    done < <(${pkgs.findutils}/bin/find "$RUNS_DIR" -type f -name '*.json' | ${pkgs.coreutils}/bin/sort)

    return "$status"
  }

  run_task() {
    if [ "$#" -lt 1 ]; then
      echo "ERROR: usage: run-task <task-id> [-- ...]"
      return "$NIXFIED_EXIT_USAGE"
    fi

    local task_id="$1"
    shift

    local command_started_at
    local command_started_epoch
    local workflow_ref=""
    local execution_mode="task"
    local ephemeral_enabled="0"
    local run_id
    local attempt_id
    local args_json
    local -a run_id_args
    run_id_args=()

    command_started_at="$(iso_now)"
    command_started_epoch="$(date +%s)"

    if ! task_descriptor_exists "$task_id"; then
      echo "ERROR: unknown task '$task_id'"
      return "$NIXFIED_EXIT_USAGE"
    fi

    workflow_ref="$(resolve_task_workflow_ref "$task_id")"

    split_process_mode "$@"
    if call_with_array_args FORWARD_ARGS task_help_requested; then
      if ! task_print_help "$task_id"; then
        echo "ERROR: unknown task '$task_id'"
        return "$NIXFIED_EXIT_USAGE"
      fi
      return 0
    fi
    if [ -n "$workflow_ref" ]; then
      call_with_array_args FORWARD_ARGS validate_workflow_args "$workflow_ref"
    else
      call_with_array_args FORWARD_ARGS validate_typed_task_args "$task_id"
    fi

    if [ -n "$workflow_ref" ]; then
      execution_mode="workflow"
      ephemeral_enabled="$(workflow_ephemeral_flag "$workflow_ref")"
    fi

    if [ "$task_id" = "task.ops.test-isolation" ]; then
      ephemeral_enabled=1
    fi

    mapfile -t run_id_args < <(call_with_array_args FORWARD_ARGS filter_run_id_args)
    run_id="$(call_with_array_args run_id_args compute_run_id "task" "$workflow_ref" "$task_id")"
    attempt_id="$(compute_attempt_id)"
    if [ -n "$MACHINE_RUN_ID_FILE" ]; then
      write_text_file_atomic "$MACHINE_RUN_ID_FILE" "$run_id"
    fi
    args_json="$(call_with_array_args FORWARD_ARGS jq_positional_args_json)"
    create_run_record "$run_id" "$attempt_id" "run-task" "$workflow_ref" "$task_id" "$execution_mode" "$PROCESS_MODE" "$ephemeral_enabled" "$args_json" || {
      deactivate_run_id "$run_id"
      return 1
    }

    export NIXFIED_ORCHESTRATOR_RUN_ID="$run_id"
    export NIXFIED_ORCHESTRATOR_ATTEMPT_ID="$attempt_id"
    export NIXFIED_ORCHESTRATOR_RUN_SUFFIX_REASON="$RUN_SUFFIX_REASON"
    export NIXFIED_ORCHESTRATOR_PROCESS_MODE="$PROCESS_MODE"
    export NIXFIED_ORCHESTRATOR_WORKFLOW_ID="$workflow_ref"
    export NIXFIED_WORKFLOW_SETUP_STARTED_AT="$command_started_at"
    export NIXFIED_WORKFLOW_SETUP_STARTED_EPOCH="$command_started_epoch"

    ensure_artifacts_root "$run_id" "$ephemeral_enabled" "$workflow_ref"

    if [ "$ephemeral_enabled" = "1" ]; then
      call_with_array_args FORWARD_ARGS launch_command "$run_id" "$PROCESS_MODE" "$workflow_ref" "$task_id" "$EPHEMERAL_EXECUTOR_WRAPPER" "$EXECUTOR_PROGRAM" run-task "$task_id"
    else
      call_with_array_args FORWARD_ARGS launch_command "$run_id" "$PROCESS_MODE" "$workflow_ref" "$task_id" "$EXECUTOR_PROGRAM" run-task "$task_id"
    fi
  }

  run_workflow() {
    if [ "$#" -lt 1 ]; then
      echo "ERROR: usage: run-workflow <workflow-id> [-- ...]"
      return "$NIXFIED_EXIT_USAGE"
    fi

    local workflow_id="$1"
    shift

    local command_started_at
    local command_started_epoch
    local mode="workflow"
    local run_id
    local attempt_id
    local args_json
    local ephemeral_enabled
    local -a run_id_args
    run_id_args=()

    command_started_at="$(iso_now)"
    command_started_epoch="$(date +%s)"

    if ! workflow_id_exists "$workflow_id"; then
      echo "ERROR: unknown workflow '$workflow_id'"
      return "$NIXFIED_EXIT_USAGE"
    fi

    split_process_mode "$@"
    call_with_array_args FORWARD_ARGS validate_workflow_args "$workflow_id"

    if [ "''${NIXFIED_WORKFLOW_PARALLEL:-}" = "1" ]; then
      mode="workflow-parallel"
    fi

    ephemeral_enabled="$(workflow_ephemeral_flag "$workflow_id")"
    mapfile -t run_id_args < <(call_with_array_args FORWARD_ARGS filter_run_id_args)
    run_id="$(call_with_array_args run_id_args compute_run_id "workflow" "$workflow_id" "")"
    attempt_id="$(compute_attempt_id)"
    if [ -n "$MACHINE_RUN_ID_FILE" ]; then
      write_text_file_atomic "$MACHINE_RUN_ID_FILE" "$run_id"
    fi
    args_json="$(call_with_array_args FORWARD_ARGS jq_positional_args_json)"
    create_run_record "$run_id" "$attempt_id" "run-workflow" "$workflow_id" "" "$mode" "$PROCESS_MODE" "$ephemeral_enabled" "$args_json" || {
      deactivate_run_id "$run_id"
      return 1
    }

    export NIXFIED_ORCHESTRATOR_RUN_ID="$run_id"
    export NIXFIED_ORCHESTRATOR_ATTEMPT_ID="$attempt_id"
    export NIXFIED_ORCHESTRATOR_RUN_SUFFIX_REASON="$RUN_SUFFIX_REASON"
    export NIXFIED_ORCHESTRATOR_PROCESS_MODE="$PROCESS_MODE"
    export NIXFIED_ORCHESTRATOR_WORKFLOW_ID="$workflow_id"
    export NIXFIED_WORKFLOW_SETUP_STARTED_AT="$command_started_at"
    export NIXFIED_WORKFLOW_SETUP_STARTED_EPOCH="$command_started_epoch"

    ensure_artifacts_root "$run_id" "$ephemeral_enabled" "$workflow_id"

    if [ "$ephemeral_enabled" = "1" ]; then
      call_with_array_args FORWARD_ARGS launch_command "$run_id" "$PROCESS_MODE" "$workflow_id" "" "$EPHEMERAL_EXECUTOR_WRAPPER" "$EXECUTOR_PROGRAM" run-workflow "$workflow_id"
    else
      call_with_array_args FORWARD_ARGS launch_command "$run_id" "$PROCESS_MODE" "$workflow_id" "" "$EXECUTOR_PROGRAM" run-workflow "$workflow_id"
    fi
  }

  main() {
    if [ "$#" -lt 1 ]; then
      echo "ERROR: usage: nixfied-orchestrator <run-task|run-workflow|runs|stop-run|stop-all-runs> ..."
      exit "$NIXFIED_EXIT_USAGE"
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
      run-workflow-parallel)
        NIXFIED_WORKFLOW_PARALLEL=1 run_workflow "$@"
        ;;
      runs)
        if [ "$#" -eq 0 ]; then
          list_runs
        elif [ "$#" -eq 1 ]; then
          show_run "$1"
        else
          echo "ERROR: usage: runs [run-id]"
          exit "$NIXFIED_EXIT_USAGE"
        fi
        ;;
      stop-run)
        if [ "$#" -ne 1 ]; then
          echo "ERROR: usage: stop-run <run-id>"
          exit "$NIXFIED_EXIT_USAGE"
        fi
        stop_single_run "$1"
        ;;
      stop-all-runs)
        if [ "$#" -ne 0 ]; then
          echo "ERROR: usage: stop-all-runs"
          exit "$NIXFIED_EXIT_USAGE"
        fi
        stop_all_runs
        ;;
      janitor-run)
        if [ "$#" -ne 8 ]; then
          echo "ERROR: usage: janitor-run <parent-pid> <run-id> <pid> <pgid> <workflow-id> <task-id> <control-file> <signal-file>"
          exit "$NIXFIED_EXIT_USAGE"
        fi
        janitor_run_loop "$@"
        ;;
      *)
        echo "ERROR: unknown subcommand '$subcommand'"
        exit "$NIXFIED_EXIT_USAGE"
        ;;
    esac
  }

  main "$@"
''
