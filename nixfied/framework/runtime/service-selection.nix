{ lib, model }:
let
  listUtils = import ../core/list-utils.nix;

  tasks = model.tasks or { };
  workflows = model.workflows or { };
  serviceCatalog = model.serviceCatalog or { };

  taskIds = builtins.sort builtins.lessThan (builtins.attrNames tasks);
  workflowIds = builtins.sort builtins.lessThan (builtins.attrNames workflows);

  uniquePreserveOrder = listUtils.uniquePreserveOrder;
  uniqueSorted = values: builtins.sort builtins.lessThan (lib.unique values);

  workflowFamilyFromId =
    workflowId:
    let
      match = builtins.match "^workflow\\.([^.]+)\\..+$" workflowId;
    in
    if match == null then null else builtins.elemAt match 0;

  workflowFamilies = uniqueSorted (
    builtins.filter (family: family != null) (map workflowFamilyFromId workflowIds)
  );

  workflowIdsByFamily = builtins.listToAttrs (
    map (family: {
      name = family;
      value = builtins.filter (workflowId: workflowFamilyFromId workflowId == family) workflowIds;
    }) workflowFamilies
  );

  enabledServices = uniqueSorted (
    builtins.map
      (
        serviceId:
        let
          service = serviceCatalog.${serviceId};
        in
        service.name or serviceId
      )
      (
        builtins.filter (serviceId: serviceCatalog.${serviceId}.enable or false) (
          builtins.attrNames serviceCatalog
        )
      )
  );

  taskDirectServicesById = builtins.mapAttrs (
    _: task: uniquePreserveOrder (((task.requirements or { }).services or [ ]))
  ) tasks;

  goTaskBase =
    seen: taskId:
    let
      token = "task:${taskId}";
    in
    if !(builtins.hasAttr taskId tasks) || builtins.elem token seen then
      [ ]
    else
      let
        task = tasks.${taskId};
        nextSeen = seen ++ [ token ];
        depIds = (task.deps.needs or [ ]) ++ (task.deps.softNeeds or [ ]);
        depServices = builtins.concatLists (map (depTaskId: goTaskBase nextSeen depTaskId) depIds);
      in
      uniquePreserveOrder (((task.requirements or { }).services or [ ]) ++ depServices);

  goTask =
    seen: taskId:
    let
      token = "task:${taskId}";
    in
    if !(builtins.hasAttr taskId tasks) || builtins.elem token seen then
      [ ]
    else
      let
        task = tasks.${taskId};
        nextSeen = seen ++ [ token ];
        workflowServices =
          if (task.runner.type or "shell") == "workflowRef" && (task.runner.workflowId or "") != "" then
            goWorkflowReference nextSeen task.runner.workflowId
          else
            [ ];
      in
      uniquePreserveOrder ((goTaskBase seen taskId) ++ workflowServices);

  goWorkflow =
    seen: workflowId:
    let
      token = "workflow:${workflowId}";
    in
    if !(builtins.hasAttr workflowId workflows) || builtins.elem token seen then
      [ ]
    else
      let
        workflow = workflows.${workflowId};
        nextSeen = seen ++ [ token ];
        unitNames = builtins.sort builtins.lessThan (builtins.attrNames (workflow.units or { }));
        unitServices = builtins.concatLists (
          map (unitName: ((workflow.units.${unitName}.requirements or { }).services or [ ])) unitNames
        );
        phaseTaskIds = (workflow.preRun.tasks or [ ]) ++ (workflow.postRun.tasks or [ ]);
        phaseTaskServices = builtins.concatLists (map (taskId: goTask nextSeen taskId) phaseTaskIds);
      in
      uniquePreserveOrder (unitServices ++ phaseTaskServices);

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

  workflowClosureServicesById = builtins.listToAttrs (
    map (workflowId: {
      name = workflowId;
      value = goWorkflow [ ] workflowId;
    }) workflowIds
  );

  workflowFamilyClosureServicesByFamily = builtins.listToAttrs (
    map (family: {
      name = family;
      value = uniquePreserveOrder (
        builtins.concatLists (
          map (workflowId: workflowClosureServicesById.${workflowId}) (workflowIdsByFamily.${family} or [ ])
        )
      );
    }) workflowFamilies
  );

  workflowReferenceClosureServicesById = builtins.listToAttrs (
    map (workflowId: {
      name = workflowId;
      value = goWorkflowReference [ ] workflowId;
    }) workflowIds
  );

  servicesToCsv = serviceNames: builtins.concatStringsSep "," (uniqueSorted serviceNames);
in
{
  inherit
    enabledServices
    taskDirectServicesById
    taskClosureServicesById
    taskBaseClosureServicesById
    workflowClosureServicesById
    workflowFamilyClosureServicesByFamily
    workflowReferenceClosureServicesById
    servicesToCsv
    ;
}
