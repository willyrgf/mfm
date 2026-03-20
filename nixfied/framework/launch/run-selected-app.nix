{
  system,
  projectRoot,
  frameworkRoot,
  nixpkgsPath,
  frameworkSourceRevision ? "unknown",
  appName,
  excludedServicesCsv ? "",
  selectedServicesCsv ? "__ALL__",
  projectModuleSpecsJson ? "[]",
  extraModuleSpecsJson ? "[]",
  localOverrideSpecsJson ? "[]",
}:
let
  toAbsPath =
    value:
    if builtins.isPath value then
      value
    else if builtins.isString value && value != "" && builtins.substring 0 1 value == "/" then
      /. + value
    else
      throw "nixfied launcher requires an absolute path string";

  projectRootPath = toAbsPath projectRoot;
  frameworkRootPath = toAbsPath frameworkRoot;
  nixpkgsSourcePath = toAbsPath nixpkgsPath;
  pkgs = import nixpkgsSourcePath { inherit system; };
  lib = pkgs.lib;

  resolveScopedPath =
    spec:
    let
      scope = spec.scope or (throw "nixfied launcher requires a module spec scope");
      relPath = spec.path or "";
      suffix = if relPath == "" then "" else "/${relPath}";
    in
    if scope == "project" then
      (projectRootPath + suffix)
    else if scope == "framework" then
      (frameworkRootPath + suffix)
    else if scope == "absolute" then
      toAbsPath relPath
    else
      throw "nixfied launcher requires module spec scope to be one of project|framework|absolute";

  projectModules = map resolveScopedPath (builtins.fromJSON projectModuleSpecsJson);
  extraModules = map resolveScopedPath (builtins.fromJSON extraModuleSpecsJson);
  localOverrides = map resolveScopedPath (builtins.fromJSON localOverrideSpecsJson);

  excludedServices = builtins.sort builtins.lessThan (
    lib.unique (builtins.filter (name: name != "") (lib.splitString "," excludedServicesCsv))
  );
  selectedServicesRaw =
    if selectedServicesCsv == "__ALL__" then
      null
    else
      builtins.sort builtins.lessThan (
        lib.unique (builtins.filter (name: name != "") (lib.splitString "," selectedServicesCsv))
      );
  selectedServices =
    if selectedServicesRaw == null then
      null
    else
      builtins.filter (serviceName: !(builtins.elem serviceName excludedServices)) selectedServicesRaw;

  frameworkLib = import (frameworkRootPath + "/framework/core/default.nix") {
    inherit
      pkgs
      system
      ;
  };

  selectedProjectModules =
    if projectModules == [ ] then
      [ (projectRootPath + "/nixfied/project/module.nix") ]
    else
      projectModules;

  compiled = frameworkLib.mkNixfied {
    projectRoot = projectRootPath;
    projectModules = selectedProjectModules;
    inherit
      extraModules
      frameworkSourceRevision
      selectedServices
      ;
    localOverrides = localOverrides ++ [
      (
        { ... }:
        {
          nixfied.graph.excludedServices = excludedServices;
        }
      )
    ];
  };

  selectedApp =
    if builtins.hasAttr appName compiled.apps then
      compiled.apps.${appName}
    else
      throw "nixfied launcher: unknown app '${appName}'";

  launcherName = "nixfied-selected-app-${
    builtins.substring 0 10 (
      builtins.hashString "sha256" (
        appName + builtins.toJSON excludedServices + builtins.toJSON (selectedServices)
      )
    )
  }";

  selectedLauncher = pkgs.writeShellScriptBin launcherName ''
    set -euo pipefail
    exec ${selectedApp.program} "$@"
  '';
in
selectedLauncher
