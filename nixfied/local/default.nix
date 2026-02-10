{
  pkgs,
  project,
  lib,
  slots ? null,
  hooks ? null,
  postgres ? null,
  nginx ? null,
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
      envVar = project.project.envVar or "PROJECT_ENV";

      mkProdSupervisorApp =
        {
          name,
          summary,
          details,
          usage ? [ "nix run .#${name}" ],
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
          api = {
            version = 1;
            summary = summary;
            details = details;
            usage = usage;
            category = "supervisor";
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
        details = "Streams logs for supervisor-managed services, forcing the production environment (MFM_ENV=prod). Arguments are forwarded to the hook.";
        usage = [ "nix run .#svc-logs -- <args>" ];
        script = ''run_hook SUPERVISOR_LOGS "$@"'';
      };

      svc-restart = mkProdSupervisorApp {
        name = "svc-restart";
        summary = "Restart a service (prod)";
        details = "Restarts a supervisor-managed service, forcing the production environment (MFM_ENV=prod). Arguments are forwarded to the hook.";
        usage = [ "nix run .#svc-restart -- <args>" ];
        script = ''
          set -euo pipefail
          if [ -n "''${1:-}" ]; then
            echo "INFO: Restarting supervisor-managed services (service arg ignored: ''${1})"
          else
            echo "INFO: Restarting supervisor-managed services"
          fi

          run_hook SUPERVISOR_STOP
          run_hook SUPERVISOR_START_DAEMON
        '';
      };
    };

  # Extra flake packages (merged into `packages` output).
  packages = { };

  # Optional: extend dev shells (merged into `devShells` output).
  devShells = { };
}
