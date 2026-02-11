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

      mkExampleApp =
        {
          name,
          summary,
          details,
          usage ? [ "nix run .#${name}" ],
          script,
        }:
        lib.appApi.mkNixfiedApp {
          inherit name script;
          env = { };
          useDeps = false;
          api = {
            version = 1;
            summary = summary;
            details = details;
            usage = usage;
            category = "examples";
          };
        };

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

      mkDevApp =
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
            "${envVar}" = "dev";
          };
          useDeps = true;
          api = {
            version = 1;
            summary = summary;
            details = details;
            usage = usage;
            category = "core";
          };
        };
    in
    {
      jq_fmt_example = mkExampleApp {
        name = "jq_fmt_example";
        summary = "Format JSON with jq";
        details = "Reads JSON from stdin and prints formatted JSON (sorted keys) to stdout.";
        usage = [
          "printf '{\"b\":1,\"a\":2}' | nix run .#jq_fmt_example"
          "printf '{\"b\":1,\"a\":2}' | nix run github:willyrgf/mfm#jq_fmt_example"
        ];
        script = ''
          exec ${pkgs.jq}/bin/jq -S '.'
        '';
      };

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
              "minio_console:$MINIO_CONSOLE_PORT" \
              "reth_rpc:$RETH_RPC_PORT"
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

      reth-up = mkDevApp {
        name = "reth-up";
        summary = "Start local reth (dev)";
        details = "Starts a local reth dev node for the current slot and leaves it running in the background.";
        script = ''
          set -euo pipefail
          eval "$($SLOT_INFO)"

          if ! command -v reth >/dev/null 2>&1; then
            echo "ERROR: reth binary not found in PATH." >&2
            exit 1
          fi

          RETH_STATE_DIR="''${XDG_DATA_HOME:-$HOME/.local/share}/mfm/reth-$SLOT-$ENV"
          mkdir -p "$RETH_STATE_DIR" "$LOG_DIR"
          pid=$(start_service reth \
            --log "$LOG_DIR/reth.log" \
            --wait-port "$RETH_RPC_PORT" \
            --timeout 60 \
            -- \
            reth node \
              --dev \
              --datadir "$RETH_STATE_DIR" \
              --http \
              --http.addr "127.0.0.1" \
              --http.port "$RETH_RPC_PORT")
          echo "reth started: pid=$pid rpc=http://127.0.0.1:$RETH_RPC_PORT"
        '';
      };

      reth-down = mkDevApp {
        name = "reth-down";
        summary = "Stop local reth (dev)";
        details = "Stops local reth listeners bound to the current slot RPC port.";
        script = ''
          set -euo pipefail
          eval "$($SLOT_INFO)"

          pids=$(lsof -tiTCP:"$RETH_RPC_PORT" -sTCP:LISTEN -n -P 2>/dev/null || true)
          if [ -z "$pids" ]; then
            echo "reth is not running on port $RETH_RPC_PORT"
            exit 0
          fi

          for pid in $pids; do
            echo "stopping reth pid=$pid"
            kill -TERM "$pid" 2>/dev/null || true
          done
        '';
      };

      reth-status = mkDevApp {
        name = "reth-status";
        summary = "Show local reth status (dev)";
        details = "Reports whether reth is listening on the current slot RPC port.";
        script = ''
          set -euo pipefail
          eval "$($SLOT_INFO)"

          pids=$(lsof -tiTCP:"$RETH_RPC_PORT" -sTCP:LISTEN -n -P 2>/dev/null || true)
          if [ -n "$pids" ]; then
            echo "UP: reth_rpc port=$RETH_RPC_PORT pid=$pids"
          else
            echo "DOWN: reth_rpc port=$RETH_RPC_PORT"
          fi
        '';
      };
    };

  # Extra flake packages (merged into `packages` output).
  packages = { };

  # Optional: extend dev shells (merged into `devShells` output).
  devShells = { };
}
