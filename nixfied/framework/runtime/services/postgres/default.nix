# PostgreSQL module aggregator
{
  pkgs,
  project,
  slots,
}:

let
  runtimeDefaults = import ../../../core/runtime-defaults.nix;
  summary = import ../../helpers/summary.nix { inherit pkgs project; };
  helpers = import ../../helpers/helpers.nix {
    inherit pkgs project;
    inherit (summary) summaryParser;
  };
  loggingPrelude = helpers.loggingPrelude;
  serviceModule = import ../../helpers/service-module.nix {
    inherit
      pkgs
      project
      slots
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

    exec ${pgPackage}/bin/psql "postgresql://${runtimeDefaults.hosts.localhost}:$PGPORT/$PGDATABASE" "$@"
  '';
in
serviceModule.mkServiceModule {
  service = "postgres";
  summaryName = "PostgreSQL";
  summary = "PostgreSQL service management API";
  details = "Public service contract for managing PostgreSQL across dev/prod/test/ci.";
  artifacts = {
    portKey = portKey;
    portVar = portVar;
    serviceDir = pgdataExpr;
    dataDir = pgdataExpr;
    logFile = "${pgdataExpr}/postgres.log";
    pidFile = "${pgdataExpr}/postmaster.pid";
    defaultDatabase = pgDatabase;
    testDatabase = testDatabase;
  };
  inherit config;
  operations = {
    init = {
      script = lifecycle.initLeaf;
      preOps = [ "preflight-init" ];
      summary = "Initialize PostgreSQL data directory";
      details = "Initializes PGDATA and writes environment-specific PostgreSQL configuration.";
    };
    preflight-init = {
      script = lifecycle.preflightInit;
      summary = "Validate PostgreSQL init preconditions";
      details = "Checks deterministic blockers before PostgreSQL initialization for the current slot and environment.";
      exposeApp = false;
      exposeHook = false;
    };
    preflight-start = {
      script = lifecycle.preflightStart;
      summary = "Validate PostgreSQL start preconditions";
      details = "Checks deterministic blockers before PostgreSQL startup for the current slot and environment.";
      exposeApp = false;
      exposeHook = false;
    };
    start = {
      script = lifecycle.startLeaf;
      preOps = [
        "init"
        "check-config"
        "preflight-start"
      ];
      summary = "Start PostgreSQL server";
      details = "Starts PostgreSQL for the current slot and environment.";
    };
    stop = {
      script = lifecycle.stop;
      summary = "Stop PostgreSQL server";
      details = "Stops PostgreSQL for the current slot and environment.";
    };
    restart = {
      preOps = [
        "stop"
        "start"
      ];
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
    setup-db = {
      script = lifecycle.setupDb;
      hook = "SETUP_DB";
      summary = "Create and configure database";
      details = "Creates the configured database and required extensions.";
    };
    full-start = {
      script = lifecycle.fullStartLeaf;
      hook = "FULL_START";
      preOps = [ "start" ];
      summary = "Init, start, and set up PostgreSQL";
      details = "Performs init/start/setup-db in one operation.";
    };
    full-start-test = {
      script = lifecycle.fullStartTestLeaf;
      hook = "FULL_START_TEST";
      preOps = [ "start" ];
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
  };
  exported = {
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

    inherit shell;

    inherit (backupMod)
      archiveWal
      setupArchiving
      backup
      restore
      listBackups
      verifyBackup
      cleanupBackups
      ;

    inherit (migration) testMigrations;

    inherit (migrationSafety)
      getMigrationHash
      ensureMigrationTested
      markMigrationTested
      detectDrift
      ;

    inherit (rollback) findBackupForCommit testRollback;

    inherit (portMgmt)
      checkPort
      getPortPids
      getPortInfo
      killPort
      assertPortsFree
      ;
  };
}
