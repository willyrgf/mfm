{
  pkgs,
  system,
  projectRoot,
  projectModules,
  extraModules ? [ ],
  localOverrides ? [ ],
  selectedServices ? null,
  frameworkSourceRevision ? import ./framework-revision.nix {
    sourcePath = ../../.;
    metadataPath = ../../VENDORED.txt;
  },
}:
let
  canonical = import ./canonical.nix { inherit (pkgs) lib; };
  frameworkSourceFlakeRef = null;

  compiledCore = import ./mkCompiledCore.nix {
    inherit
      pkgs
      system
      projectRoot
      projectModules
      extraModules
      localOverrides
      frameworkSourceRevision
      ;
  };

  execution = import ./materializeExecution.nix {
    inherit
      pkgs
      projectRoot
      selectedServices
      frameworkSourceFlakeRef
      ;
    compiledCore = compiledCore;
  };

  coreSurfaces = import ./mkCoreSurfaces.nix {
    inherit
      pkgs
      canonical
      ;
    compiledCore = compiledCore;
  };

  apps =
    execution.baseApps
    // coreSurfaces.apps
    // {
      default = if execution.baseApps ? help then execution.baseApps.help else execution.baseApps.default;
    };

  packages = coreSurfaces.packages // {
    default = pkgs.runCommand "nixfied-default" { } ''
      mkdir -p "$out/bin"
      ln -s ${apps.default.program} "$out/bin/default"
    '';
  };
in
{
  model = compiledCore.model;
  stateHash = compiledCore.stateHash;
  runtimeHash = execution.runtimeHash or compiledCore.model.identity.evalHash;
  tasks = compiledCore.model.tasks;
  services = execution.services;
  serviceCatalog = compiledCore.model.serviceCatalog;
  workflows = compiledCore.model.workflows;
  features = compiledCore.model.features;
  selectionIndex = compiledCore.selectionIndex;
  serviceSurfaceCatalog = compiledCore.serviceSurfaceCatalog;
  serviceApis = execution.serviceApis;
  serviceHookEnv = execution.serviceHookEnv;

  inherit
    apps
    packages
    ;
  checks = coreSurfaces.checks;
  devShells = coreSurfaces.devShells;
  schema = coreSurfaces.schema;
}
