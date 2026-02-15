{
  pkgs,
  project,
  lib,
  slots ? null,
  hooks ? null,
  postgres ? null,
  nginx ? null,
  minio ? null,
  reth ? null,
  helios ? null,
  supervisor ? null,
  ephemeral ? null,
}:

{
  # User-owned extension point.
  #
  # This directory is intended for customization that should survive framework
  # upgrades. Prefer nixfied/project/ for normal command/config wiring, and use
  # nixfied/local/ for extra apps/packages that shouldn't live in the framework.
  #
  # Notes:
  # - Apps must satisfy the Nixfied app API contract (meta.nixfied.api).
  # - Use `lib.appApi.mkNixfiedApp { ... }` to build compliant apps.
  apps =
    let
      # v2 shell-app contract inventory (project-local):
      # - up/down/svc-status: typed, outputs=text, wraps supervisor hooks, failure map owner=local/default.nix
      # - svc-logs/svc-restart: passthrough, outputs=text, wraps supervisor hooks, failure map owner=local/default.nix
      # - evm-contract-artifact-*: typed, outputs=json, wraps Foundry helper tools, failure map owner=local/default.nix
      failureCodesScript = {
        generic = 1;
        usage = 2;
        precondition = 3;
        unavailable = 4;
        timeout = 5;
      };
      failureCodesSupervisor = failureCodesScript;
      failureCodesFoundry = failureCodesScript;
      envVar = project.project.envVar or "PROJECT_ENV";

      mkProdSupervisorApp =
        {
          name,
          summary,
          details,
          usage ? [ "nix run .#${name}" ],
          allowUnknownArgs ? false,
          failureCodes ? failureCodesSupervisor,
          script,
        }:
        lib.appApi.mkNixfiedApp {
          inherit name script;
          env = {
            # Enforce: supervisor is production-only.
            "${envVar}" = "prod";
            # process-compose defaults to TUI; disable so daemon mode works in non-TTY contexts.
            PC_DISABLE_TUI = "1";
          };
          useDeps = false;
          api = lib.appApi.mkApi {
            inherit
              name
              summary
              details
              usage
              allowUnknownArgs
              failureCodes
              ;
            category = "supervisor";
            idempotent = false;
          };
        };
    in
    {
      up = mkProdSupervisorApp {
        name = "up";
        summary = "Start all services (prod)";
        details = "Starts all supervisor-managed services for the current slot, forcing the production environment (MFM_ENV=prod).";
        script = ''run_hook SUPERVISOR_START_DAEMON'';
      };

      down = mkProdSupervisorApp {
        name = "down";
        summary = "Stop all services (prod)";
        details = "Stops all supervisor-managed services for the current slot, forcing the production environment (MFM_ENV=prod).";
        script = ''run_hook SUPERVISOR_STOP'';
      };

      svc-status = mkProdSupervisorApp {
        name = "svc-status";
        summary = "Show service status (prod)";
        details = "Shows the status of supervisor-managed services, forcing the production environment (MFM_ENV=prod).";
        script = ''
          set -euo pipefail
          eval "$($SLOT_INFO)"

          echo "ENV=$ENV SLOT=$SLOT"
          run_hook SUPERVISOR_IS_RUNNING || true

          if command -v lsof >/dev/null 2>&1; then
            for spec in \
              "rest_api:$REST_API_PORT" \
              "postgres:$POSTGRES_PORT" \
              "minio:$MINIO_PORT" \
              "minio_console:$MINIO_CONSOLE_PORT"
            do
              name="''${spec%%:*}"
              port="''${spec##*:}"
              pids=$(lsof -tiTCP:"$port" -sTCP:LISTEN -n -P 2>/dev/null || true)
              if [ -n "$pids" ]; then
                echo "LISTEN: $name port=$port pid=$pids"
              else
                echo "DOWN: $name port=$port"
              fi
            done
          else
            echo "WARN: lsof not available; skipping port checks" >&2
          fi
        '';
      };

      svc-logs = mkProdSupervisorApp {
        name = "svc-logs";
        summary = "Show service logs (prod)";
        details = "Streams logs for supervisor-managed services, forcing the production environment (MFM_ENV=prod). Passthrough wrapper: arguments are forwarded unchanged to the supervisor hook.";
        usage = [ "nix run .#svc-logs -- <args>" ];
        allowUnknownArgs = true;
        script = ''run_hook SUPERVISOR_LOGS "$@"'';
      };

      svc-restart = mkProdSupervisorApp {
        name = "svc-restart";
        summary = "Restart a service (prod)";
        details = "Restarts supervisor-managed services, forcing the production environment (MFM_ENV=prod). Passthrough wrapper: arguments are forwarded unchanged to the supervisor hook.";
        usage = [ "nix run .#svc-restart -- <args>" ];
        allowUnknownArgs = true;
        script = ''
          set -euo pipefail
          run_hook SUPERVISOR_RESTART "$@"
        '';
      };

      evm-contract-artifact-configurable-counter = lib.appApi.mkNixfiedApp {
        name = "evm-contract-artifact-configurable-counter";
        env = { };
        useDeps = true;
        api = lib.appApi.mkApi {
          name = "evm-contract-artifact-configurable-counter";
          summary = "Build ConfigurableCounter artifact JSON";
          details = "Compiles contracts/src/ConfigurableCounter.sol with Foundry and prints compact JSON {artifact:{abi,bytecode.object}} to stdout.";
          usage = [ "nix run .#evm-contract-artifact-configurable-counter" ];
          category = "evm";
          allowUnknownArgs = false;
          outputsMode = "json";
          failureCodes = failureCodesFoundry;
        };
        script = ''
          exec mfm-contract-artifact-configurable-counter "$@"
        '';
      };

      evm-contract-artifact-mock-erc20 = lib.appApi.mkNixfiedApp {
        name = "evm-contract-artifact-mock-erc20";
        env = { };
        useDeps = true;
        api = lib.appApi.mkApi {
          name = "evm-contract-artifact-mock-erc20";
          summary = "Build MockERC20 artifact JSON";
          details = "Compiles contracts/src/MockERC20.sol with Foundry and prints compact JSON {artifact:{abi,bytecode.object}} to stdout.";
          usage = [ "nix run .#evm-contract-artifact-mock-erc20" ];
          category = "evm";
          allowUnknownArgs = false;
          outputsMode = "json";
          failureCodes = failureCodesFoundry;
        };
        script = ''
          exec mfm-contract-artifact-mock-erc20 "$@"
        '';
      };
    };

  # Extra flake packages (merged into `packages` output).
  packages = { };

  # Optional: extend dev shells (merged into `devShells` output).
  devShells = { };
}
