{ lib }:
{
  resolved,
  runtime,
  tasks,
  workflows,
}:
let
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

  workflowIds = builtins.sort builtins.lessThan (builtins.attrNames workflows);

  helpLines =
    [
      "Nixfied commands (model-generated)"
      ""
      "Core apps:"
    ]
    ++ map (
      appName: "  ${appName} - ${apps.${appName}.summary}"
    ) appNames
    ++ [
      ""
      "Dispatcher:"
      "  run-task <task-id> [-- ...]"
      "  run-workflow <workflow-id> [-- ...]"
      ""
      "Workflows:"
    ]
    ++ map (
      workflowId: "  ${workflowId} - ${workflows.${workflowId}.summary}"
    ) workflowIds;

  docsLines =
    [
      "# Nixfied Detailed Model"
      ""
      "This document is generated from nixfiedModel."
      ""
      "## Runtime"
      ""
      "- Slot variable: ${runtime.slot.var}"
      "- Slot default: ${toString runtime.slot.default}"
      "- Environment variable: ${runtime.env.var}"
      "- Environment names: ${builtins.concatStringsSep ", " runtime.env.names}"
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
    commands = map (
      appName: {
        name = appName;
        summary = apps.${appName}.summary;
      }
    ) appNames;
  };

  docs = {
    lines = docsLines;
  };
}
