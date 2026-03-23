{ config, lib, ... }:
{
  config.nixfied.state.policy = {
    id = lib.mkDefault "workspace-scoped";
    kind = lib.mkDefault "workspace-scoped";
    source = lib.mkDefault "nixfied/framework/presets/state-policies/workspace-scoped.nix";
    ownerScope = lib.mkDefault "workspace";
    discoveryScope = lib.mkDefault "workspace";
    workspace.mode = lib.mkDefault "project-root-hash";
    workspace.hashLength = lib.mkDefault 12;
    roots.runtimeBase = lib.mkDefault "/tmp/nixfied-runtime/${config.nixfied.identity.projectId}/{workspaceId}/runtime";
    roots.registryRoot = lib.mkDefault "/tmp/nixfied-runtime/${config.nixfied.identity.projectId}/{workspaceId}/registry";
    roots.artifactsRoot = lib.mkDefault "/tmp/nixfied-artifacts-${config.nixfied.identity.projectId}-{workspaceId}";
  };
}
