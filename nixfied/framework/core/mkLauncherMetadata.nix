{
  lib,
  model,
  selectionIndex,
  serviceSurfaceCatalog,
  workspaceMarkerPresent,
}:
let
  viewAppNames = builtins.sort builtins.lessThan (builtins.attrNames (model.views.apps or { }));

  selectorDispatcherAppNames = [
    "run-task"
    "run-workflow"
    "run-workflow-parallel"
  ];

  nonSelectorAppNames = [
    "framework::install"
    "framework::upgrade"
  ];

  runtimeControlAppNames = [
    "runs"
    "stop-run"
    "stop-all-runs"
  ];

  runtimeProxyAppNames = if workspaceMarkerPresent then [ ] else nonSelectorAppNames;

  viewWrappedAppNames = builtins.sort builtins.lessThan (
    builtins.filter (appName: !(builtins.elem appName nonSelectorAppNames)) (
      lib.unique (viewAppNames ++ selectorDispatcherAppNames)
    )
  );

  serviceWrappedAppNames = builtins.sort builtins.lessThan (serviceSurfaceCatalog.appNames or [ ]);

  runtimeAppNames = builtins.sort builtins.lessThan (
    lib.unique (
      runtimeProxyAppNames
      ++ builtins.filter (appName: builtins.elem appName nonSelectorAppNames) viewAppNames
    )
  );
in
{
  enabledServices = selectionIndex.enabledServices or [ ];
  taskIds =
    if selectionIndex ? taskIds then
      selectionIndex.taskIds
    else
      builtins.sort builtins.lessThan (builtins.attrNames (model.tasks or { }));
  workflowIds =
    if selectionIndex ? workflowIds then
      selectionIndex.workflowIds
    else
      builtins.sort builtins.lessThan (builtins.attrNames (model.workflows or { }));
  workflowModesByFamily = selectionIndex.workflowModesByFamily or { };
  workflowFamilies =
    if selectionIndex ? workflowFamilies then
      selectionIndex.workflowFamilies
    else
      builtins.sort builtins.lessThan (builtins.attrNames (selectionIndex.workflowModesByFamily or { }));
  taskBaseClosureCsvById = selectionIndex.taskBaseClosureServicesCsvById or { };
  taskRunnerWorkflowIdById = selectionIndex.taskRunnerWorkflowIdById or { };
  workflowClosureCsvById = selectionIndex.workflowClosureServicesCsvById or { };
  inherit
    selectorDispatcherAppNames
    nonSelectorAppNames
    runtimeControlAppNames
    runtimeProxyAppNames
    viewWrappedAppNames
    serviceWrappedAppNames
    runtimeAppNames
    ;
}
