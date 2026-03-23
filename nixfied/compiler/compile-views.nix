{ lib }:
{
  projectRoot,
  resolved,
  statePolicy,
  features,
  runtime,
  services,
  apps,
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

  appIds = builtins.sort builtins.lessThan (builtins.attrNames apps);
  visibleApps = builtins.foldl' (
    acc: appId:
    let
      app = apps.${appId};
      ownerFile =
        if (app.ownerFile or null) == null || app.ownerFile == "" then
          "nixfied/project/module.nix"
        else
          app.ownerFile;
    in
    if (!workspaceMarkerPresent) && isFrameworkHiddenApp appId then
      acc
    else
      acc
      // {
        ${appId} = {
          kind =
            if app.kind == "taskRef" then
              "task"
            else if app.kind == "workflowRef" then
              "workflow"
            else if app.kind == "serviceSetRef" then
              "service-set"
            else if app.kind == "machineOutput" then
              "machine-output"
            else
              app.kind;
          taskId = app.taskId or null;
          workflowId = app.workflowId or null;
          serviceSetId = app.serviceSetId or null;
          operation = app.operation or null;
          targetAppId = app.targetAppId or null;
          summary = app.summary;
          description = app.description;
          category = app.category or "core";
          usage = app.usage or [ ];
          examples = app.examples or [ ];
          ownerFile = ownerFile;
        };
      }
  ) { } appIds;
  appNames = builtins.sort builtins.lessThan (builtins.attrNames visibleApps);
  featureIds = builtins.sort builtins.lessThan (builtins.attrNames features);

  upgradeFallbackSummary = "Upgrade framework bundle in-place";
  needsUpgradeFallback = (!workspaceMarkerPresent) && !(builtins.elem "framework::upgrade" appNames);

  coreCommands =
    let
      fromApps = map (appName: {
        name = appName;
        summary = visibleApps.${appName}.summary;
        owner_file = visibleApps.${appName}.ownerFile;
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
      name = "introspect";
      summary = "Query compiled apps, tasks, workflows, services, and packages";
      owner_file = "nixfied/framework/core/mkCoreSurfaces.nix";
    }
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
      name = "schema";
      summary = "Print bundled export schemas";
      owner_file = "nixfied/framework/core/mkNixfied.nix";
    }
    {
      name = "stateHash";
      summary = "Print canonical model hash";
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
    "- State policy id: ${statePolicy.id}"
    "- State policy kind: ${statePolicy.kind}"
    "- State policy source: ${statePolicy.source}"
    "- Workspace id: ${statePolicy.workspaceId}"
    ""
    "## Runtime"
    ""
    "- Slot variable: ${runtime.slot.var}"
    "- Slot default: ${toString runtime.slot.default}"
    "- Environment variable: ${runtime.env.var}"
    "- Environment names: ${builtins.concatStringsSep ", " runtime.env.names}"
    "- Runtime directory base: ${runtime.directories.base}"
    "- Registry root: ${statePolicy.registryRoot}"
    "- Artifacts root: ${statePolicy.artifactsRoot}"
    "- Enabled services: ${enabledServicesLine}"
    "- Feature count: ${toString (builtins.length featureIds)}"
    ""
    "## Current Workflow Semantics"
    ""
    "- Workflows forward one shared passthrough argv to preRun, units, and postRun."
    "- Workflow units do not support per-step args, env, or outputs today."
    "- Use wrapper tasks when an app needs different step args or a strict public stdout contract."
    ""
    "## Service Lifecycle Review"
    ""
    "- Public service modules in this repo use start/stop/restart/health/ready."
    "- Supervisor is a separate runtime surface and does not provide ready."
    "- See tests/framework/WORKFLOW_REUSE.md and tests/framework/SERVICE_LIFECYCLE_API.md for the normative guide and API review."
    ""
    "## Runtime Conventions"
    ""
    "- Generic runtime port env names are derived from normalized model keys."
    "- Example: heliosRpc becomes HELIOSRPC_PORT in generic task execution."
    ""
    "## Exposed Apps"
  ]
  ++ map (appName: "- ${appName}: ${visibleApps.${appName}.summary}") appNames
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
  apps = visibleApps;

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
