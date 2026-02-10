# Framework-level app generators
# Auto-generates user-facing apps from enabled modules.
{
  pkgs,
  project,
  lib,
  postgres ? null,
  nginx ? null,
  supervisor ? null,
  slots,
}:

let
  pgDatabase = project.modules.postgres.database or "app";
  pgPackage = project.modules.postgres.package or pkgs.postgresql_16;

  mk =
    {
      name,
      summary,
      details,
      usage ? [ "nix run .#${name}" ],
      script,
      category ? "module",
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
        category = category;
      };
    };

  postgresApps =
    if postgres == null then
      { }
    else
      {
        db-start = mk {
          name = "db-start";
          summary = "Start PostgreSQL server";
          details = "Starts the PostgreSQL server for the current slot/env (configured in nixfied/project/conf.nix).";
          category = "postgres";
          script = ''run_hook POSTGRES_START'';
        };
        db-stop = mk {
          name = "db-stop";
          summary = "Stop PostgreSQL server";
          details = "Stops the PostgreSQL server for the current slot/env.";
          category = "postgres";
          script = ''run_hook POSTGRES_STOP'';
        };
        db-setup = mk {
          name = "db-setup";
          summary = "Create and set up database";
          details = "Creates the configured database/user for the current slot/env.";
          category = "postgres";
          script = ''run_hook POSTGRES_SETUP_DB'';
        };
        db-full-start = mk {
          name = "db-full-start";
          summary = "Init, start, and set up PostgreSQL";
          details = "Initializes the data directory (if needed), starts PostgreSQL, and sets up the database for the current slot/env.";
          category = "postgres";
          script = ''run_hook POSTGRES_FULL_START'';
        };
        db-list = mk {
          name = "db-list";
          summary = "List PostgreSQL instances";
          details = "Lists PostgreSQL instances managed by Nixfied for this project.";
          category = "postgres";
          script = ''run_hook POSTGRES_LIST_INSTANCES'';
        };
        db-backup = mk {
          name = "db-backup";
          summary = "Create database backup";
          details = "Creates a backup of the current slot/env database. Arguments are forwarded to the backup hook.";
          usage = [ "nix run .#db-backup -- <args>" ];
          category = "postgres";
          script = ''run_hook POSTGRES_BACKUP "$@"'';
        };
        db-restore = mk {
          name = "db-restore";
          summary = "Restore database from backup";
          details = "Restores the current slot/env database from a backup. Arguments are forwarded to the restore hook.";
          usage = [ "nix run .#db-restore -- <args>" ];
          category = "postgres";
          script = ''run_hook POSTGRES_RESTORE "$@"'';
        };
        db-backup-list = mk {
          name = "db-backup-list";
          summary = "List database backups";
          details = "Lists available backups for the current project.";
          category = "postgres";
          script = ''run_hook POSTGRES_LIST_BACKUPS'';
        };
        db-test-migration = mk {
          name = "db-test-migration";
          summary = "Test database migrations";
          details = "Runs the configured migration test routine for the current slot/env.";
          category = "postgres";
          script = ''run_hook POSTGRES_TEST_MIGRATIONS'';
        };
        db-check-ports = mk {
          name = "db-check-ports";
          summary = "Check PostgreSQL port status";
          details = "Checks whether the PostgreSQL port for the current slot/env is available.";
          category = "postgres";
          script = ''run_hook POSTGRES_CHECK_PORT'';
        };
        db-shell = mk {
          name = "db-shell";
          summary = "Open PostgreSQL shell";
          details = "Opens a psql shell connected to the current slot/env database.";
          category = "postgres";
          script = ''
            eval "$($SLOT_INFO)"
            exec ${pgPackage}/bin/psql "postgresql://localhost:$POSTGRES_PORT/${pgDatabase}" "$@"
          '';
        };
      };

  nginxApps =
    if nginx == null then
      { }
    else
      {
        nginx-start = mk {
          name = "nginx-start";
          summary = "Start nginx server";
          details = "Starts the nginx instance managed by Nixfied for the current slot/env.";
          category = "nginx";
          script = ''run_hook NGINX_START'';
        };
        nginx-stop = mk {
          name = "nginx-stop";
          summary = "Stop nginx server";
          details = "Stops the nginx instance managed by Nixfied for the current slot/env.";
          category = "nginx";
          script = ''run_hook NGINX_STOP'';
        };
        nginx-reload = mk {
          name = "nginx-reload";
          summary = "Reload nginx configuration";
          details = "Reloads nginx configuration for the current slot/env.";
          category = "nginx";
          script = ''run_hook NGINX_RELOAD'';
        };
        nginx-site-add = mk {
          name = "nginx-site-add";
          summary = "Add an nginx site";
          details = "Adds a new nginx site configuration. Arguments are forwarded to the hook.";
          usage = [ "nix run .#nginx-site-add -- <args>" ];
          category = "nginx";
          script = ''run_hook NGINX_SITE_ADD "$@"'';
        };
        nginx-site-remove = mk {
          name = "nginx-site-remove";
          summary = "Remove an nginx site";
          details = "Removes an nginx site configuration. Arguments are forwarded to the hook.";
          usage = [ "nix run .#nginx-site-remove -- <args>" ];
          category = "nginx";
          script = ''run_hook NGINX_SITE_REMOVE "$@"'';
        };
        nginx-site-list = mk {
          name = "nginx-site-list";
          summary = "List nginx sites";
          details = "Lists nginx sites configured for the project.";
          category = "nginx";
          script = ''run_hook NGINX_SITE_LIST'';
        };
        nginx-site-enable = mk {
          name = "nginx-site-enable";
          summary = "Enable an nginx site";
          details = "Enables an nginx site. Arguments are forwarded to the hook.";
          usage = [ "nix run .#nginx-site-enable -- <args>" ];
          category = "nginx";
          script = ''run_hook NGINX_SITE_ENABLE "$@"'';
        };
        nginx-site-disable = mk {
          name = "nginx-site-disable";
          summary = "Disable an nginx site";
          details = "Disables an nginx site. Arguments are forwarded to the hook.";
          usage = [ "nix run .#nginx-site-disable -- <args>" ];
          category = "nginx";
          script = ''run_hook NGINX_SITE_DISABLE "$@"'';
        };
        nginx-cert-obtain = mk {
          name = "nginx-cert-obtain";
          summary = "Obtain SSL certificate";
          details = "Obtains an SSL certificate. Arguments are forwarded to the hook.";
          usage = [ "nix run .#nginx-cert-obtain -- <args>" ];
          category = "nginx";
          script = ''run_hook NGINX_CERT_OBTAIN "$@"'';
        };
        nginx-cert-renew = mk {
          name = "nginx-cert-renew";
          summary = "Renew SSL certificates";
          details = "Renews SSL certificates for configured sites.";
          category = "nginx";
          script = ''run_hook NGINX_CERT_RENEW'';
        };
        nginx-cert-status = mk {
          name = "nginx-cert-status";
          summary = "Show SSL certificate status";
          details = "Shows SSL certificate status for configured sites.";
          category = "nginx";
          script = ''run_hook NGINX_CERT_STATUS'';
        };
      };

  supervisorApps =
    if supervisor == null then
      { }
    else
      {
        up = mk {
          name = "up";
          summary = "Start all services";
          details = "Starts all supervisor-managed services for the current slot/env.";
          category = "supervisor";
          script = ''run_hook SUPERVISOR_START_DAEMON'';
        };
        down = mk {
          name = "down";
          summary = "Stop all services";
          details = "Stops all supervisor-managed services for the current slot/env.";
          category = "supervisor";
          script = ''run_hook SUPERVISOR_STOP'';
        };
        svc-status = mk {
          name = "svc-status";
          summary = "Show service status";
          details = "Shows the status of supervisor-managed services.";
          category = "supervisor";
          script = ''run_hook SUPERVISOR_STATUS'';
        };
        svc-logs = mk {
          name = "svc-logs";
          summary = "Show service logs";
          details = "Streams logs for supervisor-managed services. Arguments are forwarded to the hook.";
          usage = [ "nix run .#svc-logs -- <args>" ];
          category = "supervisor";
          script = ''run_hook SUPERVISOR_LOGS "$@"'';
        };
        svc-restart = mk {
          name = "svc-restart";
          summary = "Restart a service";
          details = "Restarts a supervisor-managed service. Arguments are forwarded to the hook.";
          usage = [ "nix run .#svc-restart -- <args>" ];
          category = "supervisor";
          script = ''run_hook SUPERVISOR_RESTART "$@"'';
        };
      };

  portNames = builtins.attrNames (project.ports or { });

  utilityApps = {
    check-ports = mk {
      name = "check-ports";
      summary = "Scan configured ports for conflicts";
      details = "Scans the configured ports for the current slot/env and reports whether they are free or listening.";
      category = "utility";
      script = ''
        eval "$($SLOT_INFO)"
        LSOF="${pkgs.lsof}/bin/lsof"
        echo "Port status for slot ''${SLOT:-0}, env ''${ENV:-dev}:"
        echo ""
        ${pkgs.lib.concatMapStringsSep "\n" (
          portName:
          let
            varName =
              pkgs.lib.strings.toUpper (pkgs.lib.replaceStrings [ "-" "." ] [ "_" "_" ] portName) + "_PORT";
          in
          ''
            PORT_VAL="''${${varName}:-}"
            if [ -n "$PORT_VAL" ]; then
              if "$LSOF" -iTCP:"$PORT_VAL" -sTCP:LISTEN -n -P >/dev/null 2>&1; then
                PIDS=$("$LSOF" -iTCP:"$PORT_VAL" -sTCP:LISTEN -n -P -t 2>/dev/null | tr '\n' ',' | sed 's/,$//')
                echo "  ${portName} ($PORT_VAL): IN USE (PIDs: $PIDS)"
              else
                echo "  ${portName} ($PORT_VAL): free"
              fi
            fi
          ''
        ) portNames}
      '';
    };
    ports = mk {
      name = "ports";
      summary = "Show port assignments";
      details = "Prints effective port assignments for the current slot/env.";
      category = "utility";
      script = ''
        eval "$($SLOT_INFO)"
        echo "Port assignments for slot ''${SLOT:-0}, env ''${ENV:-dev}:"
        echo ""
        ${pkgs.lib.concatMapStringsSep "\n" (
          portName:
          let
            varName =
              pkgs.lib.strings.toUpper (pkgs.lib.replaceStrings [ "-" "." ] [ "_" "_" ] portName) + "_PORT";
          in
          ''
            echo "  ${portName}: ''${${varName}:-n/a}"
          ''
        ) portNames}
      '';
    };
  };

in
postgresApps // nginxApps // supervisorApps // utilityApps
