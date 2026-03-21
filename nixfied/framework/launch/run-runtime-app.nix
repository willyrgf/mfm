{
  system,
  projectRoot,
  frameworkRoot,
  nixpkgsPath,
  frameworkSourceRevision ? "unknown",
  appName,
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
      throw "nixfied runtime launcher requires an absolute path string";

  projectRootPath = toAbsPath projectRoot;
  frameworkRootPath = toAbsPath frameworkRoot;
  nixpkgsSourcePath = toAbsPath nixpkgsPath;
  pkgs = import nixpkgsSourcePath { inherit system; };

  resolveScopedPath =
    spec:
    let
      scope = spec.scope or (throw "nixfied runtime launcher requires a module spec scope");
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
      throw "nixfied runtime launcher requires module spec scope to be one of project|framework|absolute";

  projectModules = map resolveScopedPath (builtins.fromJSON projectModuleSpecsJson);
  extraModules = map resolveScopedPath (builtins.fromJSON extraModuleSpecsJson);
  localOverrides = map resolveScopedPath (builtins.fromJSON localOverrideSpecsJson);

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
      localOverrides
      ;
  };

  selectedApp =
    if builtins.hasAttr appName compiled.apps then
      compiled.apps.${appName}
    else
      throw "nixfied runtime launcher: unknown app '${appName}'";

  launcherName = "nixfied-runtime-app-${
    builtins.substring 0 10 (builtins.hashString "sha256" appName)
  }";

  selectedLauncher = pkgs.writeShellScriptBin launcherName ''
    set -euo pipefail
    exec ${selectedApp.program} "$@"
  '';
in
selectedLauncher
