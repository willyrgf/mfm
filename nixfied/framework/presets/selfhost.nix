{
  mkCommandTask,
  commonRuntimeInputs,
  ownerFile ? "nixfied/framework/presets/selfhost.nix",
}:
{
  tasks = {
    test-framework-selfhost =
      mkCommandTask {
        id = "task.test.framework.selfhost";
        appName = "test-framework-selfhost";
        kind = "internal";
        summary = "Framework self-host smoke command";
        description = "Runs framework commands through the built executor from the current evaluation closure.";
        runtimeInputs = commonRuntimeInputs;
        command = ''
          set -euo pipefail
          logs_dir="$(mktemp -d "''${TMPDIR:-/tmp}/framework-selfhost.XXXXXX")"
          task_dev_log="$logs_dir/task-dev.log"
          workflow_log="$logs_dir/workflow-ci-basic.log"
          cleanup() {
            local rc=$?
            if [ "$rc" -eq 0 ]; then
              rm -rf "$logs_dir"
            else
              echo "ERROR: self-host logs preserved dir=$logs_dir"
              echo "INFO: self-host task.dev log path=$task_dev_log"
              echo "INFO: self-host workflow.ci.basic log path=$workflow_log"
            fi
            return "$rc"
          }
          trap cleanup EXIT
          if [ -z "''${NIXFIED_EXECUTOR_SELF:-}" ]; then
            echo "ERROR: NIXFIED_EXECUTOR_SELF is not set"
            exit 3
          fi
          echo "INFO: self-host smoke start"
          echo "INFO: self-host logs dir=$logs_dir"
          if NIXFIED_CALLER_PWD="$PWD" "$NIXFIED_EXECUTOR_SELF" run-task task.dev >"$task_dev_log" 2>&1; then
            echo "OK: self-host task.dev completed"
          else
            rc="$?"
            echo "ERROR: self-host task.dev failed rc=$rc"
            cat "$task_dev_log"
            exit "$rc"
          fi
          if NIXFIED_CALLER_PWD="$PWD" "$NIXFIED_EXECUTOR_SELF" run-workflow workflow.ci.basic --summary >"$workflow_log" 2>&1; then
            echo "OK: self-host workflow.ci.basic completed"
          else
            rc="$?"
            echo "ERROR: self-host workflow.ci.basic failed rc=$rc"
            cat "$workflow_log"
            exit "$rc"
          fi
          echo "OK: self-host smoke complete"
        '';
        inherit ownerFile;
      }
      // {
        ui.app.expose = false;
      };
  };

  workflows = {
    test-framework-selfhost = {
      id = "workflow.test.framework.selfhost";
      summary = "Framework self-host smoke workflow";
      description = "Runs internal self-host command through workflow orchestration.";
      mode = "custom";
      maxWorkers = 1;
      units = {
        main = {
          taskId = "task.test.framework.selfhost";
          needs = [ ];
          locks = [ ];
          when = {
            envEquals = { };
            envPresent = [ ];
          };
          skipIfMissingEnv = [ ];
        };
      };
      stages = [ ];
      preRun.tasks = [ "task.ops.ready" ];
      postRun = {
        tasks = [ "task.ops.health" ];
        alwaysRun = true;
      };
      artifacts = {
        root = "/tmp/ci-artifacts";
        keepOnSuccess = false;
        keepOnFailure = true;
        writeSummary = true;
      };
      execution = {
        parallel = false;
        failFast = true;
        lockPolicy = "exclusive";
        emitRegistryEvents = true;
        ephemeral.enable = true;
      };
    };
  };
}
