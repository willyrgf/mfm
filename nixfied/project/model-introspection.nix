let
  root = ../..;
  flake = builtins.getFlake (toString root);
  system = builtins.currentSystem;
  pkgs = import flake.inputs.nixpkgs { inherit system; };
  nixfiedLib = import ../../nixfied/framework/core/default.nix { inherit pkgs system; };
  compiled = nixfiedLib.mkNixfied {
    projectRoot = root;
    projectModules = [ ../../nixfied/project/default.nix ];
    extraModules = [ ];
  };
  graph = compiled.introspectionGraph;
  stripContext = value: builtins.unsafeDiscardStringContext (toString value);

  hasPrefix =
    prefix: value:
    builtins.isString value
    && (builtins.stringLength value) >= (builtins.stringLength prefix)
    && (builtins.substring 0 (builtins.stringLength prefix) value) == prefix;

  ciWorkflowPrefix = "workflow.ci.";
  ciWorkflowPrefixLen = builtins.stringLength ciWorkflowPrefix;
  appIds = map stripContext (graph.resolution.appIds or [ ]);
  taskIds = map stripContext (graph.resolution.taskIds or [ ]);
  workflowIds = map stripContext (graph.resolution.workflowIds or [ ]);
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
  workflowPlanTaskIds = builtins.listToAttrs (
    map (
      workflowId:
      let
        workflowNode = graph.nodes."workflow:${workflowId}" or null;
      in
      {
        name = workflowId;
        value =
          if workflowNode == null then [ ] else map stripContext (workflowNode.closure.unitTaskIds or [ ]);
      }
    ) workflowIds
  );
in
{
  system = system;
  inherit
    appIds
    taskIds
    workflowPlanTaskIds
    ;
  inherit
    workflowIds
    ciModes
    ;
}
