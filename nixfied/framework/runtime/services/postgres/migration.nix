# PostgreSQL migration testing - test migrations against DB copy
{
  pkgs,
  project,
  slots,
  config,
  loggingPrelude,
}:

let
  postgres = config.package or pkgs.postgresql_16;
  migrationsCfg = config.migrations or { };
  migrationsDir = migrationsCfg.dir or "migrations";
  migrateCommand = migrationsCfg.command or "";
  sourceDatabase = migrationsCfg.sourceDatabase or null;
  portKey = config.portKey or "postgres";
  portVar = slots.portVarName portKey;
  database = config.database or "app";

  testMigrations = pkgs.writeShellScript "postgres-test-migrations" ''
    ${loggingPrelude}

    set -euo pipefail

    ${
      if migrateCommand == "" then
        ''
          log_skip "No migration command configured (modules.postgres.migrations.command)"
          exit 0
        ''
      else
        ""
    }

    source <(${slots.getSlotInfo})

    PORT_VAR="${portVar}"
    export PGPORT="''${!PORT_VAR}"
    SOURCE_DB="''${1:-${if sourceDatabase != null then sourceDatabase else database}}"
    TEST_DB="migration_test_$(date +%s)"

    log_info "Testing migrations against copy of '$SOURCE_DB'"

    # Create test database as copy of source
    ${postgres}/bin/createdb -h localhost -p "$PGPORT" -U postgres -T "$SOURCE_DB" "$TEST_DB" 2>/dev/null || {
      log_error "Failed to copy database '$SOURCE_DB'"
      exit 1
    }

    # Run migrations against the copy
    MIGRATION_EXIT=0
    export PGDATABASE="$TEST_DB"
    (
      cd "$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
      ${migrateCommand}
    ) || MIGRATION_EXIT=$?

    # Drop the test database
    ${postgres}/bin/dropdb -h localhost -p "$PGPORT" -U postgres "$TEST_DB" 2>/dev/null || true

    if [ $MIGRATION_EXIT -ne 0 ]; then
      log_error "Migration test failed (exit code: $MIGRATION_EXIT)"
      exit $MIGRATION_EXIT
    fi

    log_ok "Migration test passed"
  '';

in
{
  inherit testMigrations;
}
