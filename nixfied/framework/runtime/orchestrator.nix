{
  pkgs,
  model,
  services,
  runtimeHash ? model.identity.evalHash,
  registry,
  projectRoot,
  serviceHookEnv ? { },
}:
let
  lib = pkgs.lib;
  shellCommon = import ../core/shell-common.nix { inherit pkgs; };
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
      services
      registry
      projectRoot
      serviceHookEnv
      ;
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
  RUN_COUNTER_ROOT="$REGISTRY_ROOT/orchestrator/counter"
  SETSID_BIN=${lib.escapeShellArg setsidBin}
  ORCHESTRATOR_STOP_TIMEOUT_SEC_DEFAULT=${lib.escapeShellArg (toString model.runtime.orchestrator.stopTimeoutSec)}

  ${registryShell}
  ${workflowModesShell}
  ${orchestratorRuntimeShell}

  mkdir -p "$RUNS_DIR" "$RUN_LOCKS_DIR" "$RUN_LOG_DIR" "$RUN_COUNTER_ROOT"

  FOREGROUND_RUN_ACTIVE=0
  FOREGROUND_RUN_ID=""
  FOREGROUND_RUN_PID=""
  FOREGROUND_RUN_PGID=""
  FOREGROUND_RUN_WORKFLOW_ID=""
  FOREGROUND_RUN_TASK_ID=""
  FOREGROUND_SIGNAL_FILE=""
  FOREGROUND_SIGNAL_NAME=""

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
    local detail_json

    if [ -z "$workflow_id" ] && [ -z "$task_id" ]; then
      return 0
    fi

    detail_json="$(${pkgs.jq}/bin/jq -cn --arg reason "orchestrator-interrupted" --arg signal "$signal_name" '{reason: $reason, signal: $signal}')"
    if [ -n "$workflow_id" ]; then
      registry_append_event "$REGISTRY_ROOT" "$run_id" "$workflow_id" "" "canceled" "$detail_json"
    else
      registry_append_event "$REGISTRY_ROOT" "$run_id" "" "$task_id" "canceled" "$detail_json"
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

  next_run_id() {
    local seq
    local seed
    local digest
    seq="$(registry_next_seq "$RUN_COUNTER_ROOT")"
    seed="orchestrator|${runtimeHash}|$seq|$$|$RANDOM|$(iso_now)"
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
      echo "unknown 1"
      return
    }

    if [ -z "$events_file" ] || [ ! -f "$events_file" ]; then
      echo "unknown 1"
      return
    fi

    terminal_state="$(${pkgs.jq}/bin/jq -r --arg runId "$run_id" '
      select(.runId == $runId and (.state == "passed" or .state == "failed" or .state == "canceled"))
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
        echo "unknown 1"
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

    timeout_seconds="$(stop_timeout_seconds)"
    terminate_run_process "$pid" "$pgid" "$timeout_seconds"

    run_file="$(run_file_for "$run_id")"
    if [ -f "$run_file" ]; then
      current_state="$(run_file_state "$run_file")"
      if ! run_state_is_terminal "$current_state"; then
        read -r terminal_state terminal_code <<<"$(terminal_from_events "$run_id")"
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
    local args_json

    command_started_at="$(iso_now)"
    command_started_epoch="$(date +%s)"

    if ! task_descriptor_exists "$task_id"; then
      echo "ERROR: unknown task '$task_id'"
      return "$NIXFIED_EXIT_USAGE"
    fi

    workflow_ref="$(resolve_task_workflow_ref "$task_id")"

    split_process_mode "$@"
    if task_help_requested "''${FORWARD_ARGS[@]}"; then
      if ! task_print_help "$task_id"; then
        echo "ERROR: unknown task '$task_id'"
        return "$NIXFIED_EXIT_USAGE"
      fi
      return 0
    fi
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
      launch_command "$run_id" "$PROCESS_MODE" "$workflow_ref" "$task_id" "$EPHEMERAL_EXECUTOR_WRAPPER" "$EXECUTOR_PROGRAM" run-task "$task_id" "''${FORWARD_ARGS[@]}"
    else
      launch_command "$run_id" "$PROCESS_MODE" "$workflow_ref" "$task_id" "$EXECUTOR_PROGRAM" run-task "$task_id" "''${FORWARD_ARGS[@]}"
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
    local args_json
    local ephemeral_enabled

    command_started_at="$(iso_now)"
    command_started_epoch="$(date +%s)"

    if ! workflow_id_exists "$workflow_id"; then
      echo "ERROR: unknown workflow '$workflow_id'"
      return "$NIXFIED_EXIT_USAGE"
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
      launch_command "$run_id" "$PROCESS_MODE" "$workflow_id" "" "$EPHEMERAL_EXECUTOR_WRAPPER" "$EXECUTOR_PROGRAM" run-workflow "$workflow_id" "''${FORWARD_ARGS[@]}"
    else
      launch_command "$run_id" "$PROCESS_MODE" "$workflow_id" "" "$EXECUTOR_PROGRAM" run-workflow "$workflow_id" "''${FORWARD_ARGS[@]}"
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
