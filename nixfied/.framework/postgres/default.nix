# PostgreSQL module aggregator
{
  pkgs,
  project,
  slots,
}:

let
  summary = import ../lib/summary.nix { inherit pkgs project; };
  helpers = import ../lib/helpers.nix {
    inherit pkgs project;
    inherit (summary) summaryParser;
  };
  loggingPrelude = helpers.loggingPrelude;
  serviceApi = import ../lib/service-api.nix { inherit pkgs; };
  runtimeEvents = import ../lib/runtime-events.nix { inherit pkgs project; };
  observability = import ../lib/service-observability.nix {
    inherit
      pkgs
      slots
      runtimeEvents
      ;
  };
  config = import ./config.nix { inherit pkgs project; };
  pgPackage = config.package or pkgs.postgresql_16;
  pgDatabase = config.database or "app";
  testDatabase = config.testDatabase or "${pgDatabase}_test";
  portKey = config.portKey or "postgres";
  portVar = slots.portVarName portKey;
  dataDirName = config.dataDirName or "postgres";
  pgdataExpr = slots.getServiceDir dataDirName;
  runtimePrimitives = serviceApi.mkRuntimePrimitivesV1 {
    logLevelDefault = toString ((project.logging or { }).level or "info");
    outputModeDefault = toString ((project.logging or { }).output or "stdout");
  };

  lifecycle = import ./lifecycle.nix {
    inherit
      pkgs
      project
      slots
      config
      loggingPrelude
      ;
  };
  backupMod = import ./backup.nix {
    inherit
      pkgs
      project
      slots
      config
      loggingPrelude
      ;
  };
  migration = import ./migration.nix {
    inherit
      pkgs
      project
      slots
      config
      loggingPrelude
      ;
  };
  migrationSafety = import ./migration-safety.nix {
    inherit
      pkgs
      project
      slots
      config
      loggingPrelude
      ;
  };
  rollback = import ./rollback.nix {
    inherit
      pkgs
      project
      slots
      config
      loggingPrelude
      ;
  };
  portMgmt = import ./port-management.nix {
    inherit pkgs loggingPrelude;
  };
  shell = pkgs.writeShellScript "postgres-shell" ''
    set -euo pipefail
    source <(${slots.getSlotInfo})

    PORT_VAR="${portVar}"
    PGPORT="''${!PORT_VAR}"
    PGDATABASE="''${PGDATABASE:-${pgDatabase}}"

    exec ${pgPackage}/bin/psql "postgresql://localhost:$PGPORT/$PGDATABASE" "$@"
  '';

  logs = observability.mkLogScript "postgres";
  log = logs;

  events = observability.mkEventsScript "postgres";
  logEventExtensions = observability.mkLogEventExtensions {
    service = "postgres";
    summaryName = "PostgreSQL";
    logScript = log;
    eventsScript = events;
  };

  publicApi = serviceApi.mkServiceApiV3 {
    service = "postgres";
    summary = "PostgreSQL service management API";
    details = "Public service contract for managing PostgreSQL across dev/prod/test/ci.";
    inherit runtimePrimitives;
    artifacts = {
      portKey = portKey;
      portVar = portVar;
      dataDir = pgdataExpr;
      defaultDatabase = pgDatabase;
      testDatabase = testDatabase;
    };
    operations = {
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
        script = lifecycle.restart;
        summary = "Restart PostgreSQL server";
        details = "Stops then starts PostgreSQL for the current slot and environment.";
      };
      status = {
        script = lifecycle.status;
        summary = "Show PostgreSQL status";
        details = "Prints PostgreSQL status for the current slot and environment.";
      };
      health = {
        script = lifecycle.health;
        summary = "Run PostgreSQL health check";
        details = "Checks PostgreSQL readiness on the configured port.";
      };
      check-config = {
        script = lifecycle.checkConfig;
        summary = "Validate PostgreSQL configuration";
        details = "Validates postgresql.conf for the current slot and environment.";
      };
    }
    // {
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
      ready = {
        script = lifecycle.ready;
        hook = "READY";
        summary = "Wait for PostgreSQL readiness";
        details = "Checks PostgreSQL accepts local SQL queries on the configured port.";
      };
      ready-test = {
        script = lifecycle.readyTest;
        hook = "READY_TEST";
        summary = "Wait for PostgreSQL test-database readiness";
        details = "Checks PostgreSQL and the configured test database accept local SQL queries.";
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
        usage = [ "nix run .#svc::postgres::backup -- <args>" ];
      };
      restore = {
        script = backupMod.restore;
        summary = "Restore PostgreSQL backup";
        details = "Restores PostgreSQL data from a selected backup.";
        usage = [ "nix run .#svc::postgres::restore -- <backup-path>" ];
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
        usage = [ "nix run .#svc::postgres::verify-backup -- <backup-path>" ];
      };
      cleanup-backups = {
        script = backupMod.cleanupBackups;
        summary = "Prune old PostgreSQL backups";
        details = "Removes old backups while keeping the requested number of newest snapshots.";
        usage = [ "nix run .#svc::postgres::cleanup-backups -- <keep-count>" ];
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
        exposeApp = false;
      };
      check-port = {
        script = portMgmt.checkPort;
        hook = "CHECK_PORT";
        summary = "Check PostgreSQL port usage";
        details = "Checks whether a port is already in use.";
        usage = [ "nix run .#svc::postgres::check-port -- <port>" ];
      };
      kill-port = {
        script = portMgmt.killPort;
        hook = "KILL_PORT";
        summary = "Kill processes bound to a port";
        details = "Stops processes listening on a given port.";
        usage = [ "nix run .#svc::postgres::kill-port -- <port>" ];
      };
      shell = {
        script = shell;
        summary = "Open PostgreSQL shell";
        details = "Opens psql connected to the configured slot/environment database.";
        usage = [ "nix run .#svc::postgres::shell -- <psql-args>" ];
      };
    }
    // logEventExtensions;
  };
in
{
  # Lifecycle (backward compat)
  inherit (lifecycle)
    postgres
    init
    start
    stop
    restart
    status
    health
    ready
    readyTest
    checkConfig
    setupDb
    fullStart
    fullStartTest
    listInstances
    ;

  # Extended lifecycle
  inherit
    shell
    log
    logs
    events
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
