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

  postgresApps =
    if postgres == null then
      { }
    else
      {
        db-start = lib.mkApp {
          name = "db-start";
          description = "Start PostgreSQL server";
          script = ''run_hook POSTGRES_START'';
        };
        db-stop = lib.mkApp {
          name = "db-stop";
          description = "Stop PostgreSQL server";
          script = ''run_hook POSTGRES_STOP'';
        };
        db-setup = lib.mkApp {
          name = "db-setup";
          description = "Create and set up database";
          script = ''run_hook POSTGRES_SETUP_DB'';
        };
        db-full-start = lib.mkApp {
          name = "db-full-start";
          description = "Init, start, and set up PostgreSQL";
          script = ''run_hook POSTGRES_FULL_START'';
        };
        db-list = lib.mkApp {
          name = "db-list";
          description = "List PostgreSQL instances";
          script = ''run_hook POSTGRES_LIST_INSTANCES'';
        };
        db-backup = lib.mkApp {
          name = "db-backup";
          description = "Create database backup";
          script = ''run_hook POSTGRES_BACKUP "$@"'';
        };
        db-restore = lib.mkApp {
          name = "db-restore";
          description = "Restore database from backup";
          script = ''run_hook POSTGRES_RESTORE "$@"'';
        };
        db-backup-list = lib.mkApp {
          name = "db-backup-list";
          description = "List database backups";
          script = ''run_hook POSTGRES_LIST_BACKUPS'';
        };
        db-test-migration = lib.mkApp {
          name = "db-test-migration";
          description = "Test database migrations";
          script = ''run_hook POSTGRES_TEST_MIGRATIONS'';
        };
        db-check-ports = lib.mkApp {
          name = "db-check-ports";
          description = "Check PostgreSQL port status";
          script = ''run_hook POSTGRES_CHECK_PORT'';
        };
        db-shell = lib.mkApp {
          name = "db-shell";
          description = "Open PostgreSQL shell";
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
        nginx-start = lib.mkApp {
          name = "nginx-start";
          description = "Start nginx server";
          script = ''run_hook NGINX_START'';
        };
        nginx-stop = lib.mkApp {
          name = "nginx-stop";
          description = "Stop nginx server";
          script = ''run_hook NGINX_STOP'';
        };
        nginx-reload = lib.mkApp {
          name = "nginx-reload";
          description = "Reload nginx configuration";
          script = ''run_hook NGINX_RELOAD'';
        };
        nginx-site-add = lib.mkApp {
          name = "nginx-site-add";
          description = "Add an nginx site";
          script = ''run_hook NGINX_SITE_ADD "$@"'';
        };
        nginx-site-remove = lib.mkApp {
          name = "nginx-site-remove";
          description = "Remove an nginx site";
          script = ''run_hook NGINX_SITE_REMOVE "$@"'';
        };
        nginx-site-list = lib.mkApp {
          name = "nginx-site-list";
          description = "List nginx sites";
          script = ''run_hook NGINX_SITE_LIST'';
        };
        nginx-site-enable = lib.mkApp {
          name = "nginx-site-enable";
          description = "Enable an nginx site";
          script = ''run_hook NGINX_SITE_ENABLE "$@"'';
        };
        nginx-site-disable = lib.mkApp {
          name = "nginx-site-disable";
          description = "Disable an nginx site";
          script = ''run_hook NGINX_SITE_DISABLE "$@"'';
        };
        nginx-cert-obtain = lib.mkApp {
          name = "nginx-cert-obtain";
          description = "Obtain SSL certificate";
          script = ''run_hook NGINX_CERT_OBTAIN "$@"'';
        };
        nginx-cert-renew = lib.mkApp {
          name = "nginx-cert-renew";
          description = "Renew SSL certificates";
          script = ''run_hook NGINX_CERT_RENEW'';
        };
        nginx-cert-status = lib.mkApp {
          name = "nginx-cert-status";
          description = "Show SSL certificate status";
          script = ''run_hook NGINX_CERT_STATUS'';
        };
      };

  supervisorApps =
    if supervisor == null then
      { }
    else
      {
        up = lib.mkApp {
          name = "up";
          description = "Start all services";
          script = ''run_hook SUPERVISOR_START_DAEMON'';
        };
        down = lib.mkApp {
          name = "down";
          description = "Stop all services";
          script = ''run_hook SUPERVISOR_STOP'';
        };
        svc-status = lib.mkApp {
          name = "svc-status";
          description = "Show service status";
          script = ''run_hook SUPERVISOR_STATUS'';
        };
        svc-logs = lib.mkApp {
          name = "svc-logs";
          description = "Show service logs";
          script = ''run_hook SUPERVISOR_LOGS "$@"'';
        };
        svc-restart = lib.mkApp {
          name = "svc-restart";
          description = "Restart a service";
          script = ''run_hook SUPERVISOR_RESTART "$@"'';
        };
      };

  portNames = builtins.attrNames (project.ports or { });

  utilityApps = {
    check-ports = lib.mkApp {
      name = "check-ports";
      description = "Scan configured ports for conflicts";
      script = ''
        eval "$($SLOT_INFO)"
        LSOF="${pkgs.lsof}/bin/lsof"
        echo "Port status for slot ''${${project.project.slotVar}:-0}, env ''${${project.project.envVar}:-dev}:"
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
    ports = lib.mkApp {
      name = "ports";
      description = "Show port assignments";
      script = ''
        eval "$($SLOT_INFO)"
        echo "Port assignments for slot ''${${project.project.slotVar}:-0}, env ''${${project.project.envVar}:-dev}:"
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
