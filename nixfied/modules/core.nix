{ lib, ... }:
let
  t = lib.types;
in
{
  imports = [
    ./runtime.nix
    ./tasks.nix
    ./workflows.nix
    ./operations.nix
    ./services/postgres.nix
    ./services/nginx.nix
    ./services/minio.nix
    ./services/reth.nix
    ./services/helios.nix
  ];

  options.nixfied = {
    identity = {
      projectId = lib.mkOption {
        type = t.str;
        default = "nixfied-project";
        description = "Stable project identity used in model and state paths.";
      };

      projectName = lib.mkOption {
        type = t.str;
        default = "Nixfied Project";
      };

      description = lib.mkOption {
        type = t.str;
        default = "Model-driven Nixfied project";
      };
    };

    state = {
      registryRoot = lib.mkOption {
        type = t.str;
        default = "/tmp/nixfied-runtime/nixfied-project";
      };

      artifactsRoot = lib.mkOption {
        type = t.str;
        default = "/tmp/ci-artifacts";
      };
    };

    tooling = {
      runtimePackages = lib.mkOption {
        type = t.listOf t.package;
        default = [ ];
      };

      devShellPackages = lib.mkOption {
        type = t.listOf t.package;
        default = [ ];
      };

      devShellHook = lib.mkOption {
        type = t.lines;
        default = ''
          echo "INFO: nixfied dev shell ready"
        '';
      };
    };
  };
}
