# PostgreSQL module aggregator
{
  pkgs,
  project,
  slots,
}:

let
  cfg = project.modules.postgres or { };
  summary = import ../lib/summary.nix { inherit pkgs project; };
  helpers = import ../lib/helpers.nix {
    inherit pkgs project;
    inherit (summary) summaryParser;
  };
  loggingPrelude = helpers.loggingPrelude;
  serviceApi = import ../lib/service-api.nix { inherit pkgs; };
  processRegistry = import ../lib/process-registry.nix { inherit pkgs project; };
  observability = import ../lib/service-observability.nix {
    inherit
      pkgs
      slots
      processRegistry
      ;
  };
  pgPackage = cfg.package or pkgs.postgresql_16;
  pgDatabase = cfg.database or "app";
  testDatabase = cfg.testDatabase or "${pgDatabase}_test";
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
      loggingPrelude
      ;
  };
  backupMod = import ./backup.nix {
    inherit pkgs project slots loggingPrelude;
  };
  migration = import ./migration.nix {
    inherit pkgs project slots loggingPrelude;
  };
  migrationSafety = import ./migration-safety.nix {
    inherit pkgs project slots loggingPrelude;
  };
  rollback = import ./rollback.nix {
    inherit pkgs project slots loggingPrelude;
  };
  portMgmt = import ./port-management.nix {
    inherit pkgs loggingPrelude;
  };

  restart = pkgs.writeShellScript "postgres-restart" ''
    set -euo pipefail
    source <(${slots.getSlotInfo})

    PORT_VAR="${portVar}"
    export PGPORT="''${!PORT_VAR}"
    export PGDATA="${pgdataExpr}"
    SOCKET_HASH=$(printf '%s' "''${RUN_DIR:-$PGDATA}" | ${pkgs.coreutils}/bin/cksum | ${pkgs.coreutils}/bin/cut -d ' ' -f1)
    export PGSOCKET_DIR="''${PGSOCKET_DIR:-/tmp/nixfied-pg-$SOCKET_HASH}"

    ${lifecycle.stop}
    ${lifecycle.start}
  '';

  status = pkgs.writeShellScript "postgres-status" ''
    set -euo pipefail
    source <(${slots.getSlotInfo})

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

    ${observability.mkStatusMergeBlock {
      service = "postgres";
      defaultLogPathExpr = ''"$PGDATA/postgres.log"'';
    }}

    echo "service=postgres slot=$SLOT env=$ENV port=$PGPORT pgdata=$PGDATA running=$RUNNING pid=''${PID:-unknown} scope=$SCOPE owner_run_id=''${OWNER_RUN_ID:-unknown} owner_scope=''${OWNER_SCOPE:-unknown} ephemeral_root=''${EPHEMERAL_ROOT:-none} registry_state=''${REGISTRY_STATE:-unknown} slot_owner=''${SLOT_OWNER:-unknown} wait_reason=''${WAIT_REASON:-none} log_path=$EFFECTIVE_LOG_PATH"

    if [ "$RUNNING" = "true" ]; then
      exit 0
    fi
    exit 1
  '';

  health = pkgs.writeShellScript "postgres-health" ''
    ${loggingPrelude}

    set -euo pipefail
    source <(${slots.getSlotInfo})

    PORT_VAR="${portVar}"
    PGPORT="''${PGPORT:-''${!PORT_VAR}}"

    if ${pgPackage}/bin/pg_isready -U postgres -h localhost -p "$PGPORT" -q 2>/dev/null; then
      log_ok "PostgreSQL healthy port=$PGPORT"
      exit 0
    fi

    log_error "PostgreSQL unhealthy port=$PGPORT"
    exit 1
  '';

  ready = pkgs.writeShellScript "postgres-ready" ''
    ${loggingPrelude}

    set -euo pipefail
    source <(${slots.getSlotInfo})

    PORT_VAR="${portVar}"
    PGPORT="''${PGPORT:-''${!PORT_VAR}}"

    if ! ${pgPackage}/bin/pg_isready -U postgres -h localhost -p "$PGPORT" -q 2>/dev/null; then
      log_error "PostgreSQL not ready port=$PGPORT (pg_isready failed)"
      exit 1
    fi

    if ${pgPackage}/bin/psql -h localhost -p "$PGPORT" -U postgres -d postgres -Atqc "select 1;" >/dev/null 2>&1; then
      log_ok "PostgreSQL ready port=$PGPORT"
      exit 0
    fi

    log_error "PostgreSQL not ready port=$PGPORT (query failed)"
    exit 1
  '';

  readyTest = pkgs.writeShellScript "postgres-ready-test" ''
    ${loggingPrelude}

    set -euo pipefail
    source <(${slots.getSlotInfo})

    PORT_VAR="${portVar}"
    PGPORT="''${PGPORT:-''${!PORT_VAR}}"
    PGDATABASE="''${PGDATABASE:-${testDatabase}}"

    if ! ${pgPackage}/bin/pg_isready -U postgres -h localhost -p "$PGPORT" -q 2>/dev/null; then
      log_error "PostgreSQL not ready for test db port=$PGPORT database=$PGDATABASE (pg_isready failed)"
      exit 1
    fi

    if ! ${pgPackage}/bin/psql -h localhost -p "$PGPORT" -U postgres -d postgres -Atqc "select 1;" >/dev/null 2>&1; then
      log_error "PostgreSQL not ready for test db port=$PGPORT database=$PGDATABASE (maintenance query failed)"
      exit 1
    fi

    if ${pgPackage}/bin/psql -h localhost -p "$PGPORT" -U postgres -d "$PGDATABASE" -Atqc "select 1;" >/dev/null 2>&1; then
      log_ok "PostgreSQL ready for test db port=$PGPORT database=$PGDATABASE"
      exit 0
    fi

    log_error "PostgreSQL not ready for test db port=$PGPORT database=$PGDATABASE (database query failed)"
    exit 1
  '';

  checkConfig = pkgs.writeShellScript "postgres-check-config" ''
    ${loggingPrelude}

    set -euo pipefail
    source <(${slots.getSlotInfo})

    PGDATA="${pgdataExpr}"

    if [ ! -f "$PGDATA/postgresql.conf" ]; then
      log_error "missing postgresql.conf at $PGDATA"
      exit 1
    fi

    if ${pgPackage}/bin/postgres -D "$PGDATA" -C port >/dev/null 2>&1; then
      log_ok "PostgreSQL configuration valid pgdata=$PGDATA"
      exit 0
    fi

    log_error "PostgreSQL configuration invalid pgdata=$PGDATA"
    exit 1
  '';

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
        script = ready;
        hook = "READY";
        summary = "Wait for PostgreSQL readiness";
        details = "Checks PostgreSQL accepts local SQL queries on the configured port.";
      };
      ready-test = {
        script = readyTest;
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
    ready
    checkConfig
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
