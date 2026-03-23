{
  lib,
  config,
  ...
}:
let
  t = lib.types;
  statePolicyOptions = import ./lib/state-policy-options.nix { inherit lib; };
  serviceConfigLib = import ../framework/core/service-config.nix { inherit lib; };
  serviceRequirementType = t.enum serviceConfigLib.supportedServiceNames;

  configuredServices = config.nixfied.services or { };
  excludedServices = config.nixfied.graph.excludedServices or [ ];
  enabledServiceNames = builtins.filter (
    serviceName:
    (configuredServices.${serviceName}.enable or false) && !(builtins.elem serviceName excludedServices)
  ) (builtins.sort builtins.lessThan (builtins.attrNames configuredServices));

  serviceSetType = t.submodule (
    { name, ... }:
    {
      options = {
        id = lib.mkOption {
          type = t.str;
          default = "service-set.${name}";
        };

        summary = lib.mkOption {
          type = t.str;
          default = "Service set ${name}";
        };

        description = lib.mkOption {
          type = t.str;
          default = "";
        };

        services = {
          required = lib.mkOption {
            type = t.listOf serviceRequirementType;
            default = [ ];
          };

          optional = lib.mkOption {
            type = t.listOf serviceRequirementType;
            default = [ ];
          };
        };

        defaultOperation = lib.mkOption {
          type = t.enum [
            "health"
            "ready"
          ];
          default = "health";
        };

        state = {
          policy = lib.mkOption {
            type = t.nullOr (t.submodule { options = statePolicyOptions; });
            default = null;
          };
        };

        export = {
          defaultFormat = lib.mkOption {
            type = t.enum [
              "json"
              "env"
            ];
            default = "json";
          };
        };

        failureLogs = {
          capture = lib.mkOption {
            type = t.bool;
            default = true;
          };

          tailLines = lib.mkOption {
            type = t.ints.unsigned;
            default = 40;
          };
        };

        ownerFile = lib.mkOption {
          type = t.nullOr t.str;
          default = null;
        };
      };
    }
  );

  declaredServiceSets = config.nixfied.serviceSets or { };
  serviceSetNames = builtins.sort builtins.lessThan (builtins.attrNames declaredServiceSets);
  serviceSetOperationNames = [
    "start"
    "stop"
    "status"
    "health"
    "ready"
    "export"
  ];

  mkServiceSetApp =
    {
      appId,
      serviceSetId,
      operation,
      summary,
      description,
      usage ? [ ],
      examples ? [ ],
      ownerFile ? null,
      category ? "core",
    }:
    {
      kind = "serviceSetRef";
      inherit
        serviceSetId
        operation
        summary
        description
        usage
        examples
        ownerFile
        category
        ;
      id = appId;
    };

  mkNamedServiceSetApps =
    name:
    let
      serviceSet = declaredServiceSets.${name};
      serviceSetId = serviceSet.id;
      ownerFile = serviceSet.ownerFile;
      usageFor =
        operation:
        if operation == "export" then
          [ "nix run .#svcset::${name}::export -- --format json" ]
        else if operation == "health" || operation == "ready" then
          [ "nix run .#svcset::${name}::${operation} -- --service all" ]
        else
          [ "nix run .#svcset::${name}::${operation}" ];
      examplesFor = operation: usageFor operation;
    in
    builtins.listToAttrs (
      map (
        operation:
        let
          appId = "svcset::${name}::${operation}";
        in
        {
          name = appId;
          value = mkServiceSetApp {
            inherit
              appId
              serviceSetId
              operation
              ownerFile
              ;
            summary = "${serviceSet.summary} ${operation}";
            description = if serviceSet.description == "" then "" else "${serviceSet.description}\n";
            usage = usageFor operation;
            examples = examplesFor operation;
          };
        }
      ) serviceSetOperationNames
    );

  defaultAliasApps =
    if !(builtins.hasAttr "default" declaredServiceSets) then
      { }
    else
      let
        serviceSet = declaredServiceSets.default;
        serviceSetId = serviceSet.id;
        ownerFile = serviceSet.ownerFile;
      in
      {
        "services-start" = mkServiceSetApp {
          appId = "services-start";
          inherit serviceSetId ownerFile;
          operation = "start";
          summary = "Start the default service set";
          description = "Starts required services in the default service set.";
        };

        "services-stop" = mkServiceSetApp {
          appId = "services-stop";
          inherit serviceSetId ownerFile;
          operation = "stop";
          summary = "Stop the default service set";
          description = "Stops required services in the default service set.";
        };

        "services-status" = mkServiceSetApp {
          appId = "services-status";
          inherit serviceSetId ownerFile;
          operation = "status";
          summary = "Show status for the default service set";
          description = "Shows per-service status for the default service set.";
        };

        "services-export" = mkServiceSetApp {
          appId = "services-export";
          inherit serviceSetId ownerFile;
          operation = "export";
          summary = "Export the default service set contract";
          description = "Prints per-service handoff data for the default service set.";
          usage = [
            "nix run .#services-export -- --format json"
            "nix run .#services-export -- --format env"
          ];
          examples = [
            "nix run .#services-export -- --format json"
          ];
        };
      };

  generatedNamedApps = builtins.foldl' (
    acc: name: acc // mkNamedServiceSetApps name
  ) { } serviceSetNames;
in
{
  options.nixfied.serviceSets = lib.mkOption {
    type = t.attrsOf serviceSetType;
    default = { };
  };

  config = {
    nixfied.serviceSets.default = lib.mkDefault {
      id = "service-set.default";
      summary = "Default service set";
      description = "All enabled project services grouped behind stable lifecycle surfaces.";
      services.required = enabledServiceNames;
      services.optional = [ ];
      defaultOperation = "health";
      ownerFile = "nixfied/modules/service-sets.nix";
    };

    nixfied.apps = lib.mkMerge [
      generatedNamedApps
      defaultAliasApps
    ];
  };
}
