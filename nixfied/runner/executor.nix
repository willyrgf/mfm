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
  LOG_LEVEL_DEFAULT=${pkgs.lib.escapeShellArg model.runtime.logging.levelDefault}
  OUTPUT_MODE_DEFAULT=${pkgs.lib.escapeShellArg model.runtime.logging.outputDefault}

  # Backward-compat aliases for pre-upgrade env names.
  if [ -n "''${NIXFIED_ENV:-}" ] && [ -z "''${NIX_ENV:-}" ]; then
    export NIX_ENV="$NIXFIED_ENV"
  fi
  if [ -n "''${NIXFIED_LOG_LEVEL:-}" ] && [ -z "''${LOG_LEVEL:-}" ]; then
    export LOG_LEVEL="$NIXFIED_LOG_LEVEL"
  fi
  if [ -n "''${NIXFIED_OUTPUT_MODE:-}" ] && [ -z "''${OUTPUT_MODE:-}" ]; then
    export OUTPUT_MODE="$NIXFIED_OUTPUT_MODE"
  fi

  normalize_log_level() {
    case "$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')" in
      error|warn|info|debug|trace)
        printf '%s' "$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')"
        ;;
      *)
        printf '%s' "info"
        ;;
    esac
  }

  normalize_output_mode() {
    case "$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')" in
      stdout|logs|both)
        printf '%s' "$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')"
        ;;
      *)
        printf '%s' "stdout"
        ;;
    esac
  }

  LOG_LEVEL_EFFECTIVE="$(normalize_log_level "''${LOG_LEVEL:-$LOG_LEVEL_DEFAULT}")"
  OUTPUT_MODE_EFFECTIVE="$(normalize_output_mode "''${OUTPUT_MODE:-$OUTPUT_MODE_DEFAULT}")"

  level_to_rank() {
    case "$1" in
      error)
        printf '%s' "0"
        ;;
      warn)
        printf '%s' "1"
        ;;
      info)
        printf '%s' "2"
        ;;
      debug)
        printf '%s' "3"
        ;;
      trace)
        printf '%s' "4"
        ;;
      *)
        printf '%s' "2"
        ;;
    esac
  }

  should_log() {
    local level="$1"
    [ "$(level_to_rank "$LOG_LEVEL_EFFECTIVE")" -ge "$(level_to_rank "$level")" ]
  }

  log_line() {
    local level="$1"
    shift
    local level_upper
    if should_log "$level"; then
      level_upper="$(printf '%s' "$level" | tr '[:lower:]' '[:upper:]')"
      printf '%s: %s\n' "$level_upper" "$*" >&2
    fi
  }

  log_error() {
    log_line error "$@"
  }

  log_warn() {
    log_line warn "$@"
  }

  log_info() {
    log_line info "$@"
  }

  log_debug() {
    log_line debug "$@"
  }

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

  execute_task() {
    local run_id="$1"
    local workflow_id="$2"
    local task_id="$3"
    shift 3

    local task
    local runner_type
    local command
    local nested_workflow
    local detail_json
    local exit_code

    task="$(task_json "$task_id")"
    if [ -z "$task" ]; then
      log_error "unknown task '$task_id'"
      return 2
    fi

    detail_json="$(${pkgs.jq}/bin/jq -cn --arg mode "task" '{mode: $mode}')"
    append_event "$run_id" "$workflow_id" "$task_id" "queued" "$detail_json"
    append_event "$run_id" "$workflow_id" "$task_id" "running" '{}'

    runner_type="$(printf '%s' "$task" | ${pkgs.jq}/bin/jq -r '.runner.type')"
    log_debug "task start id=$task_id workflow=$workflow_id runId=$run_id runner=$runner_type"

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
          log_error "task '$task_id' runner.workflowId is empty"
          exit_code=3
        else
          log_debug "task id=$task_id delegates workflow=$nested_workflow"
          run_workflow "$nested_workflow" "$@"
          exit_code="$?"
        fi
        ;;
      derivation)
        log_error "derivation runner is not implemented for task '$task_id'"
        exit_code=3
        ;;
      *)
        log_error "unsupported runner type '$runner_type' for task '$task_id'"
        exit_code=3
        ;;
    esac
    set -e

    if [ "$exit_code" -eq 0 ]; then
      append_event "$run_id" "$workflow_id" "$task_id" "passed" '{}'
      log_debug "task passed id=$task_id workflow=$workflow_id runId=$run_id"
    else
      detail_json="$(${pkgs.jq}/bin/jq -cn --argjson exitCode "$exit_code" '{exitCode: $exitCode}')"
      append_event "$run_id" "$workflow_id" "$task_id" "failed" "$detail_json"
      log_debug "task failed id=$task_id workflow=$workflow_id runId=$run_id exitCode=$exit_code"
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
    log_info "run-task start taskId=$task_id runId=$run_id logLevel=$LOG_LEVEL_EFFECTIVE outputMode=$OUTPUT_MODE_EFFECTIVE"

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

    if [ "$status" -eq 0 ]; then
      log_info "run-task passed taskId=$task_id runId=$run_id"
    else
      log_error "run-task failed taskId=$task_id runId=$run_id exitCode=$status"
    fi

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

  validate_ci_mode() {
    local mode_override="$1"
    if [ -z "$mode_override" ]; then
      return 0
    fi
    if ! ${pkgs.jq}/bin/jq -e --arg workflowId "workflow.ci.$mode_override" '.workflows[$workflowId] != null' "$MODEL_FILE" >/dev/null; then
      log_error "unknown mode '$mode_override' (expected: basic|audit|parity|full|mainnet)"
      return 2
    fi
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
        --basic|--audit|--parity|--full|--mainnet)
          mode_override="''${1#--}"
          shift
          ;;
        --app)
          mode_override="audit"
          shift
          ;;
        --env)
          mode_override="parity"
          shift
          ;;
        --summary)
          print_summary=1
          shift
          ;;
        --bg|--background)
          log_warn "--bg/--background is not supported by the model runner; continuing in foreground"
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

    if [[ "$workflow_id" == workflow.ci.* ]]; then
      validate_ci_mode "$mode_override" || return 2
    fi

    workflow_id="$(resolve_workflow_mode "$workflow_id" "$mode_override")"

    local workflow
    local args_payload
    local run_id
    local detail_json
    local fail_fast
    local status=0
    local unit_count=0
    local unit_index=0

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
    unit_count="$(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -r '.plan | length')"

    log_info "run-workflow start workflowId=$workflow_id runId=$run_id units=$unit_count failFast=$fail_fast logLevel=$LOG_LEVEL_EFFECTIVE outputMode=$OUTPUT_MODE_EFFECTIVE"

    while IFS= read -r unit_json; do
      local unit_name
      local unit_task
      local skip=0
      local missing=""
      local step_started
      local step_elapsed

      unit_index="$(( unit_index + 1 ))"

      unit_name="$(printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '.name')"
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
        detail_json="$(${pkgs.jq}/bin/jq -cn --arg reason "missing-env" --arg missing "$missing" '{reason: $reason, missing: $missing}')"
        append_event "$run_id" "$workflow_id" "$unit_task" "canceled" "$detail_json"
        log_warn "workflow unit skipped workflowId=$workflow_id runId=$run_id index=$unit_index/$unit_count name=$unit_name taskId=$unit_task missingEnv=$missing"
        continue
      fi

      log_info "workflow unit start workflowId=$workflow_id runId=$run_id index=$unit_index/$unit_count name=$unit_name taskId=$unit_task"
      step_started="$(${pkgs.coreutils}/bin/date +%s)"

      set +e
      execute_task "$run_id" "$workflow_id" "$unit_task" "''${passthrough_args[@]}"
      status="$?"
      set -e
      step_elapsed="$(( $(${pkgs.coreutils}/bin/date +%s) - step_started ))"

      if [ "$status" -eq 0 ]; then
        log_info "workflow unit passed workflowId=$workflow_id runId=$run_id index=$unit_index/$unit_count name=$unit_name taskId=$unit_task durationSec=$step_elapsed"
      else
        log_error "workflow unit failed workflowId=$workflow_id runId=$run_id index=$unit_index/$unit_count name=$unit_name taskId=$unit_task durationSec=$step_elapsed exitCode=$status"
      fi

      if [ "$status" -ne 0 ] && [ "$fail_fast" = "true" ]; then
        log_warn "workflow fail-fast triggered workflowId=$workflow_id runId=$run_id failedTaskId=$unit_task"
        break
      fi
    done < <(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -c '.plan[]')

    if [ "$status" -eq 0 ]; then
      append_event "$run_id" "$workflow_id" "" "passed" '{}'
    else
      detail_json="$(${pkgs.jq}/bin/jq -cn --argjson exitCode "$status" '{exitCode: $exitCode}')"
      append_event "$run_id" "$workflow_id" "" "failed" "$detail_json"
    fi

    if [ "$status" -eq 0 ]; then
      log_info "run-workflow passed workflowId=$workflow_id runId=$run_id"
    else
      log_error "run-workflow failed workflowId=$workflow_id runId=$run_id exitCode=$status"
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
