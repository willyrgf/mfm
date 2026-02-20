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
    needs = unit.needs;
    locks = unit.locks;
    when = unit.when;
    skipIfMissingEnv = unit.skipIfMissingEnv;
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

      unitMap =
        if usesUnits then
          builtins.mapAttrs (_: unit: normalizeUnit unit) raw.units
        else
          unitsFromStages raw.stages;

      _validated = validateUnits workflowId unitMap;
      order = topoSort workflowId unitMap;

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
        }
      ) order;
    in
    canonical.canonicalize {
      id = workflowId;
      summary = raw.summary;
      description = raw.description;
      mode = raw.mode;
      maxWorkers = if raw.maxWorkers < 1 then 1 else raw.maxWorkers;
      units = unitMap;
      stages = raw.stages;
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
