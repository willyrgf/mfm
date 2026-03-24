{
  lib,
  canonical,
}:
{
  resolved,
  serviceSets,
  tasks,
  workflows,
  contractBundle,
}:
let
  rawApps = resolved.apps or { };
  names = builtins.sort builtins.lessThan (builtins.attrNames rawApps);
  previewAppsById = builtins.foldl' (
    acc: name:
    let
      raw = rawApps.${name};
      appId = if (raw.id or "") == "" then name else raw.id;
    in
    acc
    // {
      ${appId} = {
        kind = raw.kind or "taskRef";
        taskId = raw.taskId or "";
        workflowId = raw.workflowId or "";
        serviceSetId = raw.serviceSetId or "";
      };
    }
  ) { } names;

  appExists = appId: builtins.hasAttr appId previewAppsById;
  appPreview = appId: previewAppsById.${appId};
  isMachineOutputPreview = appId: appExists appId && (appPreview appId).kind == "machineOutput";
  supportedServiceSetOperations = [
    "start"
    "stop"
    "status"
    "health"
    "ready"
    "export"
  ];
  normalizeAppRefs = values: builtins.sort builtins.lessThan (lib.unique values);

  normalizeApp =
    name:
    let
      raw = rawApps.${name};
      appId = if (raw.id or "") == "" then name else raw.id;
      kind = raw.kind or "taskRef";
      taskId = raw.taskId or "";
      task = if taskId != "" && builtins.hasAttr taskId tasks then tasks.${taskId} else null;
      workflowId = raw.workflowId or "";
      workflow =
        if workflowId != "" && builtins.hasAttr workflowId workflows then workflows.${workflowId} else null;
      serviceSetId = raw.serviceSetId or "";
      serviceSet =
        if serviceSetId != "" && builtins.hasAttr serviceSetId serviceSets then
          serviceSets.${serviceSetId}
        else
          null;
      operation = raw.operation or "";
      operationSummary = if serviceSet == null then "" else "${serviceSet.summary} ${operation}";
      operationDescription =
        if serviceSet == null then
          ""
        else if operation == "export" then
          "Prints per-service handoff data for '${serviceSet.name}'."
        else
          "Runs '${operation}' for the '${serviceSet.name}' service set.";
      targetAppId = raw.targetAppId or "";
      targetArgs = raw.targetArgs or [ ];
      setupAppIds = normalizeAppRefs (raw.setupAppIds or [ ]);
      teardownAppIds = normalizeAppRefs (raw.teardownAppIds or [ ]);
      contractRef = ((raw.validation or { }).contractRef or "");
      validationSchema =
        if contractRef == "" then
          null
        else if builtins.hasAttr contractRef contractBundle.validationSchemas then
          contractBundle.validationSchemas.${contractRef}
        else
          throw "nixfied apps: machineOutput app '${appId}' references unknown contractRef '${contractRef}'";
      targetPreview = if targetAppId != "" && appExists targetAppId then appPreview targetAppId else null;
      referencedAppIds =
        setupAppIds ++ teardownAppIds ++ lib.optionals (targetAppId != "") [ targetAppId ];
      unknownReferencedAppIds = builtins.filter (appRef: !(appExists appRef)) referencedAppIds;
      nestedMachineOutputRefs = builtins.filter isMachineOutputPreview referencedAppIds;
      machineSummary =
        if targetPreview == null then "Machine output ${appId}" else "Machine output ${targetAppId}";
    in
    if appId == "" then
      throw "nixfied apps: app '${name}' must have a non-empty id"
    else if kind == "taskRef" then
      if taskId == "" then
        throw "nixfied apps: taskRef app '${appId}' must set taskId"
      else if task == null then
        throw "nixfied apps: app '${appId}' references unknown task '${taskId}'"
      else
        canonical.canonicalize {
          id = appId;
          kind = "taskRef";
          taskId = taskId;
          summary = if (raw.summary or "") != "" then raw.summary else task.summary or appId;
          description = if (raw.description or "") != "" then raw.description else task.description or "";
          category = raw.category or "core";
          usage = raw.usage or [ ];
          examples = raw.examples or [ ];
          ownerFile = if (raw.ownerFile or null) == null || raw.ownerFile == "" then null else raw.ownerFile;
        }
    else if kind == "workflowRef" then
      if workflowId == "" then
        throw "nixfied apps: workflowRef app '${appId}' must set workflowId"
      else if workflow == null then
        throw "nixfied apps: app '${appId}' references unknown workflow '${workflowId}'"
      else
        canonical.canonicalize {
          id = appId;
          kind = "workflowRef";
          workflowId = workflowId;
          summary = if (raw.summary or "") != "" then raw.summary else workflow.summary or appId;
          description = if (raw.description or "") != "" then raw.description else workflow.description or "";
          category = raw.category or "core";
          usage = raw.usage or [ ];
          examples = raw.examples or [ ];
          ownerFile = if (raw.ownerFile or null) == null || raw.ownerFile == "" then null else raw.ownerFile;
        }
    else if kind == "serviceSetRef" then
      if serviceSetId == "" then
        throw "nixfied apps: serviceSetRef app '${appId}' must set serviceSetId"
      else if serviceSet == null then
        throw "nixfied apps: app '${appId}' references unknown service set '${serviceSetId}'"
      else if !(builtins.elem operation supportedServiceSetOperations) then
        throw "nixfied apps: serviceSetRef app '${appId}' must use a supported operation"
      else
        canonical.canonicalize {
          id = appId;
          kind = "serviceSetRef";
          serviceSetId = serviceSetId;
          operation = operation;
          summary = if (raw.summary or "") != "" then raw.summary else operationSummary;
          description = if (raw.description or "") != "" then raw.description else operationDescription;
          category = raw.category or "core";
          usage = raw.usage or [ ];
          examples = raw.examples or [ ];
          ownerFile = if (raw.ownerFile or null) == null || raw.ownerFile == "" then null else raw.ownerFile;
        }
    else if kind == "machineOutput" then
      if targetAppId == "" then
        throw "nixfied apps: machineOutput app '${appId}' must set targetAppId"
      else if contractRef == "" then
        throw "nixfied apps: machineOutput app '${appId}' must set validation.contractRef"
      else if targetAppId == appId then
        throw "nixfied apps: machineOutput app '${appId}' cannot target itself"
      else if unknownReferencedAppIds != [ ] then
        throw "nixfied apps: machineOutput app '${appId}' references unknown apps: ${builtins.concatStringsSep ", " unknownReferencedAppIds}"
      else if nestedMachineOutputRefs != [ ] then
        throw "nixfied apps: machineOutput app '${appId}' cannot reference machineOutput apps: ${builtins.concatStringsSep ", " nestedMachineOutputRefs}"
      else
        canonical.canonicalize {
          id = appId;
          kind = "machineOutput";
          targetAppId = targetAppId;
          targetArgs = targetArgs;
          setupAppIds = setupAppIds;
          teardownAppIds = teardownAppIds;
          validation = {
            inherit contractRef;
          }
          // lib.optionalAttrs (validationSchema != null) {
            schema = validationSchema;
          };
          summary = if (raw.summary or "") != "" then raw.summary else machineSummary;
          description =
            if (raw.description or "") != "" then
              raw.description
            else
              "Runs '${targetAppId}' behind a strict JSON output contract.";
          category = raw.category or "core";
          usage = raw.usage or [ ];
          examples = raw.examples or [ ];
          ownerFile = if (raw.ownerFile or null) == null || raw.ownerFile == "" then null else raw.ownerFile;
        }
    else
      throw "nixfied apps: unsupported app kind '${kind}' for '${appId}'";

  addApp =
    acc: name:
    let
      app = normalizeApp name;
      appId = app.id;
    in
    if builtins.hasAttr appId acc then
      throw "nixfied apps: duplicate app id '${appId}'"
    else
      acc
      // {
        ${appId} = app;
      };
in
builtins.foldl' addApp { } names
