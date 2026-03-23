{
  lib,
  canonical,
  idLib,
}:
{
  resolved,
  runtime,
}:
let
  listUtils = import ../framework/core/list-utils.nix;
  rawTasks = resolved.tasks or { };
  names = builtins.sort builtins.lessThan (builtins.attrNames rawTasks);
  globalRuntimeInputs = map builtins.toString (resolved.tooling.runtimePackages or [ ]);
  excludedServices = resolved.graph.excludedServices or [ ];

  normalizeHooks =
    hooks:
    builtins.mapAttrs (_: hook: {
      command = hook.command;
      runtimeInputs = map builtins.toString hook.runtimeInputs;
      passThroughEnv = hook.passThroughEnv;
      env = hook.env;
      workdir = hook.workdir;
      customWorkdir = hook.customWorkdir;
    }) hooks;

  normalizeId =
    name: rawId:
    let
      effective = if rawId == "" || rawId == name then "task.${idLib.sanitize name}" else rawId;
    in
    idLib.ensurePrefix "task" effective;

  normalizeTaskBase =
    name:
    let
      raw = rawTasks.${name};
      id = normalizeId name raw.id;
      requiredServices = listUtils.uniquePreserveOrder (raw.requirements.services or [ ]);
    in
    canonical.canonicalize {
      inherit id;

      kind = raw.kind;
      requirements = raw.requirements // {
        services = requiredServices;
      };
      summary = raw.summary;
      description = raw.description;
      tags = raw.tags;

      runner = {
        type = raw.runner.type;
        command = raw.runner.command;
        package = if raw.runner.package == null then null else builtins.toString raw.runner.package;
        workflowId = raw.runner.workflowId;
      };

      contract = raw.contract;

      runtime = {
        slotEnv = raw.runtime.slotEnv;
        workdir = raw.runtime.workdir;
        customWorkdir = raw.runtime.customWorkdir;
        hermetic = raw.runtime.hermetic;
        runtimeInputs = listUtils.uniquePreserveOrder (
          map builtins.toString raw.runtime.runtimeInputs ++ globalRuntimeInputs
        );
        passThroughEnv = raw.runtime.passThroughEnv;
        allowSensitivePassThrough = raw.runtime.allowSensitivePassThrough;
        logging = raw.runtime.logging;
        env = raw.runtime.env;
        umask = raw.runtime.umask;
        locale = raw.runtime.locale;
        timezone = raw.runtime.timezone;
        preHooks = normalizeHooks raw.runtime.preHooks;
        postHooks = normalizeHooks raw.runtime.postHooks;
      };

      scheduling = raw.scheduling;
      deps = raw.deps;
      produces = raw.produces;
    };

  addTask =
    acc: name:
    let
      task = normalizeTaskBase name;
      id = task.id;
    in
    if builtins.hasAttr id acc then
      if canonical.toCanonicalNix acc.${id} == canonical.toCanonicalNix task then
        acc
      else
        throw "task id collision for '${id}'"
    else
      acc
      // {
        ${id} = task;
      };

  tasksByIdRaw = builtins.foldl' addTask { } names;

  normalizeTaskDepId =
    depTaskId:
    if depTaskId == "" then
      depTaskId
    else if builtins.hasAttr depTaskId tasksByIdRaw then
      depTaskId
    else
      idLib.ensurePrefix "task" depTaskId;

  tasksById = builtins.mapAttrs (
    _: task:
    let
      normalizedNeeds = listUtils.uniquePreserveOrder (map normalizeTaskDepId (task.deps.needs or [ ]));
      normalizedSoftNeeds = listUtils.uniquePreserveOrder (
        map normalizeTaskDepId (task.deps.softNeeds or [ ])
      );
    in
    canonical.canonicalize (
      task
      // {
        deps = task.deps // {
          needs = normalizedNeeds;
          softNeeds = normalizedSoftNeeds;
        };
      }
    )
  ) tasksByIdRaw;

  ids = builtins.sort builtins.lessThan (builtins.attrNames tasksById);

  _validateDependencies = map (
    taskId:
    let
      task = tasksById.${taskId};
      validateDep =
        depTaskId:
        if builtins.hasAttr depTaskId tasksById then
          true
        else
          throw "task '${taskId}' depends on unknown task '${depTaskId}'";
    in
    map validateDep ((task.deps.needs or [ ]) ++ (task.deps.softNeeds or [ ]))
  ) ids;

  initialPruneReasons = builtins.listToAttrs (
    builtins.concatLists (
      map (
        taskId:
        let
          task = tasksById.${taskId};
          excludedRequirements = builtins.filter (
            serviceName: builtins.elem serviceName excludedServices
          ) task.requirements.services;
        in
        if excludedRequirements != [ ] then
          [
            {
              name = taskId;
              value = {
                reason = "service-excluded";
                serviceName = builtins.head excludedRequirements;
                serviceNames = excludedRequirements;
              };
            }
          ]
        else
          [ ]
      ) ids
    )
  );

  pruneTasksUntilStable =
    pruneReasonsByTaskId:
    let
      nextPruneReasons = builtins.listToAttrs (
        builtins.concatLists (
          map (
            taskId:
            if builtins.hasAttr taskId pruneReasonsByTaskId then
              [ ]
            else
              let
                task = tasksById.${taskId};
                blockingNeeds = builtins.filter (depTaskId: builtins.hasAttr depTaskId pruneReasonsByTaskId) (
                  task.deps.needs or [ ]
                );
              in
              if blockingNeeds == [ ] then
                [ ]
              else
                [
                  {
                    name = taskId;
                    value = {
                      reason = "required-task-pruned";
                      dependency = builtins.head blockingNeeds;
                    };
                  }
                ]
          ) ids
        )
      );
    in
    if nextPruneReasons == { } then
      pruneReasonsByTaskId
    else
      pruneTasksUntilStable (pruneReasonsByTaskId // nextPruneReasons);

  pruneReasonsByTaskId = pruneTasksUntilStable initialPruneReasons;
  prunedTaskIds = builtins.sort builtins.lessThan (builtins.attrNames pruneReasonsByTaskId);
  survivingTasks = lib.removeAttrs tasksById prunedTaskIds;
in
{
  allTasks = tasksById;
  tasks = survivingTasks;
  declaredTaskIds = ids;
  prunedTaskIds = prunedTaskIds;
  pruneReasonsByTaskId = pruneReasonsByTaskId;
}
