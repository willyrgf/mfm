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
      echo "ERROR: unknown task '$task_id'"
      return 2
    fi

    detail_json="$(${pkgs.jq}/bin/jq -cn --arg mode "task" '{mode: $mode}')"
    append_event "$run_id" "$workflow_id" "$task_id" "queued" "$detail_json"
    append_event "$run_id" "$workflow_id" "$task_id" "running" '{}'

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

    while IFS= read -r unit_json; do
      local unit_name
      local unit_task
      local skip=0
      local missing=""

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
        continue
      fi

      set +e
      execute_task "$run_id" "$workflow_id" "$unit_task" "''${passthrough_args[@]}"
      status="$?"
      set -e

      if [ "$status" -ne 0 ] && [ "$fail_fast" = "true" ]; then
        break
      fi
    done < <(printf '%s' "$workflow" | ${pkgs.jq}/bin/jq -c '.plan[]')

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
