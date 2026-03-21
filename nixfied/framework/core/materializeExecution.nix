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
  registry = import ../runtime/registry { inherit pkgs; };
  compileServices = import ../../compiler/compile-services.nix { inherit lib; };

  services = compileServices {
    inherit
      pkgs
      selectedServices
      ;
    discardContext = false;
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
    serviceApps = serviceRuntimeSurfaces.serviceApps;
    serviceHookEnv = serviceRuntimeSurfaces.serviceHookEnv;
  };
in
{
  inherit
    services
    runtimeHash
    baseApps
    ;
  serviceApis = serviceRuntimeSurfaces.serviceApis;
  serviceApps = serviceRuntimeSurfaces.serviceApps;
  serviceHookEnv = serviceRuntimeSurfaces.serviceHookEnv;
}
