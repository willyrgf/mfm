{ config, lib, ... }:
{
  config.nixfied.state.policy = {
    id = lib.mkDefault "project-shared";
    kind = lib.mkDefault "project-shared";
    source = lib.mkDefault "nixfied/framework/presets/state-policies/project-shared.nix";
    ownerScope = lib.mkDefault "project";
    discoveryScope = lib.mkDefault "project";
    workspace.mode = lib.mkDefault "project-root-hash";
    workspace.hashLength = lib.mkDefault 12;
    roots.runtimeBase = lib.mkDefault "/tmp/nixfied-runtime/${config.nixfied.identity.projectId}/runtime";
    roots.registryRoot = lib.mkDefault "/tmp/nixfied-runtime/${config.nixfied.identity.projectId}/registry";
    roots.artifactsRoot = lib.mkDefault "/tmp/nixfied-artifacts-${config.nixfied.identity.projectId}";
  };
}
