{ lib }:
let
  t = lib.types;
  scopeType = t.enum [
    "workspace"
    "project"
    "slot"
    "custom"
  ];
in
{
  id = lib.mkOption {
    type = t.str;
    default = "workspace-scoped";
    description = "Stable identifier for the effective state policy.";
  };

  kind = lib.mkOption {
    type = t.enum [
      "workspace-scoped"
      "project-shared"
      "slot-shared"
      "custom"
    ];
    default = "workspace-scoped";
    description = "High-level policy family used for state roots.";
  };

  source = lib.mkOption {
    type = t.str;
    default = "nixfied/framework/presets/state-policies/workspace-scoped.nix";
    description = "Source module or authoring surface for the effective policy.";
  };

  ownerScope = lib.mkOption {
    type = scopeType;
    default = "workspace";
  };

  discoveryScope = lib.mkOption {
    type = scopeType;
    default = "workspace";
  };

  workspace = {
    mode = lib.mkOption {
      type = t.enum [
        "project-root-hash"
        "literal"
      ];
      default = "project-root-hash";
      description = "How the effective workspace id is derived.";
    };

    value = lib.mkOption {
      type = t.nullOr t.str;
      default = null;
      description = "Literal workspace id when workspace.mode = literal.";
    };

    hashLength = lib.mkOption {
      type = t.ints.unsigned;
      default = 12;
      description = "Prefix length when deriving the workspace id from the project root hash.";
    };
  };

  roots = {
    runtimeBase = lib.mkOption {
      type = t.str;
      default = "/tmp/nixfied-runtime/{projectId}/{workspaceId}/runtime";
      description = "Template for the runtime base root. Supports {projectId} and {workspaceId}.";
    };

    registryRoot = lib.mkOption {
      type = t.str;
      default = "/tmp/nixfied-runtime/{projectId}/{workspaceId}/registry";
      description = "Template for the registry root. Supports {projectId} and {workspaceId}.";
    };

    artifactsRoot = lib.mkOption {
      type = t.str;
      default = "/tmp/nixfied-artifacts-{projectId}-{workspaceId}";
      description = "Template for the artifacts root. Supports {projectId} and {workspaceId}.";
    };
  };
}
