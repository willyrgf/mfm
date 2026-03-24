{ lib, pkgs }:

let
  shellCommon = import ../../core/shell-common.nix { inherit pkgs; };
in
{
  mkIsolationScript =
    {
      slotVar,
      envVar,
      slotMax,
      maxParallelDefault,
      logsRootBase,
      runTaskId,
      validateTaskId,
      keepLogsSuccess,
      keepLogsFailure,
      slotsShell,
      envsShell,
      runArgsShell,
      runEnvEntriesShell,
      envPattern,
    }:
    ''
            set -euo pipefail
            ${shellCommon}
            slot_var=${lib.escapeShellArg slotVar}
            env_var=${lib.escapeShellArg envVar}
            slot_max=${toString slotMax}
            max_parallel_default=${toString maxParallelDefault}
            logs_root_base=${lib.escapeShellArg logsRootBase}
            run_task_id=${lib.escapeShellArg runTaskId}
            validate_task_id=${lib.escapeShellArg validateTaskId}
            keep_logs_success=${if keepLogsSuccess then "1" else "0"}
            keep_logs_failure=${if keepLogsFailure then "1" else "0"}

            isolation_slots=()
            isolation_envs=()
            run_args=()
            run_env_entries=()
      ${slotsShell}
      ${envsShell}
      ${runArgsShell}
      ${runEnvEntriesShell}
            selected_slot=""
            selected_env=""
            max_parallel_override=""
            effective_max_parallel=""
            executor_bin="''${NIXFIED_EXECUTOR_SELF:-''${NIXFIED_EXECUTOR_BIN:-}}"
            logs_root=""
            run_scope=""
            host_home="''${HOME:-}"
            host_xdg_config_home="''${XDG_CONFIG_HOME:-}"
            host_nix_user_conf_files="''${NIX_USER_CONF_FILES:-}"
            host_nix_user_config_file=""

            if [ -z "$host_nix_user_conf_files" ]; then
              if [ -n "$host_xdg_config_home" ] && [ -r "$host_xdg_config_home/nix/nix.conf" ]; then
                host_nix_user_config_file="$host_xdg_config_home/nix/nix.conf"
              elif [ -n "$host_home" ] && [ -r "$host_home/.config/nix/nix.conf" ]; then
                host_nix_user_config_file="$host_home/.config/nix/nix.conf"
              fi
            fi

            while [ "$#" -gt 0 ]; do
              case "$1" in
                --slot)
                  selected_slot="$(nixfied_require_next_arg --slot "a value" "$@")"
                  shift 2
                  ;;
                --slot=*)
                  selected_slot="''${1#--slot=}"
                  shift
                  ;;
                --env)
                  selected_env="$(nixfied_require_next_arg --env "a value" "$@")"
                  shift 2
                  ;;
                --env=*)
                  selected_env="''${1#--env=}"
                  shift
                  ;;
                --max-parallel)
                  max_parallel_override="$(nixfied_require_next_arg --max-parallel "a value" "$@")"
                  shift 2
                  ;;
                --max-parallel=*)
                  max_parallel_override="''${1#--max-parallel=}"
                  shift
                  ;;
                --)
                  shift
                  break
                  ;;
                *)
                  nixfied_unknown_arg "$1"
                  ;;
              esac
            done

            nixfied_unexpected_positional_args "$@"

            if [ -n "$selected_slot" ] && [ -z "$selected_env" ]; then
              nixfied_exit_usage "--slot requires --env"
            fi

            if [ -n "$selected_env" ] && [ -z "$selected_slot" ]; then
              nixfied_exit_usage "--env requires --slot"
            fi

            if [ -z "$run_task_id" ]; then
              nixfied_exit_precondition "test-isolation runTaskId is empty"
            fi

            if [ -z "$validate_task_id" ]; then
              nixfied_exit_precondition "test-isolation validateTaskId is empty"
            fi

            if [ -z "$executor_bin" ]; then
              nixfied_exit_precondition "test-isolation requires NIXFIED_EXECUTOR_SELF or NIXFIED_EXECUTOR_BIN"
            fi

            effective_max_parallel="$max_parallel_default"
            if [ -n "$max_parallel_override" ]; then
              effective_max_parallel="$max_parallel_override"
            fi
            if [ "''${CI:-}" = "1" ] || [ "''${CI:-}" = "true" ]; then
              effective_max_parallel=1
              echo "INFO: test-isolation forcing maxParallel=1 reason=ci"
            fi

            if ! [[ "$effective_max_parallel" =~ ^[0-9]+$ ]]; then
              nixfied_exit_precondition "test-isolation maxParallel is not an integer: $effective_max_parallel"
            fi

            if [ "$effective_max_parallel" -lt 1 ]; then
              nixfied_exit_precondition "test-isolation maxParallel must be >= 1"
            fi

            if [ -n "$selected_slot" ]; then
              if ! [[ "$selected_slot" =~ ^[0-9]+$ ]]; then
                nixfied_exit_usage "--slot must be an integer"
              fi
              filtered_slots=()
              for slot_value in "''${isolation_slots[@]}"; do
                if [ "$slot_value" = "$selected_slot" ]; then
                  filtered_slots+=("$slot_value")
                fi
              done
              isolation_slots=("''${filtered_slots[@]}")
              if [ "''${#isolation_slots[@]}" -eq 0 ]; then
                nixfied_exit_precondition "selected slot is not in the isolation matrix: $selected_slot"
              fi
            fi

            if [ -n "$selected_env" ]; then
              filtered_envs=()
              for env_value in "''${isolation_envs[@]}"; do
                if [ "$env_value" = "$selected_env" ]; then
                  filtered_envs+=("$env_value")
                fi
              done
              isolation_envs=("''${filtered_envs[@]}")
              if [ "''${#isolation_envs[@]}" -eq 0 ]; then
                nixfied_exit_precondition "selected environment is not in the isolation matrix: $selected_env"
              fi
            fi

            if [ "''${#isolation_slots[@]}" -eq 0 ]; then
              nixfied_exit_precondition "test-isolation matrix has no slots"
            fi

            if [ "''${#isolation_envs[@]}" -eq 0 ]; then
              nixfied_exit_precondition "test-isolation matrix has no environments"
            fi

            run_scope="''${NIXFIED_RUN_ID:-run-$(${pkgs.coreutils}/bin/date -u +%Y%m%d-%H%M%S)-$$}"
            logs_root="$logs_root_base/$run_scope"
            mkdir -p "$logs_root"
            echo "INFO: test-isolation matrix slots=''${#isolation_slots[@]} envs=''${#isolation_envs[@]} max_parallel=$effective_max_parallel"
            echo "INFO: test-isolation logs_root=$logs_root"

            statuses_dir="$(mktemp -d "$logs_root/.status.XXXXXX")"
            semaphore_dir="$(mktemp -d "$logs_root/.semaphore.XXXXXX")"
            semaphore_fifo="$semaphore_dir/tokens.fifo"
            mkfifo "$semaphore_fifo"
            exec 9<>"$semaphore_fifo"
            rm -f "$semaphore_fifo"

            run_isolated_executor_command() {
              unset \
                NIXFIED_ORCHESTRATOR_RUN_ID \
                NIXFIED_ORCHESTRATOR_ATTEMPT_ID \
                NIXFIED_ORCHESTRATOR_RUN_SUFFIX_REASON \
                NIXFIED_ORCHESTRATOR_PROCESS_MODE \
                NIXFIED_ORCHESTRATOR_WORKFLOW_ID \
                NIXFIED_WORKFLOW_SETUP_STARTED_AT \
                NIXFIED_WORKFLOW_SETUP_STARTED_EPOCH \
                NIXFIED_RUN_ID \
                NIXFIED_ATTEMPT_ID \
                NIXFIED_PARENT_WORKFLOW_ID \
                NIXFIED_TASK_ID \
                NIXFIED_WORKFLOW_NESTED \
                NIXFIED_RUN_ID_FILE_OVERRIDE \
                NIXFIED_SUMMARY_FILE_OVERRIDE || true
              NIXFIED_CALLER_PWD="$PWD" "$executor_bin" "$@"
            }

            token_count=0
            while [ "$token_count" -lt "$effective_max_parallel" ]; do
              printf 'token\n' >&9
              token_count=$((token_count + 1))
            done

            total=0
            failed=0
            worker_pids=()
            status_files=()

            for slot_value in "''${isolation_slots[@]}"; do
              if ! [[ "$slot_value" =~ ^[0-9]+$ ]]; then
                echo "ERROR: matrix slot is not an integer: $slot_value"
                failed=$((failed + 1))
                continue
              fi
              if [ "$slot_value" -gt "$slot_max" ]; then
                echo "ERROR: matrix slot exceeds max slot ($slot_max): $slot_value"
                failed=$((failed + 1))
                continue
              fi

              for env_value in "''${isolation_envs[@]}"; do
                total=$((total + 1))

                case "$env_value" in
                  ${envPattern})
                    ;;
                  *)
                    echo "ERROR: unsupported environment in matrix: $env_value"
                    failed=$((failed + 1))
                    continue
                    ;;
                esac

                cell_name="slot-''${slot_value}__env-''${env_value}"
                cell_dir="$logs_root/$cell_name"
                registry_dir="$cell_dir/registry"
                artifacts_dir="$cell_dir/artifacts"
                validate_log="$cell_dir/validate.log"
                run_log="$cell_dir/run.log"
                validate_run_id_file="$cell_dir/validate.run-id"
                run_id_file="$cell_dir/run.run-id"
                summary_file="$cell_dir/summary.json"
                status_file="$statuses_dir/$cell_name.rc"

                mkdir -p "$cell_dir" "$registry_dir" "$artifacts_dir"
                echo "INFO: isolation cell start slot=$slot_value env=$env_value"
                status_files+=("$status_file")

                IFS= read -r -u 9 _
                (
                  set +e
                  rc=1
                  runtime_root="$cell_dir/runtime"
                  services_root="$cell_dir/services"
                  mkdir -p \
                    "$runtime_root/home" \
                    "$runtime_root/tmp" \
                    "$runtime_root/xdg/data" \
                    "$runtime_root/xdg/state" \
                    "$runtime_root/xdg/cache" \
                    "$services_root"
                  export "$slot_var=$slot_value"
                  export "$env_var=$env_value"
                  export HOME="$runtime_root/home"
                  export TMPDIR="$runtime_root/tmp"
                  export XDG_DATA_HOME="$runtime_root/xdg/data"
                  export XDG_STATE_HOME="$runtime_root/xdg/state"
                  export XDG_CACHE_HOME="$runtime_root/xdg/cache"
                  if [ -n "$host_nix_user_conf_files" ]; then
                    export NIX_USER_CONF_FILES="$host_nix_user_conf_files"
                  elif [ -n "$host_nix_user_config_file" ]; then
                    # Preserve user-scoped Nix client config after HOME is redirected.
                    export NIX_USER_CONF_FILES="$host_nix_user_config_file"
                  fi
                  export REGISTRY_ROOT="$registry_dir"
                  export CI_ARTIFACTS_DIR="$artifacts_dir"
                  export NIXFIED_SERVICE_ROOT="$services_root"
                  export NIXFIED_RUNTIME_HOME="$HOME"
                  export NIXFIED_RUNTIME_TMPDIR="$TMPDIR"
                  export NIXFIED_RUNTIME_XDG_DATA_HOME="$XDG_DATA_HOME"
                  export NIXFIED_RUNTIME_XDG_STATE_HOME="$XDG_STATE_HOME"
                  export NIXFIED_RUNTIME_XDG_CACHE_HOME="$XDG_CACHE_HOME"
                  export NIXFIED_RUNTIME_REGISTRY_ROOT="$registry_dir"
                  export NIXFIED_RUNTIME_ARTIFACTS_DIR="$artifacts_dir"
                  export NIXFIED_RUNTIME_SERVICE_ROOT="$services_root"

                  for run_env_entry in "''${run_env_entries[@]}"; do
                    run_env_key="''${run_env_entry%%$'\t'*}"
                    run_env_value="''${run_env_entry#*$'\t'}"
                    export "$run_env_key=$run_env_value"
                  done

                  run_isolated_executor_command run-task "$validate_task_id" --run-id-file "$validate_run_id_file" > "$validate_log" 2>&1
                  rc="$?"
                  if [ "$rc" -eq 0 ]; then
                    run_isolated_executor_command run-task "$run_task_id" "''${run_args[@]}" --run-id-file "$run_id_file" --summary-file "$summary_file" > "$run_log" 2>&1
                    rc="$?"
                  fi

                  printf '%s\n' "$rc" > "$status_file"
                  if [ "$rc" -eq 0 ]; then
                    echo "OK: isolation cell passed slot=$slot_value env=$env_value"
                    if [ "$keep_logs_success" -eq 0 ]; then
                      rm -rf "$cell_dir"
                    fi
                  else
                    echo "ERROR: isolation cell failed slot=$slot_value env=$env_value rc=$rc"
                  fi

                  printf 'token\n' >&9
                  exit 0
                ) &
                worker_pids+=("$!")
              done
            done

            for worker_pid in "''${worker_pids[@]}"; do
              wait "$worker_pid" || true
            done

            exec 9>&-
            exec 9<&-
            rm -rf "$semaphore_dir"

            for status_file in "''${status_files[@]}"; do
              if [ ! -f "$status_file" ]; then
                failed=$((failed + 1))
                echo "ERROR: isolation cell status missing file=$status_file"
                continue
              fi

              rc="$(cat "$status_file")"
              if [ "$rc" != "0" ]; then
                failed=$((failed + 1))
              fi
            done
            rm -rf "$statuses_dir"

            if [ "$total" -eq 0 ]; then
              nixfied_exit_precondition "test-isolation matrix did not execute any cells"
            fi

            if [ "$failed" -ne 0 ]; then
              echo "ERROR: test-isolation completed with failures failed=$failed total=$total"
              if [ "$keep_logs_failure" -eq 0 ]; then
                rm -rf "$logs_root"
              fi
              exit "$NIXFIED_EXIT_GENERIC"
            fi

            if [ "$keep_logs_success" -eq 1 ]; then
              echo "INFO: isolation logs preserved at $logs_root"
            else
              rm -rf "$logs_root"
            fi
            echo "OK: test-isolation completed total=$total"
    '';
}
