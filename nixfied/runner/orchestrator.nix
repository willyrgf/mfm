{
  pkgs,
  model,
  registry,
  projectRoot,
}:
let
  lib = pkgs.lib;
  registryShell = registry.events.mkShellLib { };
  workflowModesShell = import ./workflow-modes.nix {
    inherit
      pkgs
      model
      ;
  };
  orchestratorRuntimeShell = import ./orchestrator-runtime.nix { inherit pkgs; };
  executor = import ./executor.nix {
    inherit
      pkgs
      model
      registry
      projectRoot
      ;
  };
  frameworkEphemeral = import ../.framework/ephemeral.nix {
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
      if [ "$#" -lt 1 ]; then
        echo "ERROR: missing command for ephemeral wrapper"
        exit 2
      fi
      export NIXFIED_CALLER_PWD="$(pwd -P)"
      exec "$@"
    '';
  };
  setsidBin = if pkgs ? util-linux then "${pkgs.util-linux}/bin/setsid" else "";
in
pkgs.writeShellScriptBin "nixfied-orchestrator" ''
  set -euo pipefail
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
  RUN_COUNTER_ROOT="$REGISTRY_ROOT/orchestrator/counter"
  SETSID_BIN=${lib.escapeShellArg setsidBin}

  ${registryShell}
  ${workflowModesShell}
  ${orchestratorRuntimeShell}

  mkdir -p "$RUNS_DIR" "$RUN_LOCKS_DIR" "$RUN_LOG_DIR" "$RUN_COUNTER_ROOT"

  sha256_text() {
    printf '%s' "$1" | ${pkgs.coreutils}/bin/sha256sum | ${pkgs.gawk}/bin/awk '{print $1}'
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

  next_run_id() {
    local seq
    local seed
    local digest
    seq="$(registry_next_seq "$RUN_COUNTER_ROOT")"
    seed="orchestrator|${model.identity.evalHash}|$seq|$$|$RANDOM|$(iso_now)"
    digest="$(sha256_text "$seed")"
    printf 'run-%s' "''${digest:0:24}"
  }

  create_run_record() {
    local run_id="$1"
    local command_name="$2"
    local workflow_id="$3"
    local task_id="$4"
    local execution_mode="$5"
    local process_mode="$6"
    local ephemeral_enabled="$7"
    local args_json="$8"

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
    return 0
  }

  terminal_from_events() {
    local run_id="$1"
    local events_file
    local terminal_state
    local exit_code

    events_file="$(registry_events_snapshot "$REGISTRY_ROOT")" || {
      echo "failed 1"
      return
    }

    if [ -z "$events_file" ] || [ ! -f "$events_file" ]; then
      echo "failed 1"
      return
    fi

    terminal_state="$(${pkgs.jq}/bin/jq -r --arg runId "$run_id" '
      select(.runId == $runId and (.state == "passed" or .state == "failed" or .state == "canceled"))
      | .state
    ' "$events_file" | ${pkgs.coreutils}/bin/tail -n 1)"

    if [ -z "$terminal_state" ]; then
      registry_snapshot_cleanup "$events_file"
      echo "failed 1"
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
        exit_code="$(${pkgs.jq}/bin/jq -r --arg runId "$run_id" '
          select(.runId == $runId and .state == "failed")
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
        echo "failed 1"
        ;;
    esac
  }

  refresh_one_run() {
    local run_id="$1"
    local run_file
    local state
    local pid
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

    read -r terminal_state terminal_code <<<"$(terminal_from_events "$run_id")"
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

  launch_command() {
    local run_id="$1"
    local process_mode="$2"
    shift 2

    local -a cmd
    local pid
    local pgid
    local rc
    local log_file

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

    "''${cmd[@]}" &

    pid="$!"
    pgid="$(pgid_of_pid "$pid")"
    update_run_state "$run_id" "running" "null" "" "$pid" "$pgid"

    set +e
    wait "$pid"
    rc="$?"
    set -e

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
      return 2
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
    local waited=0

    run_file="$(run_file_for "$run_id")"
    if [ ! -f "$run_file" ]; then
      echo "ERROR: unknown run '$run_id'"
      return 2
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

    if [ -n "$pgid" ] && [ "$pgid" != "0" ]; then
      kill -TERM -- "-$pgid" 2>/dev/null || true
    elif [ -n "$pid" ]; then
      kill -TERM "$pid" 2>/dev/null || true
    fi

    while [ $waited -lt 5 ]; do
      if ! is_pid_running "$pid"; then
        break
      fi
      sleep 1
      waited=$((waited + 1))
    done

    if is_pid_running "$pid"; then
      if [ -n "$pgid" ] && [ "$pgid" != "0" ]; then
        kill -KILL -- "-$pgid" 2>/dev/null || true
      elif [ -n "$pid" ]; then
        kill -KILL "$pid" 2>/dev/null || true
      fi
    fi

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
      return 2
    fi

    local task_id="$1"
    shift

    local command_started_at
    local command_started_epoch
    local workflow_ref=""
    local execution_mode="task"
    local ephemeral_enabled="0"
    local run_id
    local args_json

    command_started_at="$(iso_now)"
    command_started_epoch="$(date +%s)"

    if ! task_descriptor_exists "$task_id"; then
      echo "ERROR: unknown task '$task_id'"
      return 2
    fi

    workflow_ref="$(resolve_task_workflow_ref "$task_id")"

    split_process_mode "$@"
    if [ -n "$workflow_ref" ]; then
      validate_workflow_args "$workflow_ref" "''${FORWARD_ARGS[@]}"
    else
      validate_typed_task_args "$task_id" "''${FORWARD_ARGS[@]}"
    fi

    if [ -n "$workflow_ref" ]; then
      execution_mode="workflow"
      ephemeral_enabled="$(workflow_ephemeral_flag "$workflow_ref")"
    fi

    if [ "$task_id" = "task.ops.test-isolation" ]; then
      ephemeral_enabled=1
    fi

    run_id="$(next_run_id)"
    if [ -n "$MACHINE_RUN_ID_FILE" ]; then
      write_text_file_atomic "$MACHINE_RUN_ID_FILE" "$run_id"
    fi
    args_json="$(${pkgs.jq}/bin/jq -cn '$ARGS.positional' --args -- "''${FORWARD_ARGS[@]}")"
    create_run_record "$run_id" "run-task" "$workflow_ref" "$task_id" "$execution_mode" "$PROCESS_MODE" "$ephemeral_enabled" "$args_json"

    export NIXFIED_ORCHESTRATOR_RUN_ID="$run_id"
    export NIXFIED_ORCHESTRATOR_PROCESS_MODE="$PROCESS_MODE"
    export NIXFIED_ORCHESTRATOR_WORKFLOW_ID="$workflow_ref"
    export NIXFIED_WORKFLOW_SETUP_STARTED_AT="$command_started_at"
    export NIXFIED_WORKFLOW_SETUP_STARTED_EPOCH="$command_started_epoch"

    ensure_artifacts_root "$run_id" "$ephemeral_enabled" "$workflow_ref"

    if [ "$ephemeral_enabled" = "1" ]; then
      launch_command "$run_id" "$PROCESS_MODE" "$EPHEMERAL_EXECUTOR_WRAPPER" "$EXECUTOR_PROGRAM" run-task "$task_id" "''${FORWARD_ARGS[@]}"
    else
      launch_command "$run_id" "$PROCESS_MODE" "$EXECUTOR_PROGRAM" run-task "$task_id" "''${FORWARD_ARGS[@]}"
    fi
  }

  run_workflow() {
    if [ "$#" -lt 1 ]; then
      echo "ERROR: usage: run-workflow <workflow-id> [-- ...]"
      return 2
    fi

    local workflow_id="$1"
    shift

    local command_started_at
    local command_started_epoch
    local mode="workflow"
    local run_id
    local args_json
    local ephemeral_enabled

    command_started_at="$(iso_now)"
    command_started_epoch="$(date +%s)"

    if ! workflow_id_exists "$workflow_id"; then
      echo "ERROR: unknown workflow '$workflow_id'"
      return 2
    fi

    split_process_mode "$@"
    validate_workflow_args "$workflow_id" "''${FORWARD_ARGS[@]}"

    if [ "''${NIXFIED_WORKFLOW_PARALLEL:-}" = "1" ]; then
      mode="workflow-parallel"
    fi

    ephemeral_enabled="$(workflow_ephemeral_flag "$workflow_id")"
    run_id="$(next_run_id)"
    if [ -n "$MACHINE_RUN_ID_FILE" ]; then
      write_text_file_atomic "$MACHINE_RUN_ID_FILE" "$run_id"
    fi
    args_json="$(${pkgs.jq}/bin/jq -cn '$ARGS.positional' --args -- "''${FORWARD_ARGS[@]}")"
    create_run_record "$run_id" "run-workflow" "$workflow_id" "" "$mode" "$PROCESS_MODE" "$ephemeral_enabled" "$args_json"

    export NIXFIED_ORCHESTRATOR_RUN_ID="$run_id"
    export NIXFIED_ORCHESTRATOR_PROCESS_MODE="$PROCESS_MODE"
    export NIXFIED_ORCHESTRATOR_WORKFLOW_ID="$workflow_id"
    export NIXFIED_WORKFLOW_SETUP_STARTED_AT="$command_started_at"
    export NIXFIED_WORKFLOW_SETUP_STARTED_EPOCH="$command_started_epoch"

    ensure_artifacts_root "$run_id" "$ephemeral_enabled" "$workflow_id"

    if [ "$ephemeral_enabled" = "1" ]; then
      launch_command "$run_id" "$PROCESS_MODE" "$EPHEMERAL_EXECUTOR_WRAPPER" "$EXECUTOR_PROGRAM" run-workflow "$workflow_id" "''${FORWARD_ARGS[@]}"
    else
      launch_command "$run_id" "$PROCESS_MODE" "$EXECUTOR_PROGRAM" run-workflow "$workflow_id" "''${FORWARD_ARGS[@]}"
    fi
  }

  main() {
    if [ "$#" -lt 1 ]; then
      echo "ERROR: usage: nixfied-orchestrator <run-task|run-workflow|runs|stop-run|stop-all-runs> ..."
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
          exit 2
        fi
        ;;
      stop-run)
        if [ "$#" -ne 1 ]; then
          echo "ERROR: usage: stop-run <run-id>"
          exit 2
        fi
        stop_single_run "$1"
        ;;
      stop-all-runs)
        if [ "$#" -ne 0 ]; then
          echo "ERROR: usage: stop-all-runs"
          exit 2
        fi
        stop_all_runs
        ;;
      *)
        echo "ERROR: unknown subcommand '$subcommand'"
        exit 2
        ;;
    esac
  }

  main "$@"
''
