{
  lib,
  pkgs,
  conf,
  mkCommandTask,
  mkTaskApp,
  ownerFile ? "nixfied/framework/presets/framework-test.nix",
}:
let
  plainShellLogging = import ../core/plain-shell-logging.nix;
  shellCommon = import ../core/shell-common.nix { inherit pkgs; };
  frameworkTestMaxParallelShardsRaw = conf.frameworkTest.maxParallelShards or "auto";
  frameworkTestMaxParallelShards =
    if builtins.isInt frameworkTestMaxParallelShardsRaw then
      toString frameworkTestMaxParallelShardsRaw
    else if builtins.isString frameworkTestMaxParallelShardsRaw then
      frameworkTestMaxParallelShardsRaw
    else
      throw "ERROR: frameworkTest.maxParallelShards must be \"auto\" or a positive integer";
in
{
  tasks = {
    framework-test = mkCommandTask {
      id = "task.framework.test";
      kind = "utility";
      summary = "Run framework validation in the model";
      description = ''
        Runs framework validation shards with configurable shard parallelism.
      '';
      runtimeInputs = [
        pkgs.bash
        pkgs.coreutils
        pkgs.findutils
        pkgs.gnugrep
        pkgs.gnused
        pkgs.nix
      ];
      passThroughEnv = [ "NIXFIED_FRAMEWORK_TEST_FORCE_FAIL_SHARD" ];
      contractArgs = [
        {
          name = "summary";
          kind = "flag";
          long = "--summary";
          description = "Print compact summary output.";
        }
        {
          name = "summary-json";
          kind = "option";
          long = "--summary-json";
          type = "string";
          description = "Write summary JSON to a file.";
        }
        {
          name = "profile";
          kind = "option";
          long = "--profile";
          type = "enum";
          values = [ "ci" ];
          description = "Test profile to run (ci only).";
        }
        {
          name = "shard";
          kind = "option";
          long = "--shard";
          type = "string";
          values = [
            "flake-check"
            "launcher-pruning"
            "help"
            "workflow-ci"
            "services"
            "isolation"
            "self-host"
          ];
          description = "Run one shard only.";
        }
        {
          name = "max-parallel-shards";
          kind = "option";
          long = "--max-parallel-shards";
          type = "string";
          description = "Shard worker cap (positive integer) or 'auto' for all selected shards.";
        }
        {
          name = "serial";
          kind = "flag";
          long = "--serial";
          description = "Force serial shard execution.";
        }
        {
          name = "list-shards";
          kind = "flag";
          long = "--list-shards";
          description = "List available shards and exit.";
        }
        {
          name = "mode";
          kind = "option";
          long = "--mode";
          type = "enum";
          values = [
            "basic"
            "app"
            "env"
            "full"
          ];
          description = "CI workflow mode used by the workflow-ci shard.";
        }
        {
          name = "basic";
          kind = "flag";
          long = "--basic";
          description = "Alias for --mode basic.";
        }
        {
          name = "app";
          kind = "flag";
          long = "--app";
          description = "Alias for --mode app.";
        }
        {
          name = "env";
          kind = "flag";
          long = "--env";
          description = "Alias for --mode env.";
        }
        {
          name = "full";
          kind = "flag";
          long = "--full";
          description = "Alias for --mode full.";
        }
      ];
      command = ''
        set -euo pipefail

        ROOT="$(pwd -P)"
        PROFILE="ci"
        MODE="full"
        SHARD=""
        LIST_SHARDS=0
        SUMMARY=0
        SUMMARY_JSON=""
        MAX_PARALLEL_SHARDS_DEFAULT=${lib.escapeShellArg frameworkTestMaxParallelShards}
        MAX_PARALLEL_SHARDS="$MAX_PARALLEL_SHARDS_DEFAULT"
        SERIAL=0
        SHARDS=(
          "flake-check"
          "launcher-pruning"
          "help"
          "workflow-ci"
          "services"
          "isolation"
          "self-host"
        )
        EXECUTED=0
        FAILED_SHARDS=0
        EXIT_1_SHARDS=0
        CANCELED_SHARDS=0
        STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        START_EPOCH="$(date +%s)"

        ${plainShellLogging { }}
        ${shellCommon}

        usage() {
          cat <<'EOF'
        Usage: nix run .#framework::test [-- --profile ci] [--mode <basic|app|env|full>] [--summary] [--summary-json <path>] [--shard <name>] [--max-parallel-shards <n|auto>] [--serial] [--list-shards]

        Shards:
          flake-check   Build and run the full registered framework check suite.
          launcher-pruning  Build the launcher/runtime split regression checks and help fast paths.
          help          Validate generated help output.
          workflow-ci   Run the CI workflow surface in selected mode.
          services      Build service lifecycle, readiness, and teardown checks.
          isolation     Run isolation checks.
          self-host     Run a workflow that exercises framework entry points.
        EOF
        }

        print_shards() {
          local shard_name
          for shard_name in "''${SHARDS[@]}"; do
            printf '%s\n' "$shard_name"
          done
        }

        shard_exists() {
          local candidate="$1"
          local shard_name
          for shard_name in "''${SHARDS[@]}"; do
            if [ "$candidate" = "$shard_name" ]; then
              return 0
            fi
          done
          return 1
        }

        write_summary_json() {
          local rc="$1"
          local finished_at duration
          local summary_dir summary_tmp
          finished_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
          duration="$(( $(date +%s) - START_EPOCH ))"
          summary_dir="$(dirname "$SUMMARY_JSON")"
          mkdir -p "$summary_dir"
          summary_tmp="$(mktemp "$SUMMARY_JSON.tmp.XXXXXX")"
          cat > "$summary_tmp" <<JSON
        {
          "profile": "$PROFILE",
          "mode": "$MODE",
          "shard": $(if [ -n "$SHARD" ]; then printf '"%s"' "$SHARD"; else printf 'null'; fi),
          "executed_shards": $EXECUTED,
          "failed_shards": $FAILED_SHARDS,
          "exit_1_shards": $EXIT_1_SHARDS,
          "canceled_shards": $CANCELED_SHARDS,
          "exit_code": $rc,
          "duration_seconds": $duration,
          "started_at": "$STARTED_AT",
          "finished_at": "$finished_at"
        }
        JSON
          mv "$summary_tmp" "$SUMMARY_JSON"
          log_info "wrote summary json path=$SUMMARY_JSON"
        }

        run_shard() {
          local shard_name="$1"
          shift
          log_info "running shard=$shard_name"
          if [ -n "''${NIXFIED_FRAMEWORK_TEST_FORCE_FAIL_SHARD:-}" ] && [ "$shard_name" = "$NIXFIED_FRAMEWORK_TEST_FORCE_FAIL_SHARD" ]; then
            log_error "shard failed name=$shard_name rc=17"
            return 17
          fi
          if "$@"; then
            log_ok "shard passed name=$shard_name"
            return 0
          else
            local rc=$?
            log_error "shard failed name=$shard_name rc=$rc"
            return "$rc"
          fi
        }

        shard_flake_check() {
          nix flake check .
        }

        verify_public_launcher_help() {
          local output_file="$1"
          shift
          local rc

          if "$@" >"$output_file" 2>&1; then
            :
          else
            rc="$?"
            log_error "public launcher help command failed rc=$rc"
            cat "$output_file"
            return "$rc"
          fi

          if ! grep -Fq "ci - Run the CI pipeline" "$output_file"; then
            log_error "public launcher help output missing ci summary"
            cat "$output_file"
            return 1
          fi

          if ! grep -Fq "Usage:" "$output_file"; then
            log_error "public launcher help output missing usage block"
            cat "$output_file"
            return 1
          fi

          if grep -Fq "nixfied-selected-app-" "$output_file"; then
            log_error "public launcher help output hit selected-app path"
            cat "$output_file"
            return 1
          fi
        }

        shard_launcher_pruning() {
          local help_out
          help_out="$(mktemp)"

          nix build --no-link \
            .#checks.${pkgs.system}.launcher-surface-contract \
            .#checks.${pkgs.system}.framework-utility-launcher-contract \
            .#checks.${pkgs.system}.service-surface-catalog-contract \
            .#checks.${pkgs.system}.service-api-surface-contract \
            .#checks.${pkgs.system}.framework-install-no-caller-compile-smoke \
            .#checks.${pkgs.system}.framework-test-no-caller-compile-smoke \
            .#checks.${pkgs.system}.framework-upgrade-no-caller-compile-smoke \
            .#checks.${pkgs.system}.runtime-control-launcher-contract \
            .#checks.${pkgs.system}.runtime-controls-no-service-materialization-smoke \
            .#checks.${pkgs.system}.flake-show-no-service-materialization-smoke \
            .#checks.${pkgs.system}.run-id-noise-stability-smoke \
            .#checks.${pkgs.system}.run-id-semantic-inputs-contract \
            .#checks.${pkgs.system}.run-id-active-collision-suffix-smoke \
            .#checks.${pkgs.system}.unselected-service-no-package-resolution-smoke \
            .#checks.${pkgs.system}.unselected-service-public-launcher-smoke \
            .#checks.${pkgs.system}.selected-source-only-resolution-smoke \
            .#checks.${pkgs.system}.disabled-service-no-package-resolution-smoke \
            .#checks.${pkgs.system}.disabled-service-runtime-surface-smoke \
            .#checks.${pkgs.system}.launcher-skip-service-pruning-smoke \
            .#checks.${pkgs.system}.launcher-help-fast-path-smoke \
            .#checks.${pkgs.system}.dispatcher-help-fast-path-smoke \
            .#checks.${pkgs.system}.orchestrator-arg-forwarding-smoke \
            .#checks.${pkgs.system}.service-hook-env-smoke \
            .#checks.${pkgs.system}.runtime-service-selection-contract
          verify_public_launcher_help "$help_out" nix run .#ci -- --help
          verify_public_launcher_help "$help_out" env SKIP_HELIOS=1 nix run .#ci -- --help
          rm -f "$help_out"
        }

        shard_help() {
          local help_stderr
          local rc
          help_stderr="$(mktemp)"
          if nix run .#help >/dev/null 2>"$help_stderr"; then
            rm -f "$help_stderr"
            return 0
          fi
          rc="$?"
          log_error "help shard command failed pwd=$(pwd -P) rc=$rc"
          cat "$help_stderr"
          rm -f "$help_stderr"
          return "$rc"
        }

        shard_workflow_ci() {
          nix run .#run-workflow -- "workflow.ci.$MODE" --summary
        }

        shard_services() {
          nix build --no-link \
            .#checks.${pkgs.system}.managed-service-lifecycle-contract \
            .#checks.${pkgs.system}.service-lifecycle-matrix-smoke \
            .#checks.${pkgs.system}.ready-health-matrix-smoke \
            .#checks.${pkgs.system}.ready-health-shutdown-smoke \
            .#checks.${pkgs.system}.ready-helios-sync-gate-smoke \
            .#checks.${pkgs.system}.supervisor-lifecycle-smoke \
            .#checks.${pkgs.system}.supervisor-runtime-contract
        }

        shard_isolation() {
          local -a isolation_args
          isolation_args=()

          if [ "$SERIAL" -eq 1 ] || [ "''${CI:-}" = "1" ] || [ "''${CI:-}" = "true" ]; then
            isolation_args+=(--max-parallel 1)
          fi

          nix run .#run-task -- task.ops.test-isolation "''${isolation_args[@]}"
        }

        shard_self_host() {
          nix run .#run-workflow -- workflow.test.framework.selfhost --summary
        }

        run_named_shard() {
          local shard_name="$1"
          case "$shard_name" in
            flake-check)
              run_shard "$shard_name" shard_flake_check
              ;;
            launcher-pruning)
              run_shard "$shard_name" shard_launcher_pruning
              ;;
            help)
              run_shard "$shard_name" shard_help
              ;;
            workflow-ci)
              run_shard "$shard_name" shard_workflow_ci
              ;;
            services)
              run_shard "$shard_name" shard_services
              ;;
            isolation)
              run_shard "$shard_name" shard_isolation
              ;;
            self-host)
              run_shard "$shard_name" shard_self_host
              ;;
            *)
              log_error "unknown shard '$shard_name'"
              return "$NIXFIED_EXIT_USAGE"
              ;;
          esac
        }

        run_named_shard_recorded() {
          local shard_name="$1"
          local rc=0
          if run_named_shard "$shard_name"; then
            EXECUTED="$((EXECUTED + 1))"
            return 0
          else
            rc="$?"
            FAILED_SHARDS="$((FAILED_SHARDS + 1))"
            if [ "$rc" -eq 1 ]; then
              EXIT_1_SHARDS="$((EXIT_1_SHARDS + 1))"
            fi
          fi
          return "$rc"
        }

        resolve_parallel_workers() {
          local requested="$1"
          local shard_total="$2"
          local workers="$shard_total"

          if [ "$requested" != "auto" ]; then
            workers="$requested"
          fi

          if [ "$workers" -gt "$shard_total" ]; then
            workers="$shard_total"
          fi

          if [ "$workers" -lt 1 ]; then
            workers=1
          fi

          printf '%s' "$workers"
        }

        run_shards_parallel() {
          local requested_workers="$1"
          shift
          local shard_names=("''${@}")
          local shard_total="''${#shard_names[@]}"
          local workers
          local next_index=0
          local running_count=0
          local done_pid=""
          local done_shard=""
          local wait_rc=0
          local pid
          local failed=0
          local first_rc=1
          local failed_shard=""
          local pending_canceled=0
          local -A PID_TO_SHARD=()
          local -A CANCEL_REQUESTED=()

          if [ "$shard_total" -eq 0 ]; then
            return 0
          fi

          workers="$(resolve_parallel_workers "$requested_workers" "$shard_total")"
          if [ "$workers" -le 1 ]; then
            for shard_name in "''${shard_names[@]}"; do
              run_named_shard_recorded "$shard_name" || return $?
            done
            return 0
          fi

          log_info "running shards parallel workers=$workers total=$shard_total"

          start_shard_worker() {
            local shard_name="$1"
            (
              set +e
              run_named_shard "$shard_name"
            ) &
            pid="$!"
            PID_TO_SHARD[$pid]="$shard_name"
            CANCEL_REQUESTED[$pid]=0
            running_count="$((running_count + 1))"
          }

          cancel_running_shards() {
            local active_pid
            for active_pid in "''${!PID_TO_SHARD[@]}"; do
              CANCEL_REQUESTED[$active_pid]=1
              kill -TERM "$active_pid" 2>/dev/null || true
            done

            sleep "$NIXFIED_RETRY_INTERVAL_DEFAULT"
            for active_pid in "''${!PID_TO_SHARD[@]}"; do
              if kill -0 "$active_pid" 2>/dev/null; then
                kill -KILL "$active_pid" 2>/dev/null || true
              fi
            done
          }

          while [ "$running_count" -lt "$workers" ] && [ "$next_index" -lt "$shard_total" ]; do
            start_shard_worker "''${shard_names[$next_index]}"
            next_index="$((next_index + 1))"
          done

          while [ "''${#PID_TO_SHARD[@]}" -gt 0 ]; do
            if wait -n -p done_pid; then
              wait_rc=0
            else
              wait_rc="$?"
            fi

            done_shard="''${PID_TO_SHARD[$done_pid]:-}"
            if [ -z "$done_shard" ]; then
              continue
            fi

            unset "PID_TO_SHARD[$done_pid]"
            running_count="$((running_count - 1))"

            if [ "''${CANCEL_REQUESTED[$done_pid]:-0}" = "1" ]; then
              CANCELED_SHARDS="$((CANCELED_SHARDS + 1))"
              continue
            fi

            if [ "$wait_rc" -eq 0 ]; then
              EXECUTED="$((EXECUTED + 1))"
            else
              FAILED_SHARDS="$((FAILED_SHARDS + 1))"
              if [ "$wait_rc" -eq 1 ]; then
                EXIT_1_SHARDS="$((EXIT_1_SHARDS + 1))"
              fi
              if [ "$failed" -eq 0 ]; then
                first_rc="$wait_rc"
                failed_shard="$done_shard"
                pending_canceled="$((shard_total - next_index))"
                CANCELED_SHARDS="$((CANCELED_SHARDS + pending_canceled))"
                log_warn "framework::test fail-fast shard=$failed_shard rc=$first_rc pending_canceled=$pending_canceled running_canceled=''${#PID_TO_SHARD[@]}"
                cancel_running_shards
              fi
              failed=1
            fi

            if [ "$failed" -eq 0 ]; then
              while [ "$running_count" -lt "$workers" ] && [ "$next_index" -lt "$shard_total" ]; do
                start_shard_worker "''${shard_names[$next_index]}"
                next_index="$((next_index + 1))"
              done
            fi
          done

          if [ "$failed" -eq 1 ]; then
            return "$first_rc"
          fi
          return 0
        }

        while [ "$#" -gt 0 ]; do
          case "$1" in
            --profile)
              PROFILE="$(nixfied_require_next_arg --profile "a value" "$@")"
              shift 2
              ;;
            --mode)
              MODE="$(nixfied_require_next_arg --mode "a value" "$@")"
              shift 2
              ;;
            --basic|--app|--env|--full)
              MODE="''${1#--}"
              shift
              ;;
            --summary)
              SUMMARY=1
              shift
              ;;
            --summary-json)
              SUMMARY_JSON="$(nixfied_require_next_arg --summary-json "a value" "$@")"
              shift 2
              ;;
            --shard)
              SHARD="$(nixfied_require_next_arg --shard "a value" "$@")"
              shift 2
              ;;
            --max-parallel-shards)
              MAX_PARALLEL_SHARDS="$(nixfied_require_next_arg --max-parallel-shards "a value" "$@")"
              shift 2
              ;;
            --serial)
              SERIAL=1
              shift
              ;;
            --list-shards)
              LIST_SHARDS=1
              shift
              ;;
            --help|-h)
              usage
              exit 0
              ;;
            --)
              shift
              break
              ;;
            *)
              nixfied_unknown_arg_with_usage usage "$1"
              ;;
          esac
        done

        nixfied_unexpected_positional_args_with_usage usage "$@"

        case "$PROFILE" in
          ci)
            ;;
          full)
            log_error "profile 'full' is no longer supported; use --profile ci."
            exit "$NIXFIED_EXIT_USAGE"
            ;;
          *)
            log_error "unknown profile '$PROFILE' (expected: ci)"
            exit "$NIXFIED_EXIT_USAGE"
            ;;
        esac

        case "$MODE" in
          basic|app|env|full)
            ;;
          *)
            log_error "unknown mode '$MODE' (expected: basic|app|env|full)"
            exit "$NIXFIED_EXIT_USAGE"
            ;;
        esac

        case "$MAX_PARALLEL_SHARDS" in
          auto)
            ;;
          *)
            if ! [[ "$MAX_PARALLEL_SHARDS" =~ ^[0-9]+$ ]]; then
              log_error "invalid --max-parallel-shards '$MAX_PARALLEL_SHARDS' (expected: auto|positive-integer)"
              exit "$NIXFIED_EXIT_USAGE"
            fi
            if [ "$MAX_PARALLEL_SHARDS" -lt 1 ]; then
              log_error "invalid --max-parallel-shards '$MAX_PARALLEL_SHARDS' (expected: auto|positive-integer)"
              exit "$NIXFIED_EXIT_USAGE"
            fi
            ;;
        esac

        if [ "$LIST_SHARDS" -eq 1 ]; then
          print_shards
          exit 0
        fi

        if [ -n "$SHARD" ] && ! shard_exists "$SHARD"; then
          log_error "unknown shard '$SHARD'"
          log_info "valid shards: $(print_shards | tr '\n' ' ')"
          exit "$NIXFIED_EXIT_USAGE"
        fi

        cleanup() {
          local rc=$?
          if [ -n "$SUMMARY_JSON" ]; then
            write_summary_json "$rc"
          fi
          return "$rc"
        }
        trap cleanup EXIT

        selected_shards=()
        if [ -n "$SHARD" ]; then
          selected_shards+=("$SHARD")
        else
          selected_shards=("''${SHARDS[@]}")
        fi

        run_rc=0
        if [ "$SERIAL" -eq 1 ]; then
          log_info "running shards serial total=''${#selected_shards[@]}"
          for shard_name in "''${selected_shards[@]}"; do
            if run_named_shard_recorded "$shard_name"; then
              :
            else
              run_rc="$?"
              break
            fi
          done
        elif [ "''${#selected_shards[@]}" -le 1 ]; then
          for shard_name in "''${selected_shards[@]}"; do
            if run_named_shard_recorded "$shard_name"; then
              :
            else
              run_rc="$?"
              break
            fi
          done
        else
          if run_shards_parallel "$MAX_PARALLEL_SHARDS" "''${selected_shards[@]}"; then
            run_rc=0
          else
            run_rc="$?"
          fi
        fi

        if [ "$SUMMARY" -eq 1 ] || [ "$run_rc" -ne 0 ]; then
          log_info "summary profile=$PROFILE mode=$MODE executed_shards=$EXECUTED failed_shards=$FAILED_SHARDS exit_1_shards=$EXIT_1_SHARDS canceled_shards=$CANCELED_SHARDS"
        fi

        if [ "$run_rc" -ne 0 ]; then
          exit "$run_rc"
        fi

        log_ok "framework::test completed"
      '';
      inherit ownerFile;
    };
  };

  apps = {
    "framework::test" = mkTaskApp {
      taskId = "task.framework.test";
      appId = "framework::test";
      category = "framework";
      usage = [
        "nix run .#framework::test"
        "nix run .#framework::test -- --summary"
        "nix run .#framework::test -- --mode env --summary-json /tmp/framework-summary.json"
      ];
      examples = [
        "nix run .#framework::test -- --list-shards"
        "nix run .#framework::test -- --shard flake-check"
        "nix run .#framework::test -- --shard launcher-pruning"
        "nix run .#framework::test -- --shard services"
        "nix run .#framework::test -- --shard isolation"
        "nix run .#framework::test -- --shard self-host"
      ];
      inherit ownerFile;
    };
  };
}
