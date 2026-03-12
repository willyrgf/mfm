{ lib }:
{
  projectRoot,
  resolved,
  features,
  runtime,
  services,
  tasks,
  workflows,
}:
let
  workspaceMarker = import ../framework/workspace-marker.nix;
  workspaceMarkerPresent = workspaceMarker.isPresent projectRoot;

  frameworkHiddenApps = [
    "framework::install"
    "framework::test"
  ];

  isFrameworkHiddenApp = appName: builtins.elem appName frameworkHiddenApps;

  taskIds = builtins.sort builtins.lessThan (builtins.attrNames tasks);

  addApp =
    acc: taskId:
    let
      task = tasks.${taskId};
      app = task.ui.app;
      appName = app.name;
      ownerFile =
        if (app.ownerFile or null) == null || app.ownerFile == "" then
          "nixfied/project/module.nix"
        else
          app.ownerFile;
    in
    if !app.expose then
      acc
    else if (!workspaceMarkerPresent) && isFrameworkHiddenApp appName then
      acc
    else if builtins.hasAttr appName acc then
      throw "duplicate app name '${appName}' generated from tasks '${acc.${appName}.taskId}' and '${taskId}'"
    else
      acc
      // {
        ${appName} = {
          kind = "task";
          taskId = taskId;
          summary = task.summary;
          description = task.description;
          category = app.category;
          usage = app.usage;
          examples = app.examples;
          ownerFile = ownerFile;
        };
      };

  apps = builtins.foldl' addApp { } taskIds;
  appNames = builtins.sort builtins.lessThan (builtins.attrNames apps);
  featureIds = builtins.sort builtins.lessThan (builtins.attrNames features);

  upgradeFallbackSummary = "Upgrade framework bundle in-place";
  needsUpgradeFallback = (!workspaceMarkerPresent) && !(builtins.elem "framework::upgrade" appNames);

  coreCommands =
    let
      fromApps = map (appName: {
        name = appName;
        summary = apps.${appName}.summary;
        owner_file = apps.${appName}.ownerFile;
      }) appNames;

      withFallback =
        if needsUpgradeFallback then
          fromApps
          ++ [
            {
              name = "framework::upgrade";
              summary = upgradeFallbackSummary;
              owner_file = "nixfied/framework/runtime/dispatcher.nix";
            }
          ]
        else
          fromApps;
    in
    builtins.sort (a: b: a.name < b.name) withFallback;

  introspectionCommands = [
    {
      name = "docs";
      summary = "Render detailed model documentation";
      owner_file = "nixfied/framework/runtime/dispatcher.nix";
    }
    {
      name = "features";
      summary = "List compiled feature inventory";
      owner_file = "nixfied/framework/runtime/dispatcher.nix";
    }
    {
      name = "model";
      summary = "Print canonical compiled model";
      owner_file = "nixfied/framework/core/mkNixfied.nix";
    }
    {
      name = "schema";
      summary = "Print bundled export schemas";
      owner_file = "nixfied/framework/core/mkNixfied.nix";
    }
    {
      name = "services";
      summary = "List compiled services";
      owner_file = "nixfied/framework/core/mkNixfied.nix";
    }
    {
      name = "stateHash";
      summary = "Print canonical model hash";
      owner_file = "nixfied/framework/core/mkNixfied.nix";
    }
    {
      name = "tasks";
      summary = "List compiled tasks";
      owner_file = "nixfied/framework/core/mkNixfied.nix";
    }
  ];

  helpCommands = coreCommands ++ introspectionCommands;

  workflowIds = builtins.sort builtins.lessThan (builtins.attrNames workflows);
  serviceIds = builtins.sort builtins.lessThan (builtins.attrNames services);
  enabledServiceNames = map (serviceId: services.${serviceId}.name) (
    builtins.filter (serviceId: services.${serviceId}.enable or false) serviceIds
  );
  enabledServicesLine =
    if enabledServiceNames == [ ] then "none" else builtins.concatStringsSep ", " enabledServiceNames;

  helpLines = [
    "${resolved.identity.projectName} commands (model-generated)"
    resolved.identity.description
    ""
    "Core apps:"
  ]
  ++ map (entry: "  ${entry.name} - ${entry.summary}") coreCommands
  ++ [
    ""
    "Introspection:"
  ]
  ++ map (entry: "  ${entry.name} - ${entry.summary}") introspectionCommands
  ++ [
    ""
    "Dispatcher:"
    "  run-task <task-id> [-- ...]"
    "  run-workflow <workflow-id> [-- ...]"
    "  run-workflow-parallel <workflow-id> [-- ...]"
    "  runs [run-id]"
    "  stop-run <run-id>"
    "  stop-all-runs"
    ""
    "Workflows:"
  ]
  ++ map (workflowId: "  ${workflowId} - ${workflows.${workflowId}.summary}") workflowIds;

  docsLines = [
    "# Nixfied Detailed Model"
    ""
    "This document is generated from nixfiedModel."
    ""
    "## Identity"
    ""
    "- Project id: ${resolved.identity.projectId}"
    "- Project name: ${resolved.identity.projectName}"
    "- Description: ${resolved.identity.description}"
    "- Workspace id: ${resolved.state.workspaceId}"
    ""
    "## Runtime"
    ""
    "- Slot variable: ${runtime.slot.var}"
    "- Slot default: ${toString runtime.slot.default}"
    "- Environment variable: ${runtime.env.var}"
    "- Environment names: ${builtins.concatStringsSep ", " runtime.env.names}"
    "- Runtime directory base: ${runtime.directories.base}"
    "- Enabled services: ${enabledServicesLine}"
    "- Feature count: ${toString (builtins.length featureIds)}"
    ""
    "## Exposed Apps"
  ]
  ++ map (appName: "- ${appName}: ${apps.${appName}.summary}") appNames
  ++ [
    ""
    "## Workflows"
  ]
  ++ map (
    workflowId:
    let
      workflow = workflows.${workflowId};
      order = map (unit: unit.name) workflow.plan;
    in
    "- ${workflowId}: ${builtins.concatStringsSep " -> " order}"
  ) workflowIds;

  featureLines = [
    "${resolved.identity.projectName} features (model-generated)"
    resolved.identity.description
    ""
    "Features:"
  ]
  ++ map (
    featureId:
    let
      feature = features.${featureId};
      coverageSuffix = if feature.coverageRequired or false then " coverage=required" else "";
    in
    "  ${featureId} [${feature.kind}] - ${feature.summary}${coverageSuffix}"
  ) featureIds;
in
{
  inherit apps;

  help = {
    lines = helpLines;
    commands = map (entry: {
      inherit (entry)
        name
        summary
        ;
    }) helpCommands;
    commandSurfaces = map (entry: {
      inherit (entry)
        name
        owner_file
        ;
    }) helpCommands;
  };

  docs = {
    lines = docsLines;
  };

  features = {
    lines = featureLines;
    entries = map (featureId: {
      id = featureId;
      kind = features.${featureId}.kind;
      summary = features.${featureId}.summary;
      status = features.${featureId}.status;
      coverageRequired = features.${featureId}.coverageRequired or false;
    }) featureIds;
  };
}
