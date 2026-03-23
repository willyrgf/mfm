{
  lib,
  canonical,
  idLib,
}:
{
  resolved,
  tasks,
  serviceSets ? { },
  allTasks ? tasks,
  declaredTaskIds ? builtins.sort builtins.lessThan (builtins.attrNames allTasks),
  prunedTaskIds ? [ ],
  pruneReasonsByTaskId ? { },
}:
let
  listUtils = import ../framework/core/list-utils.nix;
  rawWorkflows = resolved.workflows or { };
  excludedServices = resolved.graph.excludedServices or [ ];
  names = builtins.sort builtins.lessThan (builtins.attrNames rawWorkflows);

  declaredTaskIdSet = builtins.listToAttrs (
    map (taskId: {
      name = taskId;
      value = true;
    }) declaredTaskIds
  );

  prunedTaskIdSet = builtins.listToAttrs (
    map (taskId: {
      name = taskId;
      value = true;
    }) prunedTaskIds
  );

  declaredServiceSetIds = builtins.sort builtins.lessThan (builtins.attrNames serviceSets);
  declaredServiceSetIdSet = builtins.listToAttrs (
    map (serviceSetId: {
      name = serviceSetId;
      value = true;
    }) declaredServiceSetIds
  );

  normalizeWorkflowId =
    name: rawId:
    let
      effective = if rawId == "" || rawId == name then "workflow.${idLib.sanitize name}" else rawId;
    in
    idLib.ensurePrefix "workflow" effective;

  resolveDeclaredTaskId =
    rawTaskId:
    if rawTaskId == "" then
      rawTaskId
    else if builtins.hasAttr rawTaskId declaredTaskIdSet then
      rawTaskId
    else
      idLib.ensurePrefix "task" rawTaskId;

  resolveDeclaredServiceSetId =
    rawServiceSetId:
    if rawServiceSetId == "" then
      rawServiceSetId
    else if builtins.hasAttr rawServiceSetId declaredServiceSetIdSet then
      rawServiceSetId
    else
      idLib.ensurePrefix "service-set" rawServiceSetId;

  isDeclaredTaskId = taskId: builtins.hasAttr taskId declaredTaskIdSet;
  isPrunedTaskId = taskId: builtins.hasAttr taskId prunedTaskIdSet;

  normalizeUnit = unit: {
    taskId = resolveDeclaredTaskId unit.taskId;
    needs = listUtils.uniquePreserveOrder unit.needs;
    locks = listUtils.uniquePreserveOrder unit.locks;
    when = unit.when;
    skipIfMissingEnv = listUtils.uniquePreserveOrder unit.skipIfMissingEnv;
    requirements = unit.requirements or { } // {
      services = listUtils.uniquePreserveOrder (unit.requirements.services or [ ]);
    };
  };

  unitsFromStages =
    stages:
    let
      foldStage =
        state: stage:
        let
          stageUnits = builtins.sort builtins.lessThan stage;
          emptyEntries = builtins.filter (item: item == "") stageUnits;
          dupes = builtins.filter (item: builtins.elem item state.seen) stageUnits;
          unitsForStage = builtins.listToAttrs (
            map (stageEntry: {
              name = stageEntry;
              value = {
                taskId = resolveDeclaredTaskId stageEntry;
                needs = state.previous;
                locks = [ ];
                when = {
                  envEquals = { };
                  envPresent = [ ];
                };
                skipIfMissingEnv = [ ];
                requirements = {
                  services = [ ];
                };
              };
            }) stageUnits
          );
        in
        if emptyEntries != [ ] then
          throw "workflow stage entries must not be empty"
        else if dupes != [ ] then
          throw "workflow stage entries are duplicated: ${builtins.concatStringsSep ", " dupes}"
        else
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
        in
        if unit.taskId == "" then
          throw "workflow '${workflowId}' unit '${unitName}' has an empty taskId"
        else if !(isDeclaredTaskId unit.taskId) then
          throw "workflow '${workflowId}' references unknown task '${unit.taskId}'"
        else if
          !(builtins.all (
            dep:
            if builtins.hasAttr dep units then
              true
            else
              throw "workflow '${workflowId}' unit '${unitName}' depends on unknown unit '${dep}'"
          ) unit.needs)
        then
          false
        else
          true;
    in
    map validateUnit unitNames;

  normalizePhaseTaskRefs =
    workflowId: phaseName: taskIds:
    listUtils.uniquePreserveOrder (
      map (
        rawTaskId:
        let
          taskId = resolveDeclaredTaskId rawTaskId;
        in
        if rawTaskId == "" then
          throw "workflow '${workflowId}' ${phaseName}.tasks references an empty task id"
        else if isDeclaredTaskId taskId then
          taskId
        else
          throw "workflow '${workflowId}' ${phaseName}.tasks references unknown task '${rawTaskId}'"
      ) taskIds
    );

  normalizePhaseServiceSetRefs =
    workflowId: phaseName: entries:
    let
      addEntry =
        acc: rawEntry:
        let
          rawServiceSetId = rawEntry.serviceSetId or "";
          serviceSetId = resolveDeclaredServiceSetId rawServiceSetId;
          serviceSet =
            if serviceSetId != "" && builtins.hasAttr serviceSetId serviceSets then
              serviceSets.${serviceSetId}
            else
              null;
          operation =
            if (rawEntry.operation or null) != null then
              rawEntry.operation
            else if serviceSet != null then
              serviceSet.defaultOperation
            else
              null;
          dedupeKey = "${serviceSetId}:${operation}";
        in
        if rawServiceSetId == "" then
          throw "workflow '${workflowId}' ${phaseName}.serviceSets references an empty serviceSetId"
        else if serviceSet == null then
          throw "workflow '${workflowId}' ${phaseName}.serviceSets references unknown service set '${rawServiceSetId}'"
        else if operation == null then
          throw "workflow '${workflowId}' ${phaseName}.serviceSets could not resolve an operation for '${serviceSetId}'"
        else if builtins.hasAttr dedupeKey acc.seen then
          acc
        else
          {
            seen = acc.seen // {
              ${dedupeKey} = true;
            };
            entries = acc.entries ++ [
              (canonical.canonicalize {
                serviceSetId = serviceSet.id;
                serviceSetName = serviceSet.name;
                operation = operation;
                selectedServices = serviceSet.services.all or [ ];
              })
            ];
          };
      normalized = builtins.foldl' addEntry {
        seen = { };
        entries = [ ];
      } entries;
    in
    normalized.entries;

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

  resolveSoftTaskDeps = unitsByTask: depTaskId: unitsByTask.${depTaskId} or [ ];

  compileWorkflow =
    name:
    let
      raw = rawWorkflows.${name};
      workflowId = normalizeWorkflowId name raw.id;
      usesUnits = raw.units != { };
      usesStages = raw.stages != [ ];

      authoredUnitsRaw =
        if usesUnits && usesStages then
          throw "workflow '${workflowId}' must use exactly one authoring style: units or stages"
        else if (!usesUnits) && (!usesStages) then
          throw "workflow '${workflowId}' must define units or stages"
        else if usesUnits then
          builtins.mapAttrs (_: unit: normalizeUnit unit) raw.units
        else
          unitsFromStages raw.stages;

      authoredUnits =
        let
          _validated = validateUnits workflowId authoredUnitsRaw;
        in
        if builtins.all (value: value) _validated then authoredUnitsRaw else authoredUnitsRaw;

      normalizedPreRunTasks = normalizePhaseTaskRefs workflowId "preRun" (raw.preRun.tasks or [ ]);
      normalizedPostRunTasks = normalizePhaseTaskRefs workflowId "postRun" (raw.postRun.tasks or [ ]);
      normalizedPreRunServiceSets = normalizePhaseServiceSetRefs workflowId "preRun" (
        raw.preRun.serviceSets or [ ]
      );
      normalizedPostRunServiceSets = normalizePhaseServiceSetRefs workflowId "postRun" (
        raw.postRun.serviceSets or [ ]
      );

      authoredUnitsByTask = unitNamesByTask authoredUnits;

      unitCompilation = builtins.mapAttrs (
        unitName: unit:
        let
          task = allTasks.${unit.taskId};
          taskRequiredServices = task.requirements.services or [ ];
          unitRequiredServices = unit.requirements.services or [ ];
          effectiveRequiredServices = listUtils.uniquePreserveOrder (
            taskRequiredServices ++ unitRequiredServices
          );
          excludedRequirements = builtins.filter (
            serviceName: builtins.elem serviceName excludedServices
          ) effectiveRequiredServices;
          initialPruneReason =
            if isPrunedTaskId unit.taskId then
              {
                reason = "task-pruned";
                taskId = unit.taskId;
                taskReason = pruneReasonsByTaskId.${unit.taskId} or null;
              }
            else if excludedRequirements != [ ] then
              {
                reason = "service-excluded";
                serviceName = builtins.head excludedRequirements;
                serviceNames = excludedRequirements;
              }
            else
              null;
          taskScheduling = task.scheduling;
          taskDeps = task.deps;
          taskProduces = task.produces;
          hardTaskNeeds =
            if initialPruneReason != null then
              [ ]
            else
              builtins.concatLists (
                map (depTaskId: resolveHardTaskDeps workflowId authoredUnitsByTask unitName depTaskId) (
                  taskDeps.needs or [ ]
                )
              );
          softTaskNeeds =
            if initialPruneReason != null then
              [ ]
            else
              builtins.concatLists (
                map (depTaskId: resolveSoftTaskDeps authoredUnitsByTask depTaskId) (taskDeps.softNeeds or [ ])
              );
        in
        {
          taskId = unit.taskId;
          explicitNeeds = unit.needs;
          hardNeeds = listUtils.uniquePreserveOrder (unit.needs ++ hardTaskNeeds);
          softNeeds = listUtils.uniquePreserveOrder softTaskNeeds;
          locks = listUtils.uniquePreserveOrder (unit.locks ++ (taskScheduling.locks or [ ]));
          when = unit.when;
          skipIfMissingEnv = unit.skipIfMissingEnv;
          requirements = {
            services = effectiveRequiredServices;
          };
          priority = taskScheduling.priority or 100;
          scheduling = {
            maxAttempts = taskScheduling.maxAttempts or 1;
            retryBackoffSec = taskScheduling.retryBackoffSec or [ ];
            priority = taskScheduling.priority or 100;
          };
          deps = taskDeps;
          produces = taskProduces;
          initialPruneReason = initialPruneReason;
        }
      ) authoredUnits;

      unitNames = builtins.sort builtins.lessThan (builtins.attrNames unitCompilation);

      initialPruneReasonsByUnit = builtins.listToAttrs (
        builtins.concatLists (
          map (
            unitName:
            let
              unit = unitCompilation.${unitName};
            in
            if unit.initialPruneReason == null then
              [ ]
            else
              [
                {
                  name = unitName;
                  value = unit.initialPruneReason;
                }
              ]
          ) unitNames
        )
      );

      pruneUnitsUntilStable =
        pruneReasonsByUnit:
        let
          nextPruneReasons = builtins.listToAttrs (
            builtins.concatLists (
              map (
                unitName:
                if builtins.hasAttr unitName pruneReasonsByUnit then
                  [ ]
                else
                  let
                    unit = unitCompilation.${unitName};
                    blockingNeeds = builtins.filter (
                      depUnitName: builtins.hasAttr depUnitName pruneReasonsByUnit
                    ) unit.hardNeeds;
                  in
                  if blockingNeeds == [ ] then
                    [ ]
                  else
                    [
                      {
                        name = unitName;
                        value = {
                          reason = "required-unit-pruned";
                          dependency = builtins.head blockingNeeds;
                        };
                      }
                    ]
              ) unitNames
            )
          );
        in
        if nextPruneReasons == { } then
          pruneReasonsByUnit
        else
          pruneUnitsUntilStable (pruneReasonsByUnit // nextPruneReasons);

      pruneReasonsByUnit = pruneUnitsUntilStable initialPruneReasonsByUnit;
      survivingUnitNames = builtins.filter (
        unitName: !(builtins.hasAttr unitName pruneReasonsByUnit)
      ) unitNames;
      survivingUnitSet = builtins.listToAttrs (
        map (unitName: {
          name = unitName;
          value = true;
        }) survivingUnitNames
      );

      finalUnits = builtins.listToAttrs (
        map (
          unitName:
          let
            unit = unitCompilation.${unitName};
            softNeeds = builtins.filter (
              depUnitName: builtins.hasAttr depUnitName survivingUnitSet
            ) unit.softNeeds;
          in
          {
            name = unitName;
            value = canonical.canonicalize {
              taskId = unit.taskId;
              needs = listUtils.uniquePreserveOrder (unit.hardNeeds ++ softNeeds);
              locks = unit.locks;
              when = unit.when;
              skipIfMissingEnv = unit.skipIfMissingEnv;
              requirements = unit.requirements;
              priority = unit.priority;
              scheduling = unit.scheduling;
              deps = unit.deps;
              produces = unit.produces;
            };
          }
        ) survivingUnitNames
      );

      survivingPreRunTasks = builtins.filter (
        taskId: builtins.hasAttr taskId tasks
      ) normalizedPreRunTasks;
      survivingPostRunTasks = builtins.filter (
        taskId: builtins.hasAttr taskId tasks
      ) normalizedPostRunTasks;

      order = topoSort workflowId finalUnits;
      workflowStages = deriveStages workflowId finalUnits;

      stageIndexByUnit = builtins.listToAttrs (
        builtins.concatLists (
          lib.imap0 (
            stageIndex: stageUnits:
            map (unitName: {
              name = unitName;
              value = stageIndex;
            }) stageUnits
          ) workflowStages
        )
      );

      plan =
        builtins.sort
          (
            a: b:
            if a.stage != b.stage then
              a.stage < b.stage
            else if a.priority != b.priority then
              a.priority > b.priority
            else
              a.name < b.name
          )
          (
            map (
              unitName:
              let
                unit = finalUnits.${unitName};
              in
              {
                name = unitName;
                stage = stageIndexByUnit.${unitName};
                taskId = unit.taskId;
                needs = unit.needs;
                locks = unit.locks;
                when = unit.when;
                skipIfMissingEnv = unit.skipIfMissingEnv;
                requirements = unit.requirements;
                priority = unit.priority;
                scheduling = unit.scheduling;
                deps = unit.deps;
                produces = unit.produces;
              }
            ) order
          );
    in
    canonical.canonicalize {
      id = workflowId;
      summary = raw.summary;
      description = raw.description;
      mode = raw.mode;
      maxWorkers = if raw.maxWorkers < 1 then 1 else raw.maxWorkers;
      logging = raw.logging;
      units = finalUnits;
      stages = workflowStages;
      preRun = (raw.preRun or { }) // {
        tasks = survivingPreRunTasks;
        serviceSets = normalizedPreRunServiceSets;
      };
      postRun = (raw.postRun or { }) // {
        tasks = survivingPostRunTasks;
        serviceSets = normalizedPostRunServiceSets;
      };
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
