{
  lib,
  canonical,
}:
{
  resolvedIdentity,
  runtime,
  state,
  serviceCatalog,
  apps,
  tasks,
  workflows,
  selectionIndex,
}:
let
  listUtils = import ../framework/core/list-utils.nix;
  uniquePreserveOrder = listUtils.uniquePreserveOrder;
  uniqueSorted = values: builtins.sort builtins.lessThan (lib.unique values);

  appIds = builtins.sort builtins.lessThan (builtins.attrNames apps);
  workflowIds = builtins.sort builtins.lessThan (builtins.attrNames workflows);

  workflowFamilyFromId =
    workflowId:
    let
      match = builtins.match "^workflow\\.([^.]+)\\..+$" workflowId;
    in
    if match == null then null else builtins.elemAt match 0;

  workflowIdsByFamily = builtins.listToAttrs (
    map (
      workflowId:
      let
        family = workflowFamilyFromId workflowId;
      in
      {
        name = workflowId;
        value =
          if family == null then
            [ workflowId ]
          else
            builtins.filter (candidateId: workflowFamilyFromId candidateId == family) workflowIds;
      }
    ) workflowIds
  );

  goTask =
    seen: taskId:
    let
      token = "task:${taskId}";
    in
    if !(builtins.hasAttr taskId tasks) || builtins.elem token seen then
      {
        taskIds = [ ];
        workflowIds = [ ];
      }
    else
      let
        task = tasks.${taskId};
        nextSeen = seen ++ [ token ];
        depIds = (task.deps.needs or [ ]) ++ (task.deps.softNeeds or [ ]);
        depClosure = map (depTaskId: goTask nextSeen depTaskId) depIds;
        workflowClosure =
          if (task.runner.type or "") == "workflowRef" && (task.runner.workflowId or "") != "" then
            goWorkflowReference nextSeen task.runner.workflowId
          else
            {
              taskIds = [ ];
              workflowIds = [ ];
            };
      in
      {
        taskIds = uniquePreserveOrder (
          [ taskId ]
          ++ builtins.concatLists (map (entry: entry.taskIds) depClosure)
          ++ workflowClosure.taskIds
        );
        workflowIds = uniquePreserveOrder (
          builtins.concatLists (map (entry: entry.workflowIds) depClosure) ++ workflowClosure.workflowIds
        );
      };

  goWorkflow =
    seen: workflowId:
    let
      token = "workflow:${workflowId}";
    in
    if !(builtins.hasAttr workflowId workflows) || builtins.elem token seen then
      {
        taskIds = [ ];
        workflowIds = [ ];
      }
    else
      let
        workflow = workflows.${workflowId};
        nextSeen = seen ++ [ token ];
        unitNames = builtins.sort builtins.lessThan (builtins.attrNames (workflow.units or { }));
        unitClosures = map (unitName: goTask nextSeen (workflow.units.${unitName}.taskId or "")) unitNames;
        phaseTaskIds = (workflow.preRun.tasks or [ ]) ++ (workflow.postRun.tasks or [ ]);
        phaseClosures = map (taskId: goTask nextSeen taskId) phaseTaskIds;
      in
      {
        taskIds = uniquePreserveOrder (
          builtins.concatLists (map (entry: entry.taskIds) (unitClosures ++ phaseClosures))
        );
        workflowIds = uniquePreserveOrder (
          [ workflowId ]
          ++ builtins.concatLists (map (entry: entry.workflowIds) (unitClosures ++ phaseClosures))
        );
      };

  goWorkflowReference =
    seen: workflowId:
    let
      workflowIdsForReference = workflowIdsByFamily.${workflowId} or [ workflowId ];
      closures = map (candidateId: goWorkflow seen candidateId) workflowIdsForReference;
    in
    {
      taskIds = uniquePreserveOrder (builtins.concatLists (map (entry: entry.taskIds) closures));
      workflowIds = uniquePreserveOrder (builtins.concatLists (map (entry: entry.workflowIds) closures));
    };

  manifestForApp =
    appId:
    let
      app = apps.${appId};
      closure =
        if (app.kind or "") == "workflowRef" then
          goWorkflowReference [ ] app.workflowId
        else
          goTask [ ] app.taskId;
      selectedServices =
        if (app.kind or "") == "workflowRef" then
          uniqueSorted (selectionIndex.workflowReferenceClosureServicesById.${app.workflowId} or [ ])
        else
          uniqueSorted (selectionIndex.taskClosureServicesById.${app.taskId} or [ ]);
      serviceCatalogFiltered = lib.filterAttrs (
        _: service: builtins.elem (service.name or service.id) selectedServices
      ) serviceCatalog;
      manifestEvalHash = canonical.hashCanonical {
        schema = {
          kind = "nixfied-app-execution-eval";
          version = 1;
        };
        runtime = runtime;
        state = state;
        serviceCatalog = serviceCatalogFiltered;
        tasks = lib.getAttrs closure.taskIds tasks;
        workflows = lib.getAttrs closure.workflowIds workflows;
      };
      manifestIdentity = {
        projectId = resolvedIdentity.projectId;
        projectName = resolvedIdentity.projectName;
        description = resolvedIdentity.description;
        evalHash = manifestEvalHash;
      };
      manifestModel = canonical.canonicalize {
        schema = {
          kind = "nixfied-execution-manifest";
          version = 1;
        };
        identity = manifestIdentity;
        runtime = runtime;
        state = state;
        serviceCatalog = serviceCatalogFiltered;
        tasks = lib.getAttrs closure.taskIds tasks;
        workflows = lib.getAttrs closure.workflowIds workflows;
      };
    in
    {
      id = appId;
      taskId = if (app.taskId or "") == "" then null else app.taskId;
      workflowId = if (app.workflowId or "") == "" then null else app.workflowId;
      taskIds = closure.taskIds;
      workflowIds = closure.workflowIds;
      selectedServices = selectedServices;
      model = manifestModel;
      evalHash = manifestEvalHash;
      modelHash = canonical.hashCanonical manifestModel;
    };
  manifestAppIds = builtins.filter (
    appId:
    let
      app = apps.${appId};
    in
    (
      ((app.kind or "") == "taskRef" && (app.taskId or "") != "")
      || ((app.kind or "") == "workflowRef" && (app.workflowId or "") != "")
    )
  ) appIds;
in
builtins.listToAttrs (
  map (appId: {
    name = appId;
    value = manifestForApp appId;
  }) manifestAppIds
)
