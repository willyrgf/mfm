{
  pkgs,
  model,
  registry,
  projectRoot,
}:
let
  lib = pkgs.lib;
  modelFile = pkgs.writeText "nixfied-model.json" (builtins.toJSON model);
  registryShell = registry.events.mkShellLib { };
  workflowModesShell = import ./workflow-modes.nix {
    inherit pkgs;
  };
  executor = import ./executor.nix {
    inherit
      pkgs
      model
      registry
      projectRoot
      ;
  };
  frameworkEphemeral = import ../.framework/ephemeral.nix {
    inherit pkgs;
    project = {
      project = {
        id = model.identity.projectId;
        slotVar = model.runtime.slot.var;
        envVar = model.runtime.env.var;
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

  MODEL_FILE=${lib.escapeShellArg (builtins.toString modelFile)}
  EXECUTOR_PROGRAM=${lib.escapeShellArg "${executor}/bin/nixfied-executor"}
  EPHEMERAL_EXECUTOR_WRAPPER=${lib.escapeShellArg (builtins.toString ephemeralExecutorWrapper)}
  PROJECT_ROOT=${lib.escapeShellArg (builtins.toString projectRoot)}
  REGISTRY_ROOT_DEFAULT=${lib.escapeShellArg model.state.registry.root}
  REGISTRY_ROOT="''${REGISTRY_ROOT:-$REGISTRY_ROOT_DEFAULT}"

  RUNS_DIR="$REGISTRY_ROOT/orchestrator/runs"
  RUN_LOCKS_DIR="$REGISTRY_ROOT/orchestrator/locks"
  RUN_LOG_DIR="$REGISTRY_ROOT/orchestrator/logs"
  RUN_COUNTER_ROOT="$REGISTRY_ROOT/orchestrator/counter"
  SETSID_BIN=${lib.escapeShellArg setsidBin}

  ${registryShell}
  ${workflowModesShell}

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

    parent_pgid="$(ps -o pgid= -p $$ 2>/dev/null | tr -d '[:space:]' || true)"
    pgid="$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d '[:space:]' || true)"

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

  workflow_json() {
    local workflow_id="$1"
    ${pkgs.jq}/bin/jq -c --arg workflowId "$workflow_id" '.workflows[$workflowId] // empty' "$MODEL_FILE"
  }

  task_json() {
    local task_id="$1"
    ${pkgs.jq}/bin/jq -c --arg taskId "$task_id" '.tasks[$taskId] // empty' "$MODEL_FILE"
  }

  contains_item() {
    local needle="$1"
    shift
    local item
    for item in "$@"; do
      if [ "$item" = "$needle" ]; then
        return 0
      fi
    done
    return 1
  }

  validate_log_level_value() {
    local value="$1"
    case "$value" in
      error|warn|info|debug|trace)
        return 0
        ;;
      *)
        echo "ERROR: invalid --log-level '$value' (expected: error|warn|info|debug|trace)"
        return 2
        ;;
    esac
  }

  validate_output_mode_value() {
    local value="$1"
    case "$value" in
      stdout|logs|both)
        return 0
        ;;
      *)
        echo "ERROR: invalid --output-mode '$value' (expected: stdout|logs|both)"
        return 2
        ;;
    esac
  }

  split_process_mode() {
    PROCESS_MODE="fg"
    FORWARD_ARGS=()

    local parse_opts=1
    local arg

    while [ "$#" -gt 0 ]; do
      arg="$1"
      shift

      if [ "$parse_opts" -eq 1 ]; then
        case "$arg" in
          --bg)
            PROCESS_MODE="bg"
            continue
            ;;
          --fg)
            PROCESS_MODE="fg"
            continue
            ;;
          --)
            parse_opts=0
            FORWARD_ARGS+=("--")
            continue
            ;;
        esac
      fi

      FORWARD_ARGS+=("$arg")
    done
  }

  validate_workflow_args() {
    local workflow_id="$1"
    shift

    local parse_opts=1
    local arg
    local mode_value
    local shorthand_mode

    while [ "$#" -gt 0 ]; do
      arg="$1"
      shift

      if [ "$parse_opts" -eq 0 ]; then
        continue
      fi

      case "$arg" in
        --)
          parse_opts=0
          ;;
        --mode)
          if [ "$#" -lt 1 ]; then
            echo "ERROR: --mode requires a value"
            return 2
          fi
          mode_value="$1"
          shift
          workflow_resolve_mode_id "$workflow_id" "$mode_value" >/dev/null || return $?
          ;;
        --mode=*)
          mode_value="''${arg#--mode=}"
          workflow_resolve_mode_id "$workflow_id" "$mode_value" >/dev/null || return $?
          ;;
        --summary)
          ;;
        --log-level)
          if [ "$#" -lt 1 ]; then
            echo "ERROR: --log-level requires a value"
            return 2
          fi
          validate_log_level_value "$1" || return $?
          shift
          ;;
        --log-level=*)
          validate_log_level_value "''${arg#--log-level=}" || return $?
          ;;
        --output-mode)
          if [ "$#" -lt 1 ]; then
            echo "ERROR: --output-mode requires a value"
            return 2
          fi
          validate_output_mode_value "$1" || return $?
          shift
          ;;
        --output-mode=*)
          validate_output_mode_value "''${arg#--output-mode=}" || return $?
          ;;
        --*)
          shorthand_mode="''${arg#--}"
          if workflow_simple_shorthand_exists_for_family "$workflow_id" "$shorthand_mode"; then
            continue
          fi
          echo "ERROR: unknown option '$arg' for workflow '$workflow_id'"
          return 2
          ;;
        -*)
          echo "ERROR: unknown option '$arg' for workflow '$workflow_id'"
          return 2
          ;;
        *)
          ;;
      esac
    done
  }

  validate_typed_task_args() {
    local task_id="$1"
    shift

    local task
    local parser
    local allow_unknown
    local has_positional

    local -a flag_longs
    local -a option_longs
    local -a flag_shorts
    local -a option_shorts

    task="$(task_json "$task_id")"
    if [ -z "$task" ]; then
      echo "ERROR: unknown task '$task_id'"
      return 2
    fi

    parser="$(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r '.contract.input.args.parser // "typed"')"
    allow_unknown="$(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r '.contract.input.args.allowUnknown // false')"

    if [ "$parser" != "typed" ] || [ "$allow_unknown" = "true" ]; then
      return 0
    fi

    mapfile -t flag_longs < <(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r '.contract.input.args.spec[]? | select(.kind == "flag" and (.long // null) != null) | .long')
    mapfile -t option_longs < <(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r '.contract.input.args.spec[]? | select(.kind == "option" and (.long // null) != null) | .long')
    mapfile -t flag_shorts < <(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r '.contract.input.args.spec[]? | select(.kind == "flag" and (.short // null) != null) | .short')
    mapfile -t option_shorts < <(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r '.contract.input.args.spec[]? | select(.kind == "option" and (.short // null) != null) | .short')

    has_positional="$(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r 'any(.contract.input.args.spec[]?; .kind == "positional")')"

    local parse_opts=1
    local arg key

    while [ "$#" -gt 0 ]; do
      arg="$1"
      shift

      if [ "$parse_opts" -eq 0 ]; then
        continue
      fi

      if [ "$arg" = "--" ]; then
        parse_opts=0
        continue
      fi

      case "$arg" in
        --*=*)
          key="''${arg%%=*}"
          if contains_item "$key" "''${option_longs[@]}"; then
            continue
          fi
          if contains_item "$key" "''${flag_longs[@]}"; then
            continue
          fi
          echo "ERROR: unknown option '$key' for task '$task_id'"
          return 2
          ;;
        --*)
          if contains_item "$arg" "''${flag_longs[@]}"; then
            continue
          fi
          if contains_item "$arg" "''${option_longs[@]}"; then
            if [ "$#" -lt 1 ]; then
              echo "ERROR: option '$arg' requires a value"
              return 2
            fi
            shift
            continue
          fi
          echo "ERROR: unknown option '$arg' for task '$task_id'"
          return 2
          ;;
        -?*)
          if [ "''${#arg}" -ne 2 ]; then
            echo "ERROR: unknown option '$arg' for task '$task_id'"
            return 2
          fi
          if contains_item "$arg" "''${flag_shorts[@]}"; then
            continue
          fi
          if contains_item "$arg" "''${option_shorts[@]}"; then
            if [ "$#" -lt 1 ]; then
              echo "ERROR: option '$arg' requires a value"
              return 2
            fi
            shift
            continue
          fi
          echo "ERROR: unknown option '$arg' for task '$task_id'"
          return 2
          ;;
        *)
          if [ "$has_positional" = "true" ]; then
            continue
          fi
          echo "ERROR: unexpected positional argument '$arg' for task '$task_id'"
          return 2
          ;;
      esac
    done
  }

  workflow_ephemeral_flag() {
    local workflow_id="$1"
    local workflow
    local explicit
    local mode

    if [ -z "$workflow_id" ]; then
      printf '0'
      return
    fi

    workflow="$(workflow_json "$workflow_id")"
    if [ -z "$workflow" ]; then
      printf '0'
      return
    fi

    explicit="$(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -r 'if .execution.ephemeral.enable == null then "null" else (.execution.ephemeral.enable | tostring) end')"
    if [ "$explicit" = "true" ]; then
      printf '1'
      return
    fi
    if [ "$explicit" = "false" ]; then
      printf '0'
      return
    fi

    mode="$(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -r '.mode // "custom"')"
    case "$mode" in
      ci|test)
        printf '1'
        ;;
      *)
        printf '0'
        ;;
    esac
  }

  workflow_mode_name() {
    local workflow_id="$1"
    local workflow

    workflow="$(workflow_json "$workflow_id")"
    if [ -z "$workflow" ]; then
      printf 'custom'
      return
    fi

    printf '%s' "$(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -r '.mode // "custom"')"
  }

  workflow_artifacts_root() {
    local workflow_id="$1"
    local workflow

    workflow="$(workflow_json "$workflow_id")"
    if [ -z "$workflow" ]; then
      printf '%s' ""
      return
    fi

    printf '%s' "$(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -r '.artifacts.root // empty')"
  }

  resolve_task_workflow_ref() {
    local task_id="$1"
    local task
    local runner_type

    task="$(task_json "$task_id")"
    if [ -z "$task" ]; then
      printf '%s' ""
      return
    fi

    runner_type="$(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r '.runner.type // "shell"')"
    if [ "$runner_type" = "workflowRef" ]; then
      printf '%s' "$(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r '.runner.workflowId // empty')"
      return
    fi

    printf '%s' ""
  }

  ensure_artifacts_root() {
    local run_id="$1"
    local ephemeral_enabled="$2"
    local workflow_id="$3"

    local configured_root
    configured_root="$(workflow_artifacts_root "$workflow_id")"

    if [ "$ephemeral_enabled" = "1" ]; then
      export NIXFIED_EXECUTION_EPHEMERAL=1
      if [ -z "''${CI_ARTIFACTS_DIR:-}" ]; then
        export CI_ARTIFACTS_DIR="$REGISTRY_ROOT/artifacts/$run_id"
      fi
      if ! mkdir -p "$CI_ARTIFACTS_DIR"; then
        echo "ERROR: failed to prepare CI_ARTIFACTS_DIR '$CI_ARTIFACTS_DIR'"
        return 3
      fi
      return 0
    fi

    export NIXFIED_EXECUTION_EPHEMERAL=0
    if [ -z "''${CI_ARTIFACTS_DIR:-}" ] && [ -n "$configured_root" ]; then
      export CI_ARTIFACTS_DIR="$configured_root"
    fi

    if [ -n "''${CI_ARTIFACTS_DIR:-}" ]; then
      mkdir -p "$CI_ARTIFACTS_DIR"
    fi
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

    run_file="$(run_file_for "$run_id")"
    now="$(iso_now)"

    ${pkgs.jq}/bin/jq -cnS \
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
      }' > "$run_file"
  }

  update_run_state() {
    local run_id="$1"
    local state="$2"
    local exit_code_json="$3"
    local stop_reason="$4"
    local pid="$5"
    local pgid="$6"

    local run_file
    local lock_dir
    local tmp
    local now

    run_file="$(run_file_for "$run_id")"
    lock_dir="$(run_lock_for "$run_id")"
    now="$(iso_now)"

    if [ ! -f "$run_file" ]; then
      return 1
    fi

    registry_lock_acquire "$lock_dir"
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
      registry_lock_release "$lock_dir"
      return 1
    fi

    mv "$tmp" "$run_file"
    registry_lock_release "$lock_dir"
    return 0
  }

  terminal_from_events() {
    local run_id="$1"
    local events_file="$REGISTRY_ROOT/events.ndjson"
    local terminal_state
    local exit_code

    if [ ! -f "$events_file" ]; then
      echo "failed 1"
      return
    fi

    terminal_state="$(${pkgs.jq}/bin/jq -r --arg runId "$run_id" '
      select(.runId == $runId and (.state == "passed" or .state == "failed" or .state == "canceled"))
      | .state
    ' "$events_file" | ${pkgs.coreutils}/bin/tail -n 1)"

    if [ -z "$terminal_state" ]; then
      echo "failed 1"
      return
    fi

    case "$terminal_state" in
      passed)
        echo "passed 0"
        ;;
      canceled)
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
        echo "failed $exit_code"
        ;;
      *)
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

    state="$(${pkgs.jq}/bin/jq -r '.state' "$run_file")"
    if [ "$state" != "running" ]; then
      return 0
    fi

    pid="$(${pkgs.jq}/bin/jq -r '.pid // empty' "$run_file")"
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

    if [ -n "$SETSID_BIN" ] && [ -x "$SETSID_BIN" ]; then
      "$SETSID_BIN" "''${cmd[@]}" &
    elif command -v setsid >/dev/null 2>&1; then
      setsid "''${cmd[@]}" &
    else
      "''${cmd[@]}" &
    fi

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
      command_name="$(${pkgs.jq}/bin/jq -r '.command' "$run_file")"
      state="$(${pkgs.jq}/bin/jq -r '.state' "$run_file")"
      mode="$(${pkgs.jq}/bin/jq -r '.process_mode' "$run_file")"
      pid="$(${pkgs.jq}/bin/jq -r '.pid // ""' "$run_file")"
      pgid="$(${pkgs.jq}/bin/jq -r '.pgid // ""' "$run_file")"

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
    state="$(${pkgs.jq}/bin/jq -r '.state' "$run_file")"

    case "$state" in
      passed|failed|canceled)
        echo "SKIP: run already terminal run_id=$run_id state=$state"
        return 0
        ;;
    esac

    pid="$(${pkgs.jq}/bin/jq -r '.pid // empty' "$run_file")"
    pgid="$(${pkgs.jq}/bin/jq -r '.pgid // empty' "$run_file")"

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
      state="$(${pkgs.jq}/bin/jq -r '.state' "$run_file")"
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
    local task
    local workflow_ref=""
    local workflow_mode="custom"
    local execution_mode="task"
    local ephemeral_enabled="0"
    local run_id
    local args_json

    command_started_at="$(iso_now)"
    command_started_epoch="$(date +%s)"

    task="$(task_json "$task_id")"
    if [ -z "$task" ]; then
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
      workflow_mode="$(workflow_mode_name "$workflow_ref")"
      execution_mode="workflow"
      ephemeral_enabled="$(workflow_ephemeral_flag "$workflow_ref")"
    fi

    if [ "$task_id" = "task.ops.test-isolation" ]; then
      ephemeral_enabled=1
    fi

    run_id="$(next_run_id)"
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
    local workflow
    local mode="workflow"
    local run_id
    local args_json
    local ephemeral_enabled

    command_started_at="$(iso_now)"
    command_started_epoch="$(date +%s)"

    workflow="$(workflow_json "$workflow_id")"
    if [ -z "$workflow" ]; then
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
