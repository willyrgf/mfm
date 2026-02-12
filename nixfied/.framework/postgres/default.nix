# PostgreSQL module aggregator
{
  pkgs,
  project,
  slots,
}:

let
  cfg = project.modules.postgres or { };
  pgPackage = cfg.package or pkgs.postgresql_16;
  pgDatabase = cfg.database or "app";
  portKey = cfg.portKey or "postgres";
  portVar = slots.portVarName portKey;
  dataDirName = cfg.dataDirName or "postgres";
  pgdataExpr = slots.getServiceDir dataDirName;

  config = import ./config.nix { inherit pkgs project; };
  lifecycle = import ./lifecycle.nix {
    inherit
      pkgs
      project
      slots
      config
      ;
  };
  backupMod = import ./backup.nix { inherit pkgs project slots; };
  migration = import ./migration.nix { inherit pkgs project slots; };
  migrationSafety = import ./migration-safety.nix { inherit pkgs project slots; };
  rollback = import ./rollback.nix { inherit pkgs project slots; };
  portMgmt = import ./port-management.nix { inherit pkgs; };

  restart = pkgs.writeShellScript "postgres-restart" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    PORT_VAR="${portVar}"
    export PGPORT="''${!PORT_VAR}"
    export PGDATA="${pgdataExpr}"
    export PGSOCKET_DIR="''${POSTGRES_SOCKET_DIR:-$PGDATA/run/sockets}"

    ${lifecycle.stop}
    ${lifecycle.start}
  '';

  status = pkgs.writeShellScript "postgres-status" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    PORT_VAR="${portVar}"
    PGPORT="''${PGPORT:-''${!PORT_VAR}}"
    PGDATA="''${PGDATA:-${pgdataExpr}}"

    RUNNING=false
    if ${pgPackage}/bin/pg_isready -U postgres -h localhost -p "$PGPORT" -q 2>/dev/null; then
      RUNNING=true
    fi

    PID=""
    if [ -f "$PGDATA/postmaster.pid" ]; then
      PID=$(head -1 "$PGDATA/postmaster.pid" 2>/dev/null || true)
    fi

    echo "service=postgres slot=$SLOT env=$ENV port=$PGPORT pgdata=$PGDATA running=$RUNNING pid=''${PID:-unknown}"

    if [ "$RUNNING" = "true" ]; then
      exit 0
    fi
    exit 1
  '';

  health = pkgs.writeShellScript "postgres-health" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    PORT_VAR="${portVar}"
    PGPORT="''${PGPORT:-''${!PORT_VAR}}"

    if ${pgPackage}/bin/pg_isready -U postgres -h localhost -p "$PGPORT" -q 2>/dev/null; then
      echo "OK: PostgreSQL healthy port=$PGPORT"
      exit 0
    fi

    echo "ERROR: PostgreSQL unhealthy port=$PGPORT" >&2
    exit 1
  '';

  checkConfig = pkgs.writeShellScript "postgres-check-config" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    PGDATA="${pgdataExpr}"

    if [ ! -f "$PGDATA/postgresql.conf" ]; then
      echo "ERROR: missing postgresql.conf at $PGDATA" >&2
      exit 1
    fi

    if ${pgPackage}/bin/postgres -D "$PGDATA" -C port >/dev/null 2>&1; then
      echo "OK: PostgreSQL configuration valid pgdata=$PGDATA"
      exit 0
    fi

    echo "ERROR: PostgreSQL configuration invalid pgdata=$PGDATA" >&2
    exit 1
  '';

  shell = pkgs.writeShellScript "postgres-shell" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    PORT_VAR="${portVar}"
    PGPORT="''${!PORT_VAR}"
    PGDATABASE="''${PGDATABASE:-${pgDatabase}}"

    exec ${pgPackage}/bin/psql "postgresql://localhost:$PGPORT/$PGDATABASE" "$@"
  '';

  publicApi = {
    version = 1;
    service = "postgres";
    summary = "PostgreSQL service management API";
    details = "Public service contract for managing PostgreSQL across dev/prod/test/ci.";
    profiles = [
      "dev"
      "prod"
      "test"
      "ci"
    ];
    artifacts = {
      portKey = portKey;
      portVar = portVar;
      dataDir = pgdataExpr;
      defaultDatabase = pgDatabase;
    };
    coreOps = {
      init = {
        script = lifecycle.init;
        summary = "Initialize PostgreSQL data directory";
        details = "Initializes PGDATA and writes environment-specific PostgreSQL configuration.";
      };
      start = {
        script = lifecycle.start;
        summary = "Start PostgreSQL server";
        details = "Starts PostgreSQL for the current slot and environment.";
      };
      stop = {
        script = lifecycle.stop;
        summary = "Stop PostgreSQL server";
        details = "Stops PostgreSQL for the current slot and environment.";
      };
      restart = {
        script = restart;
        summary = "Restart PostgreSQL server";
        details = "Stops then starts PostgreSQL for the current slot and environment.";
      };
      status = {
        script = status;
        summary = "Show PostgreSQL status";
        details = "Prints PostgreSQL status for the current slot and environment.";
      };
      health = {
        script = health;
        summary = "Run PostgreSQL health check";
        details = "Checks PostgreSQL readiness on the configured port.";
      };
      check-config = {
        script = checkConfig;
        summary = "Validate PostgreSQL configuration";
        details = "Validates postgresql.conf for the current slot and environment.";
      };
    };
    extensions = {
      setup-db = {
        script = lifecycle.setupDb;
        hook = "SETUP_DB";
        summary = "Create and configure database";
        details = "Creates the configured database and required extensions.";
      };
      full-start = {
        script = lifecycle.fullStart;
        hook = "FULL_START";
        summary = "Init, start, and set up PostgreSQL";
        details = "Performs init/start/setup-db in one operation.";
      };
      full-start-test = {
        script = lifecycle.fullStartTest;
        hook = "FULL_START_TEST";
        summary = "Init/start/setup for test database";
        details = "Performs init/start/setup-db using the configured test database.";
      };
      list-instances = {
        script = lifecycle.listInstances;
        hook = "LIST_INSTANCES";
        summary = "List PostgreSQL instances";
        details = "Lists PostgreSQL instances managed by Nixfied.";
      };
      backup = {
        script = backupMod.backup;
        summary = "Create PostgreSQL backup";
        details = "Creates a backup for the current slot and environment.";
        usage = [ "nix run .#service::postgres::backup -- <args>" ];
      };
      restore = {
        script = backupMod.restore;
        summary = "Restore PostgreSQL backup";
        details = "Restores PostgreSQL data from a selected backup.";
        usage = [ "nix run .#service::postgres::restore -- <backup-path>" ];
      };
      list-backups = {
        script = backupMod.listBackups;
        hook = "LIST_BACKUPS";
        summary = "List PostgreSQL backups";
        details = "Lists backups for the current slot and environment.";
      };
      verify-backup = {
        script = backupMod.verifyBackup;
        summary = "Verify PostgreSQL backup";
        details = "Verifies backup archive integrity.";
        usage = [ "nix run .#service::postgres::verify-backup -- <backup-path>" ];
      };
      cleanup-backups = {
        script = backupMod.cleanupBackups;
        summary = "Prune old PostgreSQL backups";
        details = "Removes old backups while keeping the requested number of newest snapshots.";
        usage = [ "nix run .#service::postgres::cleanup-backups -- <keep-count>" ];
      };
      test-migrations = {
        script = migration.testMigrations;
        hook = "TEST_MIGRATIONS";
        summary = "Test PostgreSQL migrations";
        details = "Runs migrations against a temporary copy of the source database.";
      };
      ensure-migration-tested = {
        script = migrationSafety.ensureMigrationTested;
        hook = "ENSURE_MIGRATION_TESTED";
        summary = "Ensure migrations were tested";
        details = "Fails when migration hashes were not previously tested.";
        app = false;
      };
      check-port = {
        script = portMgmt.checkPort;
        hook = "CHECK_PORT";
        summary = "Check PostgreSQL port usage";
        details = "Checks whether a port is already in use.";
        usage = [ "nix run .#service::postgres::check-port -- <port>" ];
      };
      kill-port = {
        script = portMgmt.killPort;
        hook = "KILL_PORT";
        summary = "Kill processes bound to a port";
        details = "Stops processes listening on a given port.";
        usage = [ "nix run .#service::postgres::kill-port -- <port>" ];
      };
      shell = {
        script = shell;
        summary = "Open PostgreSQL shell";
        details = "Opens psql connected to the configured slot/environment database.";
        usage = [ "nix run .#service::postgres::shell -- <psql-args>" ];
      };
    };
  };
in
{
  # Lifecycle (backward compat)
  inherit (lifecycle)
    postgres
    init
    start
    stop
    setupDb
    fullStart
    fullStartTest
    listInstances
    ;

  # Extended lifecycle
  inherit
    restart
    status
    health
    checkConfig
    shell
    ;

  # Backup
  inherit (backupMod)
    archiveWal
    setupArchiving
    backup
    restore
    listBackups
    verifyBackup
    cleanupBackups
    ;

  # Migration
  inherit (migration) testMigrations;

  # Migration safety
  inherit (migrationSafety)
    getMigrationHash
    ensureMigrationTested
    markMigrationTested
    detectDrift
    ;

  # Rollback
  inherit (rollback) findBackupForCommit testRollback;

  # Port management
  inherit (portMgmt)
    checkPort
    getPortPids
    getPortInfo
    killPort
    assertPortsFree
    ;

  # Config (for reference)
  inherit config;

  # Service API contract
  inherit publicApi;
}
