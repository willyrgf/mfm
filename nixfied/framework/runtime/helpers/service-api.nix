# Nixfied service API contract helpers (validation + app/hook generation)
{
  pkgs,
  appApi ? null,
  shellContract ? import ./shell-contract.nix { inherit pkgs; },
}:

let
  validation = import ./validation.nix { inherit pkgs; };
  inherit (validation)
    isNonEmptyString
    expect
    renderErrors
    sortedAttrNames
    optionalAttrSatisfies
    isListOfNonEmptyStrings
    isKVSpecList
    ;
  requiredLifecycleOps = [
    "start"
    "stop"
    "status"
  ];
  validCommandClasses = [
    "typed"
    "passthrough"
    "json"
    "batch-runner"
  ];
  runtimeLogLevels = shellContract.runtimeLogLevels;
  runtimeOutputModes = shellContract.runtimeOutputModes;
  runtimeLogLevelEnvName = shellContract.runtimeLogLevelEnvName;
  runtimeLogLevelAliases = shellContract.runtimeLogLevelAliases;
  runtimeLogLevelDefault = shellContract.runtimeLogLevelDefault;
  runtimeOutputModeEnvName = shellContract.runtimeOutputModeEnvName;
  runtimeOutputModeAliases = shellContract.runtimeOutputModeAliases;
  runtimeOutputModeDefault = shellContract.runtimeOutputModeDefault;
  supportedRuntimePrimitiveKeys = [
    "version"
    "logLevel"
    "outputMode"
  ];

  isAttrs = x: builtins.isAttrs x;
  normalizeToken =
    x: pkgs.lib.strings.toUpper (pkgs.lib.replaceStrings [ "-" "." ":" ] [ "_" "_" "_" ] x);
  normalizeStringSet = values: pkgs.lib.sort (a: b: a < b) (pkgs.lib.unique values);
  sameStringSet = expected: actual: normalizeStringSet expected == normalizeStringSet actual;
  isListOfOpNames = values: builtins.isList values && isListOfNonEmptyStrings values;

  isScriptLike = x: (builtins.isString x) || (builtins.isPath x) || (builtins.isAttrs x);
  opPreRefs = op: op.preOps or [ ];
  opPostRefs = op: op.postOps or [ ];
  opRefs = op: (opPreRefs op) ++ (opPostRefs op);
  opHasComposition = op: opRefs op != [ ];

  validateOpErrors =
    {
      service,
      opName,
      op,
    }:
    let
      prefix = "${service}.${opName}";
    in
    if !isAttrs op then
      [ "${prefix}: op must be an attribute set" ]
    else
      expect (
        (op ? script) || opHasComposition op
      ) "${prefix}: script is required when preOps/postOps are not defined"
      ++ expect (
        !(op ? script) || isScriptLike (op.script or null)
      ) "${prefix}: script must be string/path/derivation"
      ++ expect (op ? summary) "${prefix}: summary is required"
      ++ expect (isNonEmptyString (op.summary or "")) "${prefix}: summary must be a non-empty string"
      ++ expect (op ? details) "${prefix}: details is required"
      ++ expect (builtins.isString (op.details or null)) "${prefix}: details must be a string"
      ++ expect (optionalAttrSatisfies op "preOps"
        isListOfOpNames
      ) "${prefix}: preOps must be a list of non-empty strings"
      ++ expect (optionalAttrSatisfies op "postOps"
        isListOfOpNames
      ) "${prefix}: postOps must be a list of non-empty strings"
      ++ expect (optionalAttrSatisfies op "usage"
        isListOfNonEmptyStrings
      ) "${prefix}: usage must be a list of non-empty strings"
      ++ expect (optionalAttrSatisfies op "examples"
        isListOfNonEmptyStrings
      ) "${prefix}: examples must be a list of non-empty strings"
      ++ expect (optionalAttrSatisfies op "args"
        isKVSpecList
      ) "${prefix}: args must be a list of { name, description }"
      ++ expect (optionalAttrSatisfies op "env"
        isKVSpecList
      ) "${prefix}: env must be a list of { name, description }"
      ++ expect (optionalAttrSatisfies op "category"
        isNonEmptyString
      ) "${prefix}: category must be a non-empty string"
      ++ expect (optionalAttrSatisfies op "class"
        isNonEmptyString
      ) "${prefix}: class must be a non-empty string"
      ++ expect (
        !(op ? class) || builtins.elem op.class validCommandClasses
      ) "${prefix}: class must be one of ${builtins.concatStringsSep ", " validCommandClasses}"
      ++ expect (optionalAttrSatisfies op "idempotent"
        builtins.isBool
      ) "${prefix}: idempotent must be a boolean"
      ++ expect (optionalAttrSatisfies op "exposeApp"
        builtins.isBool
      ) "${prefix}: exposeApp must be a boolean"
      ++ expect (optionalAttrSatisfies op "exposeHook"
        builtins.isBool
      ) "${prefix}: exposeHook must be a boolean"
      ++ expect (optionalAttrSatisfies op "appName"
        isNonEmptyString
      ) "${prefix}: appName must be a non-empty string"
      ++ expect (optionalAttrSatisfies op "hook"
        isNonEmptyString
      ) "${prefix}: hook must be a non-empty string";

  validateOpCompositionErrors =
    {
      serviceName,
      ops,
    }:
    let
      names = opNames ops;
      nameSet = builtins.listToAttrs (
        map (name: {
          inherit name;
          value = true;
        }) names
      );
      refErrorsFor =
        opName: field:
        let
          refs = ops.${opName}.${field} or [ ];
          unknown = builtins.filter (ref: !(builtins.hasAttr ref nameSet)) refs;
        in
        map (ref: "${serviceName}.${opName}: ${field} references unknown op '${ref}'") unknown;
      refErrs = builtins.concatLists (
        map (opName: (refErrorsFor opName "preOps") ++ (refErrorsFor opName "postOps")) names
      );
      cycleErrorsFor =
        path: current:
        let
          refs = builtins.filter (ref: builtins.hasAttr ref nameSet) (opRefs ops.${current});
        in
        builtins.concatLists (
          map (
            ref:
            if builtins.elem ref path then
              [
                "${serviceName}.${current}: op composition contains a cycle: ${
                  builtins.concatStringsSep " -> " (path ++ [ ref ])
                }"
              ]
            else
              cycleErrorsFor (path ++ [ ref ]) ref
          ) refs
        );
      cycleErrs = pkgs.lib.unique (
        builtins.concatLists (map (opName: cycleErrorsFor [ opName ] opName) names)
      );
    in
    refErrs ++ cycleErrs;

  validateRuntimePrimitiveSpecErrors =
    {
      serviceName,
      primitiveName,
      primitiveSpec,
      expectedEnv,
      expectedAliases,
      expectedValues,
    }:
    let
      prefix = "${serviceName}: publicApi.runtimePrimitives.${primitiveName}";
      envName = primitiveSpec.env or "";
      aliases = primitiveSpec.aliases or [ ];
      values = primitiveSpec.values or [ ];
      defaultValue = primitiveSpec.default or null;
    in
    if primitiveSpec == null then
      [ "${prefix} is required" ]
    else if !isAttrs primitiveSpec then
      [ "${prefix} must be an attribute set" ]
    else
      expect (primitiveSpec ? env) "${prefix}.env is required"
      ++ expect (isNonEmptyString envName) "${prefix}.env must be a non-empty string"
      ++ expect (envName == expectedEnv) "${prefix}.env must be ${expectedEnv}"
      ++ expect (primitiveSpec ? aliases) "${prefix}.aliases is required"
      ++ expect (
        builtins.isList aliases && isListOfNonEmptyStrings aliases
      ) "${prefix}.aliases must be a list of non-empty strings"
      ++ expect (sameStringSet expectedAliases aliases) "${prefix}.aliases must be [${builtins.concatStringsSep ", " expectedAliases}]"
      ++ expect (primitiveSpec ? values) "${prefix}.values is required"
      ++ expect (
        builtins.isList values && isListOfNonEmptyStrings values
      ) "${prefix}.values must be a list of non-empty strings"
      ++ expect (sameStringSet expectedValues values) "${prefix}.values must be [${builtins.concatStringsSep ", " expectedValues}]"
      ++ expect (primitiveSpec ? default) "${prefix}.default is required"
      ++ expect (isNonEmptyString (toString defaultValue)) "${prefix}.default must be a non-empty string"
      ++ expect (builtins.elem defaultValue expectedValues) "${prefix}.default must be one of [${builtins.concatStringsSep ", " expectedValues}]";

  validateRuntimePrimitivesErrors =
    {
      serviceName,
      runtimePrimitives,
    }:
    let
      keys =
        if runtimePrimitives == null || !isAttrs runtimePrimitives then
          [ ]
        else
          builtins.attrNames runtimePrimitives;
      unknownKeys = builtins.filter (k: !(builtins.elem k supportedRuntimePrimitiveKeys)) keys;
      logLevelSpec =
        if runtimePrimitives != null && isAttrs runtimePrimitives then
          runtimePrimitives.logLevel or null
        else
          null;
      outputModeSpec =
        if runtimePrimitives != null && isAttrs runtimePrimitives then
          runtimePrimitives.outputMode or null
        else
          null;
    in
    if runtimePrimitives == null then
      [ "${serviceName}: publicApi.runtimePrimitives is required" ]
    else if !isAttrs runtimePrimitives then
      [ "${serviceName}: publicApi.runtimePrimitives must be an attribute set" ]
    else
      expect (
        runtimePrimitives ? version
      ) "${serviceName}: publicApi.runtimePrimitives.version is required"
      ++ expect (builtins.isInt (
        runtimePrimitives.version or null
      )) "${serviceName}: publicApi.runtimePrimitives.version must be an integer"
      ++ expect (
        (runtimePrimitives.version or null) == 1
      ) "${serviceName}: publicApi.runtimePrimitives.version must be 1"
      ++
        expect (unknownKeys == [ ])
          "${serviceName}: publicApi.runtimePrimitives contains unsupported keys: ${builtins.concatStringsSep ", " unknownKeys}"
      ++ validateRuntimePrimitiveSpecErrors {
        inherit serviceName;
        primitiveName = "logLevel";
        primitiveSpec = logLevelSpec;
        expectedEnv = runtimeLogLevelEnvName;
        expectedAliases = runtimeLogLevelAliases;
        expectedValues = runtimeLogLevels;
      }
      ++ validateRuntimePrimitiveSpecErrors {
        inherit serviceName;
        primitiveName = "outputMode";
        primitiveSpec = outputModeSpec;
        expectedEnv = runtimeOutputModeEnvName;
        expectedAliases = runtimeOutputModeAliases;
        expectedValues = runtimeOutputModes;
      };

  opNames = ops: builtins.attrNames ops;

  validateServiceApiErrors =
    { serviceName, api }:
    let
      base = [ ];
      version = api.version or null;
      profiles = api.profiles or [ ];
      ops = api.operations or { };
      runtimePrimitives = api.runtimePrimitives or null;
      missingLifecycleOps = builtins.filter (op: !(builtins.hasAttr op ops)) requiredLifecycleOps;
      opErrs = builtins.concatLists (
        map (
          name:
          validateOpErrors {
            service = serviceName;
            opName = name;
            op = ops.${name};
          }
        ) (opNames ops)
      );
      compositionErrs = validateOpCompositionErrors { inherit serviceName ops; };
      runtimeErrs = validateRuntimePrimitivesErrors { inherit serviceName runtimePrimitives; };
    in
    if api == null then
      [ "${serviceName}: missing publicApi" ]
    else if !isAttrs api then
      [ "${serviceName}: publicApi must be an attribute set" ]
    else
      base
      ++ expect (api ? version) "${serviceName}: publicApi.version is required"
      ++ expect (builtins.isInt (
        api.version or null
      )) "${serviceName}: publicApi.version must be an integer"
      ++ expect (version == 3) "${serviceName}: publicApi.version must be 3"
      ++ expect (api ? service) "${serviceName}: publicApi.service is required"
      ++ expect (isNonEmptyString (
        api.service or ""
      )) "${serviceName}: publicApi.service must be a non-empty string"
      ++ expect (
        (api.service or "") == serviceName
      ) "${serviceName}: publicApi.service must match service key (${serviceName})"
      ++ expect (api ? summary) "${serviceName}: publicApi.summary is required"
      ++ expect (isNonEmptyString (
        api.summary or ""
      )) "${serviceName}: publicApi.summary must be a non-empty string"
      ++ expect (api ? details) "${serviceName}: publicApi.details is required"
      ++ expect (builtins.isString (
        api.details or null
      )) "${serviceName}: publicApi.details must be a string"
      ++ expect (
        !(api ? profiles) || isListOfNonEmptyStrings profiles
      ) "${serviceName}: publicApi.profiles must be a list of non-empty strings when set"
      ++ expect (api ? operations) "${serviceName}: publicApi.operations is required"
      ++ expect (isAttrs ops) "${serviceName}: publicApi.operations must be an attribute set"
      ++
        expect (missingLifecycleOps == [ ])
          "${serviceName}: publicApi.operations missing required lifecycle ops: ${builtins.concatStringsSep ", " missingLifecycleOps}"
      ++ expect (api ? artifacts) "${serviceName}: publicApi.artifacts is required"
      ++ expect (isAttrs (
        api.artifacts or null
      )) "${serviceName}: publicApi.artifacts must be an attribute set"
      ++ opErrs
      ++ compositionErrs
      ++ runtimeErrs;

  validateServiceApi =
    { serviceName, api }:
    let
      errs = validateServiceApiErrors { inherit serviceName api; };
    in
    if errs == [ ] then
      api
    else
      throw ''
        Nixfied service API contract violated for "${serviceName}":
        ${renderErrors errs}

        Fix:
          - Define ${serviceName}.publicApi with:
            - version=3 + operations + artifacts
      '';

  validateServiceApis =
    serviceApis:
    let
      names = sortedAttrNames serviceApis;
      errs = builtins.concatLists (
        map (
          serviceName:
          validateServiceApiErrors {
            inherit serviceName;
            api = serviceApis.${serviceName};
          }
        ) names
      );
    in
    if errs == [ ] then
      serviceApis
    else
      throw ''
        Nixfied service API contract violated:
        ${renderErrors errs}
      '';

  validateEnabledServicesHaveContracts =
    {
      enabledServices,
      serviceApis,
    }:
    let
      missing = builtins.filter (name: !(builtins.hasAttr name serviceApis)) enabledServices;
      _ = validateServiceApis serviceApis;
    in
    if missing == [ ] then
      serviceApis
    else
      throw ''
        Nixfied service API contract violated:
          - Missing publicApi for enabled services: ${builtins.concatStringsSep ", " missing}
      '';

  mkRuntimePrimitivesV1 =
    {
      logLevelDefault ? runtimeLogLevelDefault,
      outputModeDefault ? runtimeOutputModeDefault,
    }:
    shellContract.mkServiceRuntimePrimitivesV1 {
      inherit
        logLevelDefault
        outputModeDefault
        ;
    };

  mkServiceApiV3 =
    {
      service,
      summary,
      details,
      artifacts,
      operations,
      profiles ? [ ],
      runtimePrimitives ? mkRuntimePrimitivesV1 { },
    }:
    {
      version = 3;
      inherit
        service
        summary
        details
        profiles
        artifacts
        operations
        runtimePrimitives
        ;
    };

  mkServiceApisFromModules =
    modules:
    let
      names = sortedAttrNames modules;
      pairs = builtins.concatLists (
        map (
          name:
          let
            mod = modules.${name};
          in
          if mod == null then
            [ ]
          else
            [
              {
                inherit name;
                value = mod.publicApi or null;
              }
            ]
        ) names
      );
    in
    builtins.listToAttrs pairs;

  serviceOps = api: api.operations or { };

  hookNameFor =
    service: opName: opCfg:
    let
      prefix = normalizeToken service;
      suffix = if opCfg ? hook then opCfg.hook else normalizeToken opName;
    in
    "SVC_${prefix}_${suffix}";

  sanitizeScriptToken = x: pkgs.lib.replaceStrings [ "/" ":" "." " " ] [ "-" "-" "-" "-" ] x;

  launcherNameFor =
    serviceName: opName: "service-op-${sanitizeScriptToken serviceName}-${sanitizeScriptToken opName}";

  noopScriptFor =
    serviceName: opName:
    pkgs.writeShellScript (launcherNameFor serviceName "${opName}-noop") ''
      exit 0
    '';

  scriptForOp =
    serviceName: opName: opCfg:
    if opCfg ? script then opCfg.script else noopScriptFor serviceName opName;

  buildExecutionPlan =
    {
      serviceName,
      ops,
      opName,
      passArgs ? true,
    }:
    let
      opCfg = ops.${opName};
      mkNestedPlan =
        ref:
        buildExecutionPlan {
          inherit
            serviceName
            ops
            ;
          opName = ref;
          passArgs = false;
        };
    in
    builtins.concatLists (map mkNestedPlan (opPreRefs opCfg))
    ++ [
      {
        inherit
          opName
          passArgs
          ;
        script = scriptForOp serviceName opName opCfg;
      }
    ]
    ++ builtins.concatLists (map mkNestedPlan (opPostRefs opCfg));

  mkServiceOpLauncher =
    {
      serviceName,
      opName,
      plan,
      runtimePrimitives,
    }:
    let
      logLevelDefault = runtimePrimitives.logLevel.default or runtimeLogLevelDefault;
      outputModeDefault = runtimePrimitives.outputMode.default or runtimeOutputModeDefault;
      renderPlanStep =
        step: if step.passArgs then ''${toString step.script} "$@"'' else "${toString step.script}";
    in
    pkgs.writeShellScript (launcherNameFor serviceName opName) ''
      set -euo pipefail

      source ${toString shellContract.runtime}
      nixfied_contract_resolve_runtime_primitives "${logLevelDefault}" "${outputModeDefault}"

      ${builtins.concatStringsSep "\n" (map renderPlanStep plan)}
    '';

  collectServiceOps =
    serviceApis:
    let
      names = sortedAttrNames serviceApis;
      validated = validateServiceApis serviceApis;
      toOps =
        serviceName:
        let
          ops = serviceOps validated.${serviceName};
          opNamesSorted = sortedAttrNames ops;
        in
        map (
          opName:
          let
            opCfg = ops.${opName};
            appName = if opCfg ? appName then opCfg.appName else "svc::${serviceName}::${opName}";
            opRuntimePrimitives = validated.${serviceName}.runtimePrimitives;
            plan = buildExecutionPlan {
              inherit serviceName ops opName;
            };
          in
          {
            inherit
              serviceName
              opName
              opCfg
              appName
              plan
              ;
            hookName = hookNameFor serviceName opName opCfg;
            includeApp = opCfg.exposeApp or true;
            usage = if opCfg ? usage then opCfg.usage else [ "nix run .#${appName}" ];
            category = if opCfg ? category then opCfg.category else serviceName;
            class = opCfg.class or "passthrough";
            idempotent = opCfg.idempotent or false;
            includeHook = opCfg.exposeHook or true;
            runtimePrimitives = opRuntimePrimitives;
            launcher = mkServiceOpLauncher {
              inherit
                serviceName
                opName
                plan
                ;
              runtimePrimitives = opRuntimePrimitives;
            };
          }
        ) opNamesSorted;
    in
    builtins.concatLists (map toOps names);

  mkServiceHookEnvFromContract =
    serviceApis:
    let
      ops = builtins.filter (op: op.includeHook) (collectServiceOps serviceApis);
      pairs = map (op: {
        name = op.hookName;
        value = toString op.launcher;
      }) ops;
      dedup =
        acc: pair:
        if builtins.hasAttr pair.name acc then
          throw "Nixfied service API hook name collision: ${pair.name}"
        else
          acc
          // (builtins.listToAttrs [
            {
              name = pair.name;
              value = pair.value;
            }
          ]);
    in
    builtins.foldl' dedup { } pairs;

  mkServiceAppsFromContract =
    serviceApis:
    let
      _ = if appApi == null then throw "mkServiceAppsFromContract requires appApi" else null;
      ops = builtins.filter (op: op.includeApp) (collectServiceOps serviceApis);
      pairs = map (op: {
        name = op.appName;
        value = appApi.mkNixfiedApp {
          name = op.appName;
          script = ''
            exec ${toString op.launcher} "$@"
          '';
          env = { };
          useDeps = false;
          api = appApi.mkCommandApi {
            class = op.class;
            name = op.appName;
            summary = op.opCfg.summary;
            details = op.opCfg.details;
            usage = op.usage;
            examples = op.opCfg.examples or [ ];
            args = op.opCfg.args or [ ];
            env = op.opCfg.env or [ ];
            category = op.category;
            idempotent = op.idempotent;
          };
          meta = {
            nixfied = {
              service = op.serviceName;
              operation = op.opName;
            };
          };
        };
      }) ops;
    in
    builtins.listToAttrs pairs;
in
{
  inherit
    collectServiceOps
    validateServiceApi
    validateServiceApis
    validateEnabledServicesHaveContracts
    mkRuntimePrimitivesV1
    mkServiceApiV3
    mkServiceApisFromModules
    mkServiceHookEnvFromContract
    mkServiceAppsFromContract
    ;
}
