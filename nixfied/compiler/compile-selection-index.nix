{ lib }:
{
  tasks,
  workflows,
  serviceCatalog,
}:
let
  listUtils = import ../framework/core/list-utils.nix;

  taskSet = if tasks == null then { } else tasks;
  workflowSet = if workflows == null then { } else workflows;
  catalog = if serviceCatalog == null then { } else serviceCatalog;

  taskIds = builtins.sort builtins.lessThan (builtins.attrNames taskSet);
  workflowIds = builtins.sort builtins.lessThan (builtins.attrNames workflowSet);

  uniquePreserveOrder = listUtils.uniquePreserveOrder;
  uniqueSorted = values: builtins.sort builtins.lessThan (lib.unique values);

  workflowFamilyFromId =
    workflowId:
    let
      match = builtins.match "^workflow\\.([^.]+)\\..+$" workflowId;
    in
    if match == null then null else builtins.elemAt match 0;

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

  workflowIdsByFamily = builtins.listToAttrs (
    map (family: {
      name = family;
      value = builtins.filter (workflowId: workflowFamilyFromId workflowId == family) workflowIds;
    }) workflowFamilies
  );

  enabledServices = uniqueSorted (
    builtins.map (
      serviceId:
      let
        service = catalog.${serviceId};
      in
      service.name or serviceId
    ) (builtins.filter (serviceId: catalog.${serviceId}.enable or false) (builtins.attrNames catalog))
  );

  taskDirectServicesById = builtins.mapAttrs (
    _: task: uniquePreserveOrder (((task.requirements or { }).services or [ ]))
  ) taskSet;

  goTaskBase =
    seen: taskId:
    let
      token = "task:${taskId}";
    in
    if !(builtins.hasAttr taskId taskSet) || builtins.elem token seen then
      [ ]
    else
      let
        task = taskSet.${taskId};
        nextSeen = seen ++ [ token ];
        depIds = (task.deps.needs or [ ]) ++ (task.deps.softNeeds or [ ]);
        depServices = builtins.concatLists (map (depTaskId: goTaskBase nextSeen depTaskId) depIds);
      in
      uniquePreserveOrder (((task.requirements or { }).services or [ ]) ++ depServices);

  goWorkflow =
    seen: workflowId:
    let
      token = "workflow:${workflowId}";
    in
    if !(builtins.hasAttr workflowId workflowSet) || builtins.elem token seen then
      [ ]
    else
      let
        workflow = workflowSet.${workflowId};
        nextSeen = seen ++ [ token ];
        unitNames = builtins.sort builtins.lessThan (builtins.attrNames (workflow.units or { }));
        unitServices = builtins.concatLists (
          map (
            unitName:
            let
              unit = workflow.units.${unitName};
              taskId = unit.taskId or "";
            in
            uniquePreserveOrder (
              ((unit.requirements or { }).services or [ ])
              ++ (lib.optionals (taskId != "") (goTask nextSeen taskId))
            )
          ) unitNames
        );
        phaseTaskIds = (workflow.preRun.tasks or [ ]) ++ (workflow.postRun.tasks or [ ]);
        phaseTaskServices = builtins.concatLists (map (taskId: goTask nextSeen taskId) phaseTaskIds);
        phaseServiceSetServices = builtins.concatLists (
          map (entry: entry.selectedServices or [ ]) (
            (workflow.preRun.serviceSets or [ ]) ++ (workflow.postRun.serviceSets or [ ])
          )
        );
      in
      uniquePreserveOrder (unitServices ++ phaseTaskServices ++ phaseServiceSetServices);

  goWorkflowUnitsOnly =
    seen: workflowId:
    let
      token = "workflow:${workflowId}:units";
    in
    if !(builtins.hasAttr workflowId workflowSet) || builtins.elem token seen then
      [ ]
    else
      let
        workflow = workflowSet.${workflowId};
        nextSeen = seen ++ [ token ];
        unitNames = builtins.sort builtins.lessThan (builtins.attrNames (workflow.units or { }));
      in
      uniquePreserveOrder (
        builtins.concatLists (
          map (
            unitName:
            let
              unit = workflow.units.${unitName};
              taskId = unit.taskId or "";
            in
            uniquePreserveOrder (
              ((unit.requirements or { }).services or [ ])
              ++ (lib.optionals (taskId != "") (goTask nextSeen taskId))
            )
          ) unitNames
        )
      );

  goWorkflowReference =
    seen: workflowId:
    let
      family = workflowFamilyFromId workflowId;
      workflowIdsForFamily =
        if family == null then [ workflowId ] else workflowIdsByFamily.${family} or [ workflowId ];
    in
    uniquePreserveOrder (
      builtins.concatLists (map (candidateId: goWorkflow seen candidateId) workflowIdsForFamily)
    );

  goTask =
    seen: taskId:
    let
      token = "task:${taskId}";
    in
    if !(builtins.hasAttr taskId taskSet) || builtins.elem token seen then
      [ ]
    else
      let
        task = taskSet.${taskId};
        nextSeen = seen ++ [ token ];
        workflowServices =
          if (task.runner.type or "shell") == "workflowRef" && (task.runner.workflowId or "") != "" then
            goWorkflowReference nextSeen task.runner.workflowId
          else
            [ ];
      in
      uniquePreserveOrder ((goTaskBase seen taskId) ++ workflowServices);

  taskClosureServicesById = builtins.listToAttrs (
    map (taskId: {
      name = taskId;
      value = goTask [ ] taskId;
    }) taskIds
  );

  taskBaseClosureServicesById = builtins.listToAttrs (
    map (taskId: {
      name = taskId;
      value = goTaskBase [ ] taskId;
    }) taskIds
  );

  taskRunnerWorkflowIdById = builtins.listToAttrs (
    map (taskId: {
      name = taskId;
      value = taskSet.${taskId}.runner.workflowId or "";
    }) taskIds
  );

  workflowUnitClosureServicesById = builtins.listToAttrs (
    map (workflowId: {
      name = workflowId;
      value = goWorkflowUnitsOnly [ ] workflowId;
    }) workflowIds
  );

  workflowClosureServicesById = builtins.listToAttrs (
    map (workflowId: {
      name = workflowId;
      value = goWorkflow [ ] workflowId;
    }) workflowIds
  );

  workflowReferenceClosureServicesById = builtins.listToAttrs (
    map (workflowId: {
      name = workflowId;
      value = goWorkflowReference [ ] workflowId;
    }) workflowIds
  );

  servicesToCsv = serviceNames: builtins.concatStringsSep "," (uniqueSorted serviceNames);

  taskBaseClosureServicesCsvById = builtins.listToAttrs (
    map (taskId: {
      name = taskId;
      value = servicesToCsv (taskBaseClosureServicesById.${taskId} or [ ]);
    }) taskIds
  );

  workflowClosureServicesCsvById = builtins.listToAttrs (
    map (workflowId: {
      name = workflowId;
      value = servicesToCsv (workflowClosureServicesById.${workflowId} or [ ]);
    }) workflowIds
  );
in
{
  inherit
    enabledServices
    taskIds
    workflowIds
    workflowFamilies
    workflowIdsByFamily
    servicesToCsv
    taskDirectServicesById
    taskBaseClosureServicesById
    taskBaseClosureServicesCsvById
    taskClosureServicesById
    taskRunnerWorkflowIdById
    workflowModesByFamily
    workflowUnitClosureServicesById
    workflowClosureServicesById
    workflowClosureServicesCsvById
    workflowReferenceClosureServicesById
    ;
}
