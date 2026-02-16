# Nixfied service API contract helpers (validation + app/hook generation)
{
  pkgs,
  appApi ? null,
}:

let
  lib = pkgs.lib;
  validation = import ./validation.nix { inherit pkgs; };
  slotEnvRuntime = import ./slot-env-runtime.nix { inherit pkgs; };
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

  isAttrs = x: builtins.isAttrs x;
  normalizeToken =
    x: pkgs.lib.strings.toUpper (pkgs.lib.replaceStrings [ "-" "." ":" ] [ "_" "_" "_" ] x);

  isScriptLike = x: (builtins.isString x) || (builtins.isPath x) || (builtins.isAttrs x);

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
      expect (op ? script) "${prefix}: script is required"
      ++ expect (isScriptLike (op.script or null)) "${prefix}: script must be string/path/derivation"
      ++ expect (op ? summary) "${prefix}: summary is required"
      ++ expect (isNonEmptyString (op.summary or "")) "${prefix}: summary must be a non-empty string"
      ++ expect (op ? details) "${prefix}: details is required"
      ++ expect (builtins.isString (op.details or null)) "${prefix}: details must be a string"
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
      ++ expect (optionalAttrSatisfies op "appName"
        isNonEmptyString
      ) "${prefix}: appName must be a non-empty string"
      ++ expect (optionalAttrSatisfies op "hook"
        isNonEmptyString
      ) "${prefix}: hook must be a non-empty string";

  opNames = ops: builtins.attrNames ops;

  validateServiceApiErrors =
    { serviceName, api }:
    let
      base = [ ];
      version = api.version or null;
      profiles = api.profiles or [ ];
      ops = api.operations or { };
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
      ++ opErrs;

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

  mkServiceApiV3 =
    {
      service,
      summary,
      details,
      artifacts,
      operations,
      profiles ? [ ],
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

  mkServiceOpLauncher =
    {
      serviceName,
      opName,
      opCfg,
    }:
    pkgs.writeShellScript (launcherNameFor serviceName opName) ''
      set -euo pipefail

      ${slotEnvRuntime.requireSlotEnvJson { }}

      exec ${toString opCfg.script} "$@"
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
          in
          {
            inherit
              serviceName
              opName
              opCfg
              appName
              ;
            hookName = hookNameFor serviceName opName opCfg;
            includeApp = opCfg.exposeApp or true;
            usage = if opCfg ? usage then opCfg.usage else [ "nix run .#${appName}" ];
            category = if opCfg ? category then opCfg.category else serviceName;
            class = opCfg.class or "passthrough";
            idempotent = opCfg.idempotent or false;
            launcher = mkServiceOpLauncher {
              inherit
                serviceName
                opName
                opCfg
                ;
            };
          }
        ) opNamesSorted;
    in
    builtins.concatLists (map toOps names);

  mkServiceHookEnvFromContract =
    serviceApis:
    let
      ops = collectServiceOps serviceApis;
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
    validateServiceApi
    validateServiceApis
    validateEnabledServicesHaveContracts
    mkServiceApiV3
    mkServiceApisFromModules
    mkServiceHookEnvFromContract
    mkServiceAppsFromContract
    ;
}
