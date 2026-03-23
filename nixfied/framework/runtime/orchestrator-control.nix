{
  pkgs,
  model,
  registry ? import ./registry { inherit pkgs; },
}:
let
  lib = pkgs.lib;
  shellCommon = import ../core/shell-common.nix { inherit pkgs; };
  registryShell = registry.events.mkShellLib { };
  orchestratorRuntimeShell = import ./orchestrator-runtime.nix { inherit pkgs; };
in
pkgs.writeShellScriptBin "nixfied-orchestrator-control" ''
  set -euo pipefail
  ${shellCommon}

  REGISTRY_ROOT_DEFAULT=${lib.escapeShellArg model.state.policy.registryRoot}
  REGISTRY_ROOT="''${REGISTRY_ROOT:-$REGISTRY_ROOT_DEFAULT}"
  RUNS_DIR="$REGISTRY_ROOT/orchestrator/runs"
  RUN_LOCKS_DIR="$REGISTRY_ROOT/orchestrator/locks"
  RUN_ID_ACTIVE_ROOT="$REGISTRY_ROOT/active"
  ORCHESTRATOR_STOP_TIMEOUT_SEC_DEFAULT=${lib.escapeShellArg (toString model.runtime.orchestrator.stopTimeoutSec)}

  ${registryShell}
  ${orchestratorRuntimeShell}

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

  deactivate_run_id() {
    local run_id="$1"
    rm -f "$RUN_ID_ACTIVE_ROOT/$run_id"
  }

  is_pid_running() {
    local pid="$1"
    if [ -z "$pid" ] || [ "$pid" = "null" ]; then
      return 1
    fi
    kill -0 "$pid" 2>/dev/null
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

    if [ ! -d "$RUNS_DIR" ]; then
      return 0
    fi

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

  main() {
    if [ "$#" -lt 1 ]; then
      echo "ERROR: usage: nixfied-orchestrator-control <runs|stop-run|stop-all-runs> ..."
      exit "$NIXFIED_EXIT_USAGE"
    fi

    local subcommand="$1"
    shift

    case "$subcommand" in
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
      *)
        echo "ERROR: unknown subcommand '$subcommand'"
        exit "$NIXFIED_EXIT_USAGE"
        ;;
    esac
  }

  main "$@"
''
