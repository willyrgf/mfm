{
  lib,
  canonical,
  idLib,
}:
{
  resolved,
  tasks,
}:
let
  rawWorkflows = resolved.workflows or { };
  names = builtins.sort builtins.lessThan (builtins.attrNames rawWorkflows);

  unique =
    list:
    builtins.foldl' (acc: value: if builtins.elem value acc then acc else acc ++ [ value ]) [ ] list;

  normalizeWorkflowId =
    name: rawId:
    let
      effective = if rawId == "" || rawId == name then "workflow.${idLib.sanitize name}" else rawId;
    in
    idLib.ensurePrefix "workflow" effective;

  normalizeUnit = unit: {
    taskId = unit.taskId;
    needs = unique unit.needs;
    locks = unique unit.locks;
    when = unit.when;
    skipIfMissingEnv = unique unit.skipIfMissingEnv;
  };

  unitsFromStages =
    stages:
    let
      foldStage =
        state: stage:
        let
          stageUnits = builtins.sort builtins.lessThan stage;
          dupes = builtins.filter (item: builtins.elem item state.seen) stageUnits;
          _ =
            if dupes == [ ] then
              true
            else
              throw "workflow stage entries are duplicated: ${builtins.concatStringsSep ", " dupes}";
          unitsForStage = builtins.listToAttrs (
            map (stageEntry: {
              name = stageEntry;
              value = {
                taskId =
                  if builtins.hasAttr stageEntry tasks then stageEntry else idLib.ensurePrefix "task" stageEntry;
                needs = state.previous;
                locks = [ ];
                when = {
                  envEquals = { };
                  envPresent = [ ];
                };
                skipIfMissingEnv = [ ];
              };
            }) stageUnits
          );
        in
        {
          units = state.units // unitsForStage;
          previous = stageUnits;
          seen = state.seen ++ stageUnits;
        };
      folded = builtins.foldl' foldStage {
        units = { };
        previous = [ ];
        seen = [ ];
      } stages;
    in
    folded.units;

  validateUnits =
    workflowId: units:
    let
      unitNames = builtins.sort builtins.lessThan (builtins.attrNames units);

      validateUnit =
        unitName:
        let
          unit = units.${unitName};
          _taskRef =
            if builtins.hasAttr unit.taskId tasks then
              true
            else
              throw "workflow '${workflowId}' references unknown task '${unit.taskId}'";
          _needsRef = map (
            dep:
            if builtins.hasAttr dep units then
              true
            else
              throw "workflow '${workflowId}' unit '${unitName}' depends on unknown unit '${dep}'"
          ) unit.needs;
        in
        true;
    in
    map validateUnit unitNames;

  topoSort =
    workflowId: units:
    let
      unitNames = builtins.sort builtins.lessThan (builtins.attrNames units);

      loop =
        completed: remaining:
        if remaining == [ ] then
          [ ]
        else
          let
            ready = builtins.filter (
              unitName:
              let
                deps = units.${unitName}.needs;
              in
              builtins.all (dep: builtins.elem dep completed) deps
            ) remaining;
            readySorted = builtins.sort builtins.lessThan ready;
          in
          if readySorted == [ ] then
            throw "workflow '${workflowId}' has a dependency cycle"
          else
            readySorted
            ++ loop (completed ++ readySorted) (
              builtins.filter (candidate: !(builtins.elem candidate readySorted)) remaining
            );
    in
    loop [ ] unitNames;

  deriveStages =
    workflowId: units:
    let
      unitNames = builtins.sort builtins.lessThan (builtins.attrNames units);

      loop =
        completed: remaining:
        if remaining == [ ] then
          [ ]
        else
          let
            ready = builtins.filter (
              unitName:
              let
                deps = units.${unitName}.needs;
              in
              builtins.all (dep: builtins.elem dep completed) deps
            ) remaining;
            readySorted = builtins.sort builtins.lessThan ready;
          in
          if readySorted == [ ] then
            throw "workflow '${workflowId}' has a dependency cycle while deriving stages"
          else
            [ readySorted ]
            ++ loop (completed ++ readySorted) (
              builtins.filter (candidate: !(builtins.elem candidate readySorted)) remaining
            );
    in
    loop [ ] unitNames;

  unitNamesByTask =
    units:
    let
      namesByUnit = builtins.sort builtins.lessThan (builtins.attrNames units);
    in
    builtins.foldl' (
      acc: unitName:
      let
        taskId = units.${unitName}.taskId;
        existing = acc.${taskId} or [ ];
      in
      acc
      // {
        ${taskId} = existing ++ [ unitName ];
      }
    ) { } namesByUnit;

  resolveHardTaskDeps =
    workflowId: unitsByTask: unitName: depTaskId:
    let
      depUnits = unitsByTask.${depTaskId} or [ ];
    in
    if depUnits == [ ] then
      throw "workflow '${workflowId}' unit '${unitName}' task dependency '${depTaskId}' is not present in workflow units"
    else
      depUnits;

  resolveSoftTaskDeps =
    unitsByTask: depTaskId:
    unitsByTask.${depTaskId} or [ ];

  mergeTaskMetadata =
    workflowId: units:
    let
      unitsByTask = unitNamesByTask units;
    in
    builtins.mapAttrs (
      unitName: unit:
      let
        task = tasks.${unit.taskId};
        taskScheduling = task.scheduling;
        taskDeps = task.deps;
        taskProduces = task.produces;

        hardNeeds = builtins.concatLists (
          map (depTaskId: resolveHardTaskDeps workflowId unitsByTask unitName depTaskId) (taskDeps.needs or [ ])
        );

        softNeeds = builtins.concatLists (
          map (depTaskId: resolveSoftTaskDeps unitsByTask depTaskId) (taskDeps.softNeeds or [ ])
        );
      in
      unit
      // {
        needs = unique (unit.needs ++ hardNeeds ++ softNeeds);
        locks = unique (unit.locks ++ (taskScheduling.locks or [ ]));
        priority = taskScheduling.priority or 100;
        scheduling = {
          maxAttempts = taskScheduling.maxAttempts or 1;
          retryBackoffSec = taskScheduling.retryBackoffSec or [ ];
          priority = taskScheduling.priority or 100;
        };
        deps = taskDeps;
        produces = taskProduces;
      }
    ) units;

  compileWorkflow =
    name:
    let
      raw = rawWorkflows.${name};
      workflowId = normalizeWorkflowId name raw.id;
      usesUnits = raw.units != { };
      usesStages = raw.stages != [ ];

      _styleCheck =
        if usesUnits && usesStages then
          throw "workflow '${workflowId}' must use exactly one authoring style: units or stages"
        else if (!usesUnits) && (!usesStages) then
          throw "workflow '${workflowId}' must define units or stages"
        else
          true;

      authoredUnits =
        if usesUnits then
          builtins.mapAttrs (_: unit: normalizeUnit unit) raw.units
        else
          unitsFromStages raw.stages;

      unitMap = mergeTaskMetadata workflowId authoredUnits;

      _validated = validateUnits workflowId unitMap;
      order = topoSort workflowId unitMap;

      workflowStages =
        if usesStages then
          map (stage: builtins.sort builtins.lessThan stage) raw.stages
        else
          deriveStages workflowId unitMap;

      plan = map (
        unitName:
        let
          unit = unitMap.${unitName};
        in
        {
          name = unitName;
          taskId = unit.taskId;
          needs = unit.needs;
          locks = unit.locks;
          when = unit.when;
          skipIfMissingEnv = unit.skipIfMissingEnv;
          priority = unit.priority;
          scheduling = unit.scheduling;
          deps = unit.deps;
          produces = unit.produces;
        }
      ) order;
    in
    canonical.canonicalize {
      id = workflowId;
      summary = raw.summary;
      description = raw.description;
      mode = raw.mode;
      maxWorkers = if raw.maxWorkers < 1 then 1 else raw.maxWorkers;
      logging = raw.logging;
      units = unitMap;
      stages = workflowStages;
      preRun = raw.preRun;
      postRun = raw.postRun;
      artifacts = raw.artifacts;
      execution = raw.execution;
      plan = plan;
    };

  addWorkflow =
    acc: name:
    let
      workflow = compileWorkflow name;
      id = workflow.id;
    in
    if builtins.hasAttr id acc then
      if canonical.toCanonicalNix acc.${id} == canonical.toCanonicalNix workflow then
        acc
      else
        throw "workflow id collision for '${id}'"
    else
      acc
      // {
        ${id} = workflow;
      };

  workflowsById = builtins.foldl' addWorkflow { } names;
  ids = builtins.sort builtins.lessThan (builtins.attrNames workflowsById);
in
builtins.listToAttrs (
  map (id: {
    name = id;
    value = workflowsById.${id};
  }) ids
)
