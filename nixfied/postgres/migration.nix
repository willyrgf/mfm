# PostgreSQL migration testing - test migrations against DB copy
{
  pkgs,
  project,
  slots,
}:

let
  cfg = project.modules.postgres or { };
  postgres = cfg.package or pkgs.postgresql_16;
  migrationsCfg = cfg.migrations or { };
  migrationsDir = migrationsCfg.dir or "migrations";
  migrateCommand = migrationsCfg.command or "";
  sourceDatabase = migrationsCfg.sourceDatabase or null;
  portKey = cfg.portKey or "postgres";
  portVar = slots.portVarName portKey;
  database = cfg.database or "app";

  testMigrations = pkgs.writeShellScript "postgres-test-migrations" ''
    set -euo pipefail

    ${
      if migrateCommand == "" then
        ''
          echo "ℹ️  No migration command configured (modules.postgres.migrations.command)"
          echo "   Skipping migration test"
          exit 0
        ''
      else
        ""
    }

    eval "$(${slots.getSlotInfo})"

    PORT_VAR="${portVar}"
    export PGPORT="''${!PORT_VAR}"
    SOURCE_DB="''${1:-${if sourceDatabase != null then sourceDatabase else database}}"
    TEST_DB="migration_test_$(date +%s)"

    echo "🧪 Testing migrations against copy of '$SOURCE_DB'..."

    # Create test database as copy of source
    ${postgres}/bin/createdb -h localhost -p "$PGPORT" -U postgres -T "$SOURCE_DB" "$TEST_DB" 2>/dev/null || {
      echo "❌ Failed to copy database '$SOURCE_DB'" >&2
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
      echo "❌ Migration test failed (exit code: $MIGRATION_EXIT)" >&2
      exit $MIGRATION_EXIT
    fi

    echo "✅ Migration test passed"
  '';

in
{
  inherit testMigrations;
}
