let
  root = ../..;
  flake = builtins.getFlake (toString root);
  system = builtins.currentSystem;
  pkgs = import flake.inputs.nixpkgs { inherit system; };
  nixfiedLib = import ../../nixfied/lib/default.nix { inherit pkgs system; };
  compiled = nixfiedLib.mkNixfied {
    projectRoot = root;
    projectModules = [ ../../nixfied/project/default.nix ];
    extraModules = [ ];
  };
  stripContext = value: builtins.unsafeDiscardStringContext (toString value);

  hasPrefix =
    prefix: value:
    builtins.isString value
    && (builtins.stringLength value) >= (builtins.stringLength prefix)
    && (builtins.substring 0 (builtins.stringLength prefix) value) == prefix;

  ciWorkflowPrefix = "workflow.ci.";
  ciWorkflowPrefixLen = builtins.stringLength ciWorkflowPrefix;
  workflowIds = builtins.sort builtins.lessThan (
    map stripContext (builtins.attrNames compiled.workflows)
  );
  ciModes = builtins.sort builtins.lessThan (
    pkgs.lib.unique (
      map (
        workflowId:
        builtins.substring ciWorkflowPrefixLen (
          (builtins.stringLength workflowId) - ciWorkflowPrefixLen
        ) workflowId
      ) (builtins.filter (workflowId: hasPrefix ciWorkflowPrefix workflowId) workflowIds)
    )
  );
in
{
  system = system;
  taskIds = builtins.sort builtins.lessThan (
    map stripContext (builtins.attrNames compiled.tasks)
  );
  workflowPlanTaskIds = builtins.mapAttrs (
    _: workflow:
    map (unit: stripContext unit.taskId) (workflow.plan or [ ])
  ) compiled.workflows;
  inherit
    workflowIds
    ciModes
    ;
}
