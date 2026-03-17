{
  pkgs,
  model,
}:
let
  lib = pkgs.lib;

  uniqueSorted = values: builtins.sort builtins.lessThan (lib.unique values);

  workflows = model.workflows or { };
  tasks = model.tasks or { };

  workflowIds = uniqueSorted (builtins.attrNames workflows);

  workflowModesByFamily = builtins.foldl' (
    acc: workflowId:
    let
      match = builtins.match "^workflow\\.([^.]+)\\.(.+)$" workflowId;
    in
    if match == null then
      acc
    else
      let
        family = builtins.elemAt match 0;
        mode = builtins.elemAt match 1;
        existing = acc.${family} or [ ];
      in
      acc
      // {
        ${family} = uniqueSorted (existing ++ [ mode ]);
      }
  ) { } workflowIds;

  workflowFamilies = uniqueSorted (builtins.attrNames workflowModesByFamily);

  workflowDescriptorById = builtins.listToAttrs (
    map (
      workflowId:
      let
        workflow = workflows.${workflowId};
        execution = workflow.execution or { };
        ephemeral = execution.ephemeral or { };
        artifacts = workflow.artifacts or { };
        logging = workflow.logging or { };
        postRun = workflow.postRun or { };
        mode = workflow.mode or "custom";
        explicitEphemeral =
          if (ephemeral ? enable) && ephemeral.enable != null then ephemeral.enable else null;
        effectiveEphemeral =
          if explicitEphemeral != null then
            explicitEphemeral
          else
            builtins.elem mode [
              "ci"
              "test"
            ];
      in
      {
        name = workflowId;
        value = {
          inherit mode;
          artifactsRoot = artifacts.root or "";
          ephemeralFlag = if effectiveEphemeral then "1" else "0";
          logLevelDefault = logging.levelDefault or "";
          outputModeDefault = logging.outputDefault or "";
          failFast = if execution.failFast or false then "true" else "false";
          parallelEnabled = if execution.parallel or false then "true" else "false";
          maxWorkers = toString (workflow.maxWorkers or 1);
          lockPolicy = execution.lockPolicy or "exclusive";
          writeSummary = if artifacts.writeSummary or false then "true" else "false";
          postRunAlways = if postRun.alwaysRun or false then "true" else "false";
        };
      }
    ) workflowIds
  );

  normalizeTaskArgSpec =
    spec:
    let
      hasLong = (spec ? long) && spec.long != null && spec.long != "";
      hasShort = (spec ? short) && spec.short != null && spec.short != "";
      kind =
        if (spec ? kind) && spec.kind != null then
          spec.kind
        else if hasLong || hasShort then
          "option"
        else
          "positional";
    in
    {
      inherit kind;
      long = if hasLong then spec.long else "";
      short = if hasShort then spec.short else "";
      type = if (spec ? type) && spec.type != null && spec.type != "" then toString spec.type else "";
      values = if (spec ? values) && spec.values != null then map toString spec.values else [ ];
      description = if (spec ? description) && spec.description != null then spec.description else "";
    };

  formatTaskArgHelpLine =
    spec:
    let
      tokens =
        (lib.optionals (spec.short != "") [ spec.short ])
        ++ (lib.optionals (spec.long != "") [ spec.long ]);
      valueLabel =
        if spec.kind != "option" then
          ""
        else if spec.values != [ ] then
          "<${builtins.concatStringsSep "|" spec.values}>"
        else if spec.type != "" then
          "<${spec.type}>"
        else
          "<value>";
      descriptionSuffix = if spec.description != "" then ": ${spec.description}" else "";
    in
    "  ${builtins.concatStringsSep ", " tokens}${
        lib.optionalString (valueLabel != "") " ${valueLabel}"
      }${descriptionSuffix}";

  mergeTaskRuntimeWithRunnerPackage =
    task:
    let
      packagePath = task.runner.package or null;
    in
    task.runtime
    // {
      runtimeInputs =
        (task.runtime.runtimeInputs or [ ])
        ++ lib.optionals (packagePath != null && packagePath != "") [ packagePath ];
    };

  mergeHookRuntime =
    task: hook:
    let
      taskRuntime = task.runtime;
      hookWorkdir = hook.workdir or null;
      hookCustomWorkdir = hook.customWorkdir or null;
      taskCustomWorkdir = taskRuntime.customWorkdir or null;
    in
    {
      slotEnv = taskRuntime.slotEnv;
      workdir = if hookWorkdir == null then taskRuntime.workdir else hookWorkdir;
      customWorkdir =
        if hookCustomWorkdir != null then
          hookCustomWorkdir
        else if hookWorkdir == null then
          taskCustomWorkdir
        else if hookWorkdir == "custom" then
          taskCustomWorkdir
        else
          null;
      hermetic = taskRuntime.hermetic;
      runtimeInputs = (taskRuntime.runtimeInputs or [ ]) ++ (hook.runtimeInputs or [ ]);
      passThroughEnv = (taskRuntime.passThroughEnv or [ ]) ++ (hook.passThroughEnv or [ ]);
      allowSensitivePassThrough = taskRuntime.allowSensitivePassThrough or false;
      env = (taskRuntime.env or { }) // (hook.env or { });
      umask = taskRuntime.umask or "022";
      locale = taskRuntime.locale or "C.UTF-8";
      timezone = taskRuntime.timezone or "UTC";
    };

  taskIds = uniqueSorted (builtins.attrNames tasks);

  taskDescriptorById = builtins.listToAttrs (
    map (
      taskId:
      let
        task = tasks.${taskId};
        ui = task.ui or { };
        app = ui.app or { };
        argsContract = (((task.contract or { }).input or { }).args or { });
        specs = map normalizeTaskArgSpec (argsContract.spec or [ ]);
        packagePath = task.runner.package or null;
        preHookIds = uniqueSorted (builtins.attrNames (task.runtime.preHooks or { }));
        postHookIds = uniqueSorted (builtins.attrNames (task.runtime.postHooks or { }));
        displayName = if (app.expose or false) && (app.name or "") != "" then app.name else taskId;
        usageLines =
          let
            configuredUsage = app.usage or [ ];
          in
          if configuredUsage != [ ] then configuredUsage else [ "nix run .#run-task -- ${taskId} [-- ...]" ];
        exampleLines = app.examples or [ ];
        taskHelpLines = [
          "${displayName} - ${task.summary}"
        ]
        ++ lib.optionals (task.description or "" != "") [
          ""
          task.description
        ]
        ++ [
          ""
          "Usage:"
        ]
        ++ map (line: "  ${line}") usageLines
        ++ [
          ""
          "Options:"
        ]
        ++ map formatTaskArgHelpLine specs
        ++ [
          "  -h, --help: Show this help."
        ]
        ++ lib.optionals (exampleLines != [ ]) [
          ""
          "Examples:"
        ]
        ++ map (line: "  ${line}") exampleLines;
        longKinds = builtins.concatLists (
          map (
            spec:
            lib.optionals (spec.long != "") [
              {
                token = spec.long;
                kind = spec.kind;
              }
            ]
          ) specs
        );
        shortKinds = builtins.concatLists (
          map (
            spec:
            lib.optionals (spec.short != "") [
              {
                token = spec.short;
                kind = spec.kind;
              }
            ]
          ) specs
        );
      in
      {
        name = taskId;
        value = {
          parser = argsContract.parser or "typed";
          allowUnknown = if argsContract.allowUnknown or false then "true" else "false";
          hasPositional = if builtins.any (spec: spec.kind == "positional") specs then "true" else "false";
          hookCount = toString (builtins.length preHookIds + builtins.length postHookIds);
          serviceName = task.serviceName or "";
          runnerCommand = if (task.runner.command or null) == null then "" else task.runner.command;
          runnerPackage = if packagePath == null then "" else packagePath;
          runtimeJson = builtins.toJSON (mergeTaskRuntimeWithRunnerPackage task);
          producesJson = builtins.toJSON {
            artifacts = task.produces.artifacts or [ ];
            stateKeys = task.produces.stateKeys or [ ];
          };
          maxAttempts = toString (
            let
              attempts = task.scheduling.maxAttempts or 1;
            in
            if attempts < 1 then 1 else attempts
          );
          retryBackoffJson = builtins.toJSON (task.scheduling.retryBackoffSec or [ ]);
          needs = task.deps.needs or [ ];
          softNeeds = task.deps.softNeeds or [ ];
          inherit
            preHookIds
            postHookIds
            ;
          inherit
            longKinds
            shortKinds
            ;
          helpLines = taskHelpLines;
          runnerType = task.runner.type or "shell";
          runnerWorkflowId = task.runner.workflowId or "";
        };
      }
    ) taskIds
  );

  renderCaseReturn =
    valueExpr: cases:
    lib.concatStringsSep "\n" (
      map (entry: ''
        ${lib.escapeShellArg entry.key})
          printf '%s' ${lib.escapeShellArg (valueExpr entry)}
          return 0
          ;;
      '') cases
    );

  renderCasePrintLines =
    valuesExpr: cases:
    lib.concatStringsSep "\n" (
      map (
        entry:
        let
          values = valuesExpr entry;
        in
        ''
          ${lib.escapeShellArg entry.key})
            ${
              if values == [ ] then
                ":"
              else
                "printf '%s\\n' " + lib.concatStringsSep " " (map lib.escapeShellArg values)
            }
            return 0
            ;;
        ''
      ) cases
    );

  workflowIdCases = map (workflowId: { key = workflowId; }) workflowIds;

  workflowModeCases = map (family: {
    key = family;
    value = workflowModesByFamily.${family};
  }) workflowFamilies;

  workflowCases = map (workflowId: {
    key = workflowId;
    value = workflowDescriptorById.${workflowId};
  }) workflowIds;

  workflowPlanCases = map (workflowId: {
    key = workflowId;
    value = map builtins.toJSON (workflows.${workflowId}.plan or [ ]);
  }) workflowIds;

  workflowPhaseTaskCases = builtins.concatLists (
    map (
      workflowId:
      let
        workflow = workflows.${workflowId};
        preRun = workflow.preRun or { };
        postRun = workflow.postRun or { };
      in
      [
        {
          key = "${workflowId}:preRun";
          value = preRun.tasks or [ ];
        }
        {
          key = "${workflowId}:postRun";
          value = postRun.tasks or [ ];
        }
      ]
    ) workflowIds
  );

  taskCases = map (taskId: {
    key = taskId;
    value = taskDescriptorById.${taskId};
  }) taskIds;

  taskLongKindCases = builtins.concatLists (
    map (
      taskId:
      map (entry: {
        key = "${taskId}:${entry.token}";
        value = entry.kind;
      }) taskDescriptorById.${taskId}.longKinds
    ) taskIds
  );

  taskShortKindCases = builtins.concatLists (
    map (
      taskId:
      map (entry: {
        key = "${taskId}:${entry.token}";
        value = entry.kind;
      }) taskDescriptorById.${taskId}.shortKinds
    ) taskIds
  );

  taskHookCases = builtins.concatLists (
    map (
      taskId:
      let
        task = tasks.${taskId};
        preHooks = task.runtime.preHooks or { };
        postHooks = task.runtime.postHooks or { };
        mkPhaseCases =
          phase: hooks:
          map (
            hookId:
            let
              hook = hooks.${hookId};
            in
            {
              key = "${taskId}:${phase}:${hookId}";
              value = {
                command = hook.command;
                runtimeJson = builtins.toJSON (mergeHookRuntime task hook);
              };
            }
          ) (uniqueSorted (builtins.attrNames hooks));
      in
      mkPhaseCases "pre" preHooks ++ mkPhaseCases "post" postHooks
    ) taskIds
  );
in
''
    workflow_family_from_id() {
      local workflow_id="$1"
      if [[ "$workflow_id" =~ ^workflow\.([^.]+)\..+$ ]]; then
        printf '%s' "''${BASH_REMATCH[1]}"
        return 0
      fi
      return 1
    }

    workflow_id_exists() {
      local workflow_id="$1"
      case "$workflow_id" in
  ${lib.concatStringsSep "\n" (
    map (entry: ''
      ${lib.escapeShellArg entry.key})
        return 0
        ;;
    '') workflowIdCases
  )}
        *)
          return 1
          ;;
      esac
    }

    workflow_modes_for_family() {
      local workflow_id="$1"
      local family

      family="$(workflow_family_from_id "$workflow_id" || true)"
      if [ -z "$family" ]; then
        return 0
      fi

      case "$family" in
  ${lib.concatStringsSep "\n" (
    map (entry: ''
            ${lib.escapeShellArg entry.key})
      ${lib.concatStringsSep "\n" (
        map (mode: "        printf '%s\\n' ${lib.escapeShellArg mode}") entry.value
      )}
              return 0
              ;;
    '') workflowModeCases
  )}
        *)
          return 0
          ;;
      esac
    }

    workflow_mode_is_simple_shorthand() {
      local mode="$1"
      [[ "$mode" =~ ^[a-z0-9-]+$ ]]
    }

    workflow_mode_name() {
      local workflow_id="$1"
      case "$workflow_id" in
  ${renderCaseReturn (entry: entry.value.mode) workflowCases}
        *)
          printf '%s' "custom"
          return 0
          ;;
      esac
    }

    workflow_artifacts_root() {
      local workflow_id="$1"
      case "$workflow_id" in
  ${renderCaseReturn (entry: entry.value.artifactsRoot) workflowCases}
        *)
          printf '%s' ""
          return 0
          ;;
      esac
    }

    workflow_ephemeral_flag() {
      local workflow_id="$1"
      case "$workflow_id" in
  ${renderCaseReturn (entry: entry.value.ephemeralFlag) workflowCases}
        *)
          printf '0'
          return 0
          ;;
      esac
    }

    workflow_logging_level_default() {
      local workflow_id="$1"
      case "$workflow_id" in
  ${renderCaseReturn (entry: entry.value.logLevelDefault) workflowCases}
        *)
          printf '%s' ""
          return 0
          ;;
      esac
    }

    workflow_logging_output_default() {
      local workflow_id="$1"
      case "$workflow_id" in
  ${renderCaseReturn (entry: entry.value.outputModeDefault) workflowCases}
        *)
          printf '%s' ""
          return 0
          ;;
      esac
    }

    workflow_fail_fast() {
      local workflow_id="$1"
      case "$workflow_id" in
  ${renderCaseReturn (entry: entry.value.failFast) workflowCases}
        *)
          printf '%s' "false"
          return 0
          ;;
      esac
    }

    workflow_parallel_enabled() {
      local workflow_id="$1"
      case "$workflow_id" in
  ${renderCaseReturn (entry: entry.value.parallelEnabled) workflowCases}
        *)
          printf '%s' "false"
          return 0
          ;;
      esac
    }

    workflow_max_workers() {
      local workflow_id="$1"
      case "$workflow_id" in
  ${renderCaseReturn (entry: entry.value.maxWorkers) workflowCases}
        *)
          printf '%s' "1"
          return 0
          ;;
      esac
    }

    workflow_lock_policy() {
      local workflow_id="$1"
      case "$workflow_id" in
  ${renderCaseReturn (entry: entry.value.lockPolicy) workflowCases}
        *)
          printf '%s' "exclusive"
          return 0
          ;;
      esac
    }

    workflow_write_summary() {
      local workflow_id="$1"
      case "$workflow_id" in
  ${renderCaseReturn (entry: entry.value.writeSummary) workflowCases}
        *)
          printf '%s' "false"
          return 0
          ;;
      esac
    }

    workflow_post_run_always() {
      local workflow_id="$1"
      case "$workflow_id" in
  ${renderCaseReturn (entry: entry.value.postRunAlways) workflowCases}
        *)
          printf '%s' "false"
          return 0
          ;;
      esac
    }

    workflow_plan_records() {
      local workflow_id="$1"
      case "$workflow_id" in
  ${lib.concatStringsSep "\n" (
    map (entry: ''
            ${lib.escapeShellArg entry.key})
      ${lib.concatStringsSep "\n" (
        map (record: "        printf '%s\\n' ${lib.escapeShellArg record}") entry.value
      )}
              return 0
              ;;
    '') workflowPlanCases
  )}
        *)
          return 0
          ;;
      esac
    }

    workflow_phase_tasks() {
      local workflow_id="$1"
      local phase_key="$2"
      case "$workflow_id:$phase_key" in
  ${renderCasePrintLines (entry: entry.value) workflowPhaseTaskCases}
        *)
          return 0
          ;;
      esac
    }

    workflow_simple_shorthand_exists_for_family() {
      local workflow_id="$1"
      local candidate="$2"
      local mode

      while IFS= read -r mode; do
        if [ "$mode" = "$candidate" ] && workflow_mode_is_simple_shorthand "$mode"; then
          return 0
        fi
      done < <(workflow_modes_for_family "$workflow_id")

      return 1
    }

    workflow_expected_modes_for_family() {
      local workflow_id="$1"
      local mode
      local expected=""

      while IFS= read -r mode; do
        if [ -z "$expected" ]; then
          expected="$mode"
        else
          expected="$expected|$mode"
        fi
      done < <(workflow_modes_for_family "$workflow_id")

      printf '%s' "$expected"
    }

    workflow_resolve_mode_id() {
      local workflow_id="$1"
      local mode_override="$2"
      local family
      local candidate
      local expected

      if [ -z "$mode_override" ]; then
        printf '%s' "$workflow_id"
        return 0
      fi

      family="$(workflow_family_from_id "$workflow_id" || true)"
      if [ -z "$family" ]; then
        echo "ERROR: workflow '$workflow_id' does not support mode overrides" >&2
        return 2
      fi

      candidate="workflow.$family.$mode_override"
      if workflow_id_exists "$candidate"; then
        printf '%s' "$candidate"
        return 0
      fi

      expected="$(workflow_expected_modes_for_family "$workflow_id")"
      if [ -n "$expected" ]; then
        echo "ERROR: unknown mode '$mode_override' (expected: $expected)" >&2
      else
        echo "ERROR: unknown mode '$mode_override'" >&2
      fi
      return 2
    }

    task_descriptor_exists() {
      local task_id="$1"
      case "$task_id" in
  ${lib.concatStringsSep "\n" (
    map (entry: ''
      ${lib.escapeShellArg entry.key})
        return 0
        ;;
    '') taskCases
  )}
        *)
          return 1
          ;;
      esac
    }

    task_arg_parser() {
      local task_id="$1"
      case "$task_id" in
  ${renderCaseReturn (entry: entry.value.parser) taskCases}
        *)
          return 1
          ;;
      esac
    }

    task_arg_allow_unknown() {
      local task_id="$1"
      case "$task_id" in
  ${renderCaseReturn (entry: entry.value.allowUnknown) taskCases}
        *)
          return 1
          ;;
      esac
    }

    task_arg_has_positional() {
      local task_id="$1"
      case "$task_id" in
  ${renderCaseReturn (entry: entry.value.hasPositional) taskCases}
        *)
          return 1
          ;;
      esac
    }

    task_arg_long_kind() {
      local task_id="$1"
      local token="$2"
      case "$task_id:$token" in
  ${renderCaseReturn (entry: entry.value) taskLongKindCases}
        *)
          return 1
          ;;
      esac
    }

    task_arg_short_kind() {
      local task_id="$1"
      local token="$2"
      case "$task_id:$token" in
  ${renderCaseReturn (entry: entry.value) taskShortKindCases}
        *)
          return 1
          ;;
      esac
    }

    task_help_requested() {
      local arg=""

      while [ "$#" -gt 0 ]; do
        arg="$1"
        shift

        case "$arg" in
          --help|-h)
            return 0
            ;;
          --)
            return 1
            ;;
        esac
      done

      return 1
    }

    task_print_help() {
      local task_id="$1"
      case "$task_id" in
  ${renderCasePrintLines (entry: entry.value.helpLines) taskCases}
        *)
          return 1
          ;;
      esac
    }

    task_runner_type() {
      local task_id="$1"
      case "$task_id" in
  ${renderCaseReturn (entry: entry.value.runnerType) taskCases}
        *)
          printf '%s' "shell"
          return 0
          ;;
      esac
    }

    task_runner_workflow_id() {
      local task_id="$1"
      case "$task_id" in
  ${renderCaseReturn (entry: entry.value.runnerWorkflowId) taskCases}
        *)
          printf '%s' ""
          return 0
          ;;
      esac
    }

    task_service_name() {
      local task_id="$1"
      case "$task_id" in
  ${renderCaseReturn (entry: entry.value.serviceName) taskCases}
        *)
          printf '%s' ""
          return 0
          ;;
      esac
    }

    task_runner_command() {
      local task_id="$1"
      case "$task_id" in
  ${renderCaseReturn (entry: entry.value.runnerCommand) taskCases}
        *)
          printf '%s' ""
          return 0
          ;;
      esac
    }

    task_runner_package() {
      local task_id="$1"
      case "$task_id" in
  ${renderCaseReturn (entry: entry.value.runnerPackage) taskCases}
        *)
          printf '%s' ""
          return 0
          ;;
      esac
    }

    task_runtime_json() {
      local task_id="$1"
      case "$task_id" in
  ${renderCaseReturn (entry: entry.value.runtimeJson) taskCases}
        *)
          return 1
          ;;
      esac
    }

    task_produces_json() {
      local task_id="$1"
      case "$task_id" in
  ${renderCaseReturn (entry: entry.value.producesJson) taskCases}
        *)
          return 1
          ;;
      esac
    }

    task_max_attempts() {
      local task_id="$1"
      case "$task_id" in
  ${renderCaseReturn (entry: entry.value.maxAttempts) taskCases}
        *)
          printf '%s' "1"
          return 0
          ;;
      esac
    }

    task_retry_backoff_json() {
      local task_id="$1"
      case "$task_id" in
  ${renderCaseReturn (entry: entry.value.retryBackoffJson) taskCases}
        *)
          printf '%s' "[]"
          return 0
          ;;
      esac
    }

    task_needs() {
      local task_id="$1"
      case "$task_id" in
  ${renderCasePrintLines (entry: entry.value.needs) taskCases}
        *)
          return 1
          ;;
      esac
    }

    task_soft_needs() {
      local task_id="$1"
      case "$task_id" in
  ${renderCasePrintLines (entry: entry.value.softNeeds) taskCases}
        *)
          return 1
          ;;
      esac
    }

    task_hook_count() {
      local task_id="$1"
      case "$task_id" in
  ${renderCaseReturn (entry: entry.value.hookCount) taskCases}
        *)
          printf '%s' "0"
          return 0
          ;;
      esac
    }

    task_hook_ids() {
      local task_id="$1"
      local phase="$2"
      case "$task_id:$phase" in
  ${renderCasePrintLines (entry: entry.value.preHookIds) (
    map (entry: {
      key = "${entry.key}:pre";
      value = entry.value;
    }) taskCases
  )}
  ${renderCasePrintLines (entry: entry.value.postHookIds) (
    map (entry: {
      key = "${entry.key}:post";
      value = entry.value;
    }) taskCases
  )}
        *)
          return 1
          ;;
      esac
    }

    task_hook_command() {
      local task_id="$1"
      local phase="$2"
      local hook_id="$3"
      case "$task_id:$phase:$hook_id" in
  ${renderCaseReturn (entry: entry.value.command) taskHookCases}
        *)
          return 1
          ;;
      esac
    }

    task_hook_runtime_json() {
      local task_id="$1"
      local phase="$2"
      local hook_id="$3"
      case "$task_id:$phase:$hook_id" in
  ${renderCaseReturn (entry: entry.value.runtimeJson) taskHookCases}
        *)
          return 1
          ;;
      esac
    }
''
