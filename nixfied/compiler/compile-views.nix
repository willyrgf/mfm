{ lib }:
{
  projectRoot,
  resolved,
  runtime,
  services,
  tasks,
  workflows,
}:
let
  workspaceMarkerPresent = builtins.pathExists "${projectRoot}/nixfied/.framework/.workspace";

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
        };
      };

  apps = builtins.foldl' addApp { } taskIds;
  appNames = builtins.sort builtins.lessThan (builtins.attrNames apps);

  upgradeFallbackSummary = "Upgrade framework bundle in-place";
  needsUpgradeFallback = (!workspaceMarkerPresent) && !(builtins.elem "framework::upgrade" appNames);

  coreCommands =
    let
      fromApps = map (appName: {
        name = appName;
        summary = apps.${appName}.summary;
      }) appNames;

      withFallback =
        if needsUpgradeFallback then
          fromApps
          ++ [
            {
              name = "framework::upgrade";
              summary = upgradeFallbackSummary;
            }
          ]
        else
          fromApps;
    in
    builtins.sort (a: b: a.name < b.name) withFallback;

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
    ""
    "## Runtime"
    ""
    "- Slot variable: ${runtime.slot.var}"
    "- Slot default: ${toString runtime.slot.default}"
    "- Environment variable: ${runtime.env.var}"
    "- Environment names: ${builtins.concatStringsSep ", " runtime.env.names}"
    "- Runtime directory base: ${runtime.directories.base}"
    "- Enabled services: ${enabledServicesLine}"
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
in
{
  inherit apps;

  help = {
    lines = helpLines;
    commands = coreCommands;
  };

  docs = {
    lines = docsLines;
  };
}
