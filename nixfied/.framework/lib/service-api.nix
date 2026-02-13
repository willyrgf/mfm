# Nixfied service API contract helpers (validation + app/hook generation)
{
  pkgs,
  appApi ? null,
}:

let
  lib = pkgs.lib;
  validation = import ./validation.nix { inherit pkgs; };
  inherit (validation)
    isNonEmptyString
    isNonEmptyList
    expect
    isListOfNonEmptyStrings
    isKVSpec
    ;

  requiredProfiles = [
    "dev"
    "prod"
    "test"
    "ci"
  ];

  requiredCoreOps = [
    "init"
    "start"
    "stop"
    "restart"
    "status"
    "health"
    "check-config"
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
      ++ expect (
        !(op ? usage) || isListOfNonEmptyStrings (op.usage or null)
      ) "${prefix}: usage must be a list of non-empty strings"
      ++ expect (
        !(op ? examples) || isListOfNonEmptyStrings (op.examples or null)
      ) "${prefix}: examples must be a list of non-empty strings"
      ++ expect (
        !(op ? args) || (builtins.isList (op.args or null) && builtins.all isKVSpec (op.args or [ ]))
      ) "${prefix}: args must be a list of { name, description }"
      ++ expect (
        !(op ? env) || (builtins.isList (op.env or null) && builtins.all isKVSpec (op.env or [ ]))
      ) "${prefix}: env must be a list of { name, description }"
      ++ expect (
        !(op ? category) || isNonEmptyString (op.category or "")
      ) "${prefix}: category must be a non-empty string"
      ++ expect (!(op ? app) || builtins.isBool (op.app or null)) "${prefix}: app must be a boolean"
      ++ expect (
        !(op ? appName) || isNonEmptyString (op.appName or "")
      ) "${prefix}: appName must be a non-empty string"
      ++ expect (
        !(op ? hook) || isNonEmptyString (op.hook or "")
      ) "${prefix}: hook must be a non-empty string";

  opNames = ops: builtins.attrNames ops;

  opsOverlap =
    coreOps: extOps: builtins.filter (name: builtins.elem name (opNames extOps)) (opNames coreOps);

  validateServiceApiErrors =
    { serviceName, api }:
    let
      base = [ ];
      profiles = api.profiles or [ ];
      coreOps = api.coreOps or { };
      extOps = api.extensions or { };
      allOps = coreOps // extOps;
      overlap = opsOverlap coreOps extOps;
      missingProfiles = builtins.filter (p: !(builtins.elem p profiles)) requiredProfiles;
      missingCoreOps = builtins.filter (op: !(builtins.hasAttr op coreOps)) requiredCoreOps;
      opErrs = builtins.concatLists (
        map (
          name:
          validateOpErrors {
            service = serviceName;
            opName = name;
            op = allOps.${name};
          }
        ) (opNames allOps)
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
      ++ expect ((api.version or null) == 1) "${serviceName}: publicApi.version must be 1"
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
      ++ expect (api ? profiles) "${serviceName}: publicApi.profiles is required"
      ++ expect (isNonEmptyList (profiles)) "${serviceName}: publicApi.profiles must be a non-empty list"
      ++ expect (isListOfNonEmptyStrings profiles) "${serviceName}: publicApi.profiles must be a list of non-empty strings"
      ++
        expect (missingProfiles == [ ])
          "${serviceName}: publicApi.profiles missing required values: ${builtins.concatStringsSep ", " missingProfiles}"
      ++ expect (api ? coreOps) "${serviceName}: publicApi.coreOps is required"
      ++ expect (isAttrs coreOps) "${serviceName}: publicApi.coreOps must be an attribute set"
      ++
        expect (missingCoreOps == [ ])
          "${serviceName}: publicApi.coreOps missing required ops: ${builtins.concatStringsSep ", " missingCoreOps}"
      ++ expect (
        !(api ? extensions) || isAttrs extOps
      ) "${serviceName}: publicApi.extensions must be an attribute set"
      ++
        expect (overlap == [ ])
          "${serviceName}: publicApi.coreOps/extensions overlap on ops: ${builtins.concatStringsSep ", " overlap}"
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
        ${builtins.concatStringsSep "\n" (map (e: "  - " + e) errs)}

        Fix:
          - Define ${serviceName}.publicApi with:
            version=1, service="${serviceName}", profiles=["dev" "prod" "test" "ci"],
            coreOps={ init/start/stop/restart/status/health/check-config = { script, summary, details, ... }; },
            artifacts={ ... }.
      '';

  validateServiceApis =
    serviceApis:
    let
      names = lib.sort (a: b: a < b) (builtins.attrNames serviceApis);
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
        ${builtins.concatStringsSep "\n" (map (e: "  - " + e) errs)}
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

  serviceOps = api: (api.coreOps or { }) // (api.extensions or { });

  hookNameFor =
    service: opName: opCfg:
    let
      prefix = normalizeToken service;
      suffix = if opCfg ? hook then opCfg.hook else normalizeToken opName;
    in
    "${prefix}_${suffix}";

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

      REQUIRE_SLOT_ENV_CMD="''${REQUIRE_SLOT_ENV:-}"
      if [ -z "$REQUIRE_SLOT_ENV_CMD" ]; then
        echo "ERROR: REQUIRE_SLOT_ENV is not set; run via nixfied app/hook context." >&2
        exit 1
      fi
      if [ ! -x "$REQUIRE_SLOT_ENV_CMD" ]; then
        echo "ERROR: REQUIRE_SLOT_ENV is not executable: $REQUIRE_SLOT_ENV_CMD" >&2
        exit 1
      fi

      SLOT_ENV_OUT="$("$REQUIRE_SLOT_ENV_CMD")" || exit 1
      eval "$SLOT_ENV_OUT"

      exec ${toString opCfg.script} "$@"
    '';

  collectServiceOps =
    serviceApis:
    let
      names = lib.sort (a: b: a < b) (builtins.attrNames serviceApis);
      validated = validateServiceApis serviceApis;
      toOps =
        serviceName:
        let
          ops = serviceOps validated.${serviceName};
          opNamesSorted = lib.sort (a: b: a < b) (builtins.attrNames ops);
        in
        map (
          opName:
          let
            opCfg = ops.${opName};
            appName = if opCfg ? appName then opCfg.appName else "service::${serviceName}::${opName}";
          in
          {
            inherit
              serviceName
              opName
              opCfg
              appName
              ;
            hookName = hookNameFor serviceName opName opCfg;
            includeApp = opCfg.app or true;
            usage = if opCfg ? usage then opCfg.usage else [ "nix run .#${appName}" ];
            category = if opCfg ? category then opCfg.category else serviceName;
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
          api = {
            version = 1;
            summary = op.opCfg.summary;
            details = op.opCfg.details;
            usage = op.usage;
          }
          // lib.optionalAttrs (op.opCfg ? examples) { examples = op.opCfg.examples; }
          // lib.optionalAttrs (op.opCfg ? args) { args = op.opCfg.args; }
          // lib.optionalAttrs (op.opCfg ? env) { env = op.opCfg.env; }
          // {
            category = op.category;
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
    requiredProfiles
    requiredCoreOps
    validateServiceApi
    validateServiceApis
    validateEnabledServicesHaveContracts
    mkServiceHookEnvFromContract
    mkServiceAppsFromContract
    ;
}
