{
  pkgs,
  projectRoot,
  compiledCore,
  selectedServices ? null,
  frameworkSourceFlakeRef ? null,
}:
let
  lib = pkgs.lib;
  canonical = import ./canonical.nix { inherit lib; };
  mkShellApp = import ./mk-shell-app.nix { inherit pkgs; };
  registry = import ../runtime/registry { inherit pkgs; };
  compileServices = import ../../compiler/compile-services.nix { inherit lib; };
  normalizedSelectedServices =
    if selectedServices == null then
      null
    else
      builtins.sort builtins.lessThan (lib.unique (builtins.filter (name: name != "") selectedServices));

  selectedServiceScopeSet =
    if normalizedSelectedServices == null then
      { }
    else
      builtins.listToAttrs (
        map (serviceName: {
          name = serviceName;
          value = true;
        }) normalizedSelectedServices
      );

  isSelectedServiceAllowed =
    serviceName:
    normalizedSelectedServices == null
    || builtins.hasAttr serviceName selectedServiceScopeSet
    || builtins.hasAttr "service.${serviceName}" selectedServiceScopeSet
    || (
      lib.hasPrefix "service." serviceName
      && builtins.hasAttr (lib.removePrefix "service." serviceName) selectedServiceScopeSet
    );

  constrainSelectedServices =
    serviceNames:
    let
      normalized = builtins.sort builtins.lessThan (
        lib.unique (builtins.filter (name: name != "") serviceNames)
      );
    in
    if normalizedSelectedServices == null then
      normalized
    else
      builtins.filter isSelectedServiceAllowed normalized;

  enabledServiceFlags = builtins.listToAttrs (
    map (
      serviceId:
      let
        service = compiledCore.model.serviceCatalog.${serviceId};
      in
      {
        name = service.name or (lib.removePrefix "service." serviceId);
        value = service.enable or false;
      }
    ) (builtins.attrNames compiledCore.model.serviceCatalog)
  );

  services = compileServices {
    inherit
      pkgs
      selectedServices
      enabledServiceFlags
      ;
    resolved = compiledCore.resolved;
  };

  runtimeHash = canonical.hashCanonical {
    schema = {
      kind = "nixfied-runtime";
      version = 1;
    };
    services = services;
  };

  serviceRuntimeSurfaces = import ./mkServiceRuntimeSurfaces.nix {
    inherit
      pkgs
      selectedServices
      ;
    model = compiledCore.model;
    services = services;
  };

  manifestBackedAppPrograms = builtins.mapAttrs (
    appId: manifest:
    let
      app = compiledCore.model.apps.${appId};
      effectiveSelectedServices = constrainSelectedServices (manifest.selectedServices or [ ]);
      appServices = compileServices {
        inherit
          pkgs
          enabledServiceFlags
          ;
        resolved = compiledCore.resolved;
        selectedServices = effectiveSelectedServices;
      };

      appRuntimeHash = canonical.hashCanonical {
        schema = {
          kind = "nixfied-runtime";
          version = 1;
        };
        services = appServices;
      };

      appServiceRuntimeSurfaces = import ./mkServiceRuntimeSurfaces.nix {
        inherit
          pkgs
          ;
        model = manifest.model;
        selectedServices = effectiveSelectedServices;
        services = appServices;
      };

      appOrchestrator = import ../runtime/orchestrator.nix {
        inherit
          pkgs
          registry
          projectRoot
          ;
        model = manifest.model;
        services = appServices;
        runtimeHash = appRuntimeHash;
        serviceHookEnv = appServiceRuntimeSurfaces.serviceHookEnv;
        inherit serviceSetPrograms;
      };
    in
    (mkShellApp {
      appName = "app-runtime:${appId}";
      binPrefix = "nixfied-app-runtime";
      body = ''
        exec ${appOrchestrator}/bin/nixfied-orchestrator ${
          if (app.kind or "") == "workflowRef" then "run-workflow" else "run-task"
        } ${
          lib.escapeShellArg (if (app.kind or "") == "workflowRef" then app.workflowId else app.taskId)
        } "$@"
      '';
    }).program
  ) (compiledCore.appExecutionManifests or { });

  serviceSetPrograms = builtins.mapAttrs (
    serviceSetId: serviceSet:
    let
      effectiveSelectedServices = constrainSelectedServices (serviceSet.services.all or [ ]);
      serviceSetServices = compileServices {
        inherit
          pkgs
          enabledServiceFlags
          ;
        resolved = compiledCore.resolved;
        selectedServices = effectiveSelectedServices;
      };

      serviceSetModel = compiledCore.model // {
        runtime = compiledCore.model.runtime // {
          directories = (compiledCore.model.runtime.directories or { }) // {
            base = serviceSet.state.policy.runtimeBase;
          };
        };
        state = compiledCore.model.state // {
          policy = serviceSet.state.policy;
        };
      };

      serviceSetRuntimeSurfaces = import ./mkServiceRuntimeSurfaces.nix {
        inherit pkgs;
        model = serviceSetModel;
        services = serviceSetServices;
        selectedServices = effectiveSelectedServices;
      };
    in
    import ./mkServiceSetPrograms.nix {
      inherit
        pkgs
        serviceSet
        ;
      model = serviceSetModel;
      resolvedServices = compiledCore.resolved.services or { };
      serviceRuntimeSurfaces = serviceSetRuntimeSurfaces;
    }
  ) (compiledCore.serviceSets or { });

  appPrograms = builtins.mapAttrs (
    appId: app:
    if builtins.hasAttr appId manifestBackedAppPrograms then
      manifestBackedAppPrograms.${appId}
    else if (app.kind or "") == "serviceSetRef" then
      serviceSetPrograms.${app.serviceSetId}.programsByOperation.${app.operation}.program
    else if (app.kind or "") == "machineOutput" then
      import ./mkMachineOutputPrograms.nix {
        inherit
          pkgs
          appId
          app
          ;
        targetProgram = appPrograms.${app.targetAppId};
        setupPrograms = map (setupAppId: appPrograms.${setupAppId}) (app.setupAppIds or [ ]);
        teardownPrograms = map (teardownAppId: appPrograms.${teardownAppId}) (app.teardownAppIds or [ ]);
      }
    else
      throw "materializeExecution: unsupported app kind '${app.kind or ""}' for '${appId}'"
  ) (compiledCore.model.apps or { });

  runner = import ../runtime {
    inherit
      pkgs
      projectRoot
      registry
      ;
  };

  baseApps = runner.mkApps {
    model = compiledCore.model;
    selectionIndex = compiledCore.selectionIndex;
    services = services;
    inherit
      runtimeHash
      frameworkSourceFlakeRef
      ;
    appPrograms = appPrograms;
    serviceSetPrograms = serviceSetPrograms;
    serviceApps = serviceRuntimeSurfaces.serviceApps;
    serviceHookEnv = serviceRuntimeSurfaces.serviceHookEnv;
  };
in
{
  inherit
    services
    runtimeHash
    appPrograms
    serviceSetPrograms
    baseApps
    ;
  serviceApis = serviceRuntimeSurfaces.serviceApis;
  serviceApps = serviceRuntimeSurfaces.serviceApps;
  serviceHookEnv = serviceRuntimeSurfaces.serviceHookEnv;
}
