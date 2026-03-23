{ lib }:
{
  resolved,
  statePolicy,
}:
let
  envNames =
    if resolved.runtime.env.names == [ ] then
      builtins.sort builtins.lessThan (builtins.attrNames resolved.runtime.env.offsets)
    else
      resolved.runtime.env.names;

  envOffsets = lib.genAttrs envNames (name: resolved.runtime.env.offsets.${name} or 0);
in
{
  slot = {
    var = resolved.runtime.slot.var;
    default = resolved.runtime.slot.default;
    max = resolved.runtime.slot.max;
    stride = resolved.runtime.slot.stride;
  };

  env = {
    var = resolved.runtime.env.var;
    names = envNames;
    offsets = envOffsets;
    default = resolved.runtime.env.default;
  };

  logging = {
    levelDefault = resolved.runtime.logging.levelDefault;
    outputDefault = resolved.runtime.logging.outputDefault;
  };

  orchestrator = {
    stopTimeoutSec = resolved.runtime.orchestrator.stopTimeoutSec;
  };

  primitives = {
    version = resolved.runtime.primitives.version;
    defs = resolved.runtime.primitives.defs;
  };

  ports = resolved.runtime.ports;

  directories = {
    base = statePolicy.runtimeBase;
  };

  ephemeral = {
    copyMode = resolved.runtime.ephemeral.copyMode;
    includeUntracked = resolved.runtime.ephemeral.includeUntracked;
    excludePatterns = resolved.runtime.ephemeral.excludePatterns;
    extraDirs = resolved.runtime.ephemeral.extraDirs;
    keepFailures = resolved.runtime.ephemeral.keepFailures;
    maxFailedRoots = resolved.runtime.ephemeral.maxFailedRoots;
    maxFailedRootAgeHours = resolved.runtime.ephemeral.maxFailedRootAgeHours;
    maxCopyBytes = resolved.runtime.ephemeral.maxCopyBytes;
    minFreeBytesAfterCopy = resolved.runtime.ephemeral.minFreeBytesAfterCopy;
    envFileMode = resolved.runtime.ephemeral.envFileMode;
    envFilePath = resolved.runtime.ephemeral.envFilePath;
  };

  runtimePackages = map builtins.toString (resolved.tooling.runtimePackages or [ ]);
}
