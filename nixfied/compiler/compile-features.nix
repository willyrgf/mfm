{
  lib,
  canonical,
}:
{
  projectRoot,
  runtime,
  services,
  tasks,
  workflows,
}:
let
  listUtils = import ../framework/core/list-utils.nix;
  featureCatalog = import ./feature-catalog.nix { inherit runtime; };
  safeProjectRoot = builtins.unsafeDiscardStringContext (builtins.toString projectRoot);

  maybeExistingPaths =
    paths:
    listUtils.uniqueNonEmptyPreserveOrder (
      builtins.filter (path: builtins.pathExists "${safeProjectRoot}/${path}") paths
    );

  taskOwnerFiles =
    taskId:
    let
      task = tasks.${taskId};
      app = task.ui.app or { };
      appOwner = if (app.ownerFile or null) == null then "" else app.ownerFile;
    in
    listUtils.uniqueNonEmptyPreserveOrder (
      maybeExistingPaths [ appOwner ]
      ++ maybeExistingPaths [
        "nixfied/project/module.nix"
        "nixfied/modules/operations.nix"
      ]
    );

  taskFeature =
    taskId:
    let
      task = tasks.${taskId};
      app = task.ui.app or { };
      appName = app.name or "";
      appExpose = app.expose or false;
    in
    canonical.canonicalize {
      id = taskId;
      kind = "task";
      summary = task.summary;
      surfaces =
        if appExpose then
          [
            {
              kind = "app";
              name = appName;
            }
          ]
        else
          [
            {
              kind = "dispatcher";
              name = "run-task";
            }
          ];
      ownerFiles = taskOwnerFiles taskId;
      modelPaths = [ "tasks.${taskId}" ];
      status = if appExpose then "stable" else "internal";
      defaults = {
        appName = appName;
        expose = appExpose;
      };
      coverageRequired = appExpose;
      docs = [ ];
    };

  workflowTaskIds =
    workflow:
    listUtils.uniquePreserveOrder (
      map (unit: unit.taskId) workflow.plan
      ++ (workflow.preRun.tasks or [ ])
      ++ (workflow.postRun.tasks or [ ])
    );

  workflowFeature =
    workflowId:
    let
      workflow = workflows.${workflowId};
      ownerFiles = listUtils.uniqueNonEmptyPreserveOrder (
        builtins.concatLists (map taskOwnerFiles (workflowTaskIds workflow))
      );
    in
    canonical.canonicalize {
      id = workflowId;
      kind = "workflow";
      summary = workflow.summary;
      surfaces = [
        {
          kind = "workflow";
          name = workflowId;
          dispatcher = "run-workflow";
        }
      ];
      ownerFiles = ownerFiles;
      modelPaths = [ "workflows.${workflowId}" ];
      status = "stable";
      defaults = {
        mode = workflow.mode;
        maxWorkers = workflow.maxWorkers;
      };
      coverageRequired = true;
      docs = [ ];
    };

  serviceFeature =
    serviceId:
    let
      service = services.${serviceId};
      modulePath = "nixfied/modules/services/${service.name}.nix";
    in
    canonical.canonicalize {
      id = serviceId;
      kind = "service";
      summary = "Service configuration and runtime surface for ${service.name}";
      surfaces = [
        {
          kind = "service";
          name = service.name;
        }
      ];
      ownerFiles = maybeExistingPaths [
        modulePath
        "nixfied/project/conf.nix"
      ];
      modelPaths = [ "services.${serviceId}" ];
      status = "stable";
      defaults = {
        enable = service.enable;
      };
      coverageRequired = true;
      docs = [ ];
    };

  runtimeFeature =
    featureId:
    let
      spec = featureCatalog.${featureId};
    in
    canonical.canonicalize (
      spec
      // {
        id = featureId;
        ownerFiles = maybeExistingPaths spec.ownerFiles;
      }
    );

  taskIds = builtins.sort builtins.lessThan (builtins.attrNames tasks);
  workflowIds = builtins.sort builtins.lessThan (builtins.attrNames workflows);
  serviceIds = builtins.sort builtins.lessThan (builtins.attrNames services);
  runtimeIds = builtins.sort builtins.lessThan (builtins.attrNames featureCatalog);

  taskFeatures = builtins.listToAttrs (
    map (taskId: {
      name = taskId;
      value = taskFeature taskId;
    }) taskIds
  );

  workflowFeatures = builtins.listToAttrs (
    map (workflowId: {
      name = workflowId;
      value = workflowFeature workflowId;
    }) workflowIds
  );

  serviceFeatures = builtins.listToAttrs (
    map (serviceId: {
      name = serviceId;
      value = serviceFeature serviceId;
    }) serviceIds
  );

  runtimeFeatures = builtins.listToAttrs (
    map (featureId: {
      name = featureId;
      value = runtimeFeature featureId;
    }) runtimeIds
  );

  merged = taskFeatures // workflowFeatures // serviceFeatures // runtimeFeatures;
  featureIds = builtins.sort builtins.lessThan (builtins.attrNames merged);
in
builtins.listToAttrs (
  map (featureId: {
    name = featureId;
    value = merged.${featureId};
  }) featureIds
)
