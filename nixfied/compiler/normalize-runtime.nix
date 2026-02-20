{ lib }:
{ resolved }:
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

  primitives = {
    version = resolved.runtime.primitives.version;
    defs = resolved.runtime.primitives.defs;
  };

  ports = resolved.runtime.ports;

  directories = {
    base = resolved.runtime.directories.base;
  };

  runtimePackages = map builtins.toString (resolved.tooling.runtimePackages or [ ]);
}
