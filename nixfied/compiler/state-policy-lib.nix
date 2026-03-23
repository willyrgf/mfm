{ lib }:
let
  compilePolicy =
    {
      projectRoot,
      identity,
      policy,
    }:
    let
      workspaceId =
        if policy.workspace.mode == "literal" then
          if (policy.workspace.value or null) == null || policy.workspace.value == "" then
            throw "ERROR: nixfied.state.policy.workspace.value must be set when workspace.mode = literal"
          else
            policy.workspace.value
        else
          builtins.substring 0 policy.workspace.hashLength (
            builtins.hashString "sha256" (toString projectRoot)
          );

      replaceTokens =
        template:
        builtins.replaceStrings
          [ "{projectId}" "{workspaceId}" ]
          [
            identity.projectId
            workspaceId
          ]
          template;
    in
    {
      id = policy.id;
      kind = policy.kind;
      source = policy.source;
      ownerScope = policy.ownerScope;
      discoveryScope = policy.discoveryScope;
      workspace = {
        mode = policy.workspace.mode;
        hashLength = policy.workspace.hashLength;
        value = policy.workspace.value or null;
      };
      roots = {
        runtimeBaseTemplate = policy.roots.runtimeBase;
        registryRootTemplate = policy.roots.registryRoot;
        artifactsRootTemplate = policy.roots.artifactsRoot;
      };
      workspaceId = workspaceId;
      runtimeBase = replaceTokens policy.roots.runtimeBase;
      registryRoot = replaceTokens policy.roots.registryRoot;
      artifactsRoot = replaceTokens policy.roots.artifactsRoot;
    };
in
{
  inherit compilePolicy;
}
