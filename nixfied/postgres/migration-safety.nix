# PostgreSQL migration safety - hash-based tracking, prod blocking
{
  pkgs,
  project,
  slots,
}:

let
  cfg = project.modules.postgres or { };
  migrationsCfg = cfg.migrations or { };
  migrationsDir = migrationsCfg.dir or "migrations";

  getMigrationHash = pkgs.writeShellScript "postgres-get-migration-hash" ''
    set -euo pipefail

    MIGRATIONS_PATH="''${1:-${migrationsDir}}"

    if [ ! -d "$MIGRATIONS_PATH" ]; then
      echo "no-migrations"
      exit 0
    fi

    find "$MIGRATIONS_PATH" -type f -name '*.sql' -o -name '*.ts' -o -name '*.js' | sort | xargs cat 2>/dev/null | shasum -a 256 | cut -d' ' -f1
  '';

  markerDir = pkgs.writeShellScript "postgres-marker-dir" ''
    echo "''${XDG_DATA_HOME:-$HOME/.local/share}/migration-markers"
  '';

  ensureMigrationTested = pkgs.writeShellScript "postgres-ensure-migration-tested" ''
    set -euo pipefail

    MIGRATIONS_PATH="''${1:-${migrationsDir}}"
    MARKER_DIR=$(${markerDir})
    mkdir -p "$MARKER_DIR"

    HASH=$(${getMigrationHash} "$MIGRATIONS_PATH")

    if [ "$HASH" = "no-migrations" ]; then
      echo "SKIP: No migrations found (skipping safety check)"
      exit 0
    fi

    MARKER_FILE="$MARKER_DIR/$HASH.tested"

    if [ -f "$MARKER_FILE" ]; then
      echo "OK: Migrations already tested (hash: ''${HASH:0:12}...)"
      exit 0
    fi

    echo "ERROR: Migrations have not been tested against a database copy" >&2
    echo "   Hash: $HASH" >&2
    echo "   Run 'run_hook POSTGRES_TEST_MIGRATIONS' first" >&2
    exit 1
  '';

  markMigrationTested = pkgs.writeShellScript "postgres-mark-migration-tested" ''
    set -euo pipefail

    MIGRATIONS_PATH="''${1:-${migrationsDir}}"
    MARKER_DIR=$(${markerDir})
    mkdir -p "$MARKER_DIR"

    HASH=$(${getMigrationHash} "$MIGRATIONS_PATH")

    if [ "$HASH" = "no-migrations" ]; then
      exit 0
    fi

    echo "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$MARKER_DIR/$HASH.tested"
    echo "OK: Migration marked as tested (hash: ''${HASH:0:12}...)"
  '';

  detectDrift = pkgs.writeShellScript "postgres-detect-drift" ''
    set -euo pipefail

    MIGRATIONS_PATH="''${1:-${migrationsDir}}"
    MARKER_DIR=$(${markerDir})

    CURRENT_HASH=$(${getMigrationHash} "$MIGRATIONS_PATH")

    if [ "$CURRENT_HASH" = "no-migrations" ]; then
      echo "SKIP: No migrations found"
      exit 0
    fi

    if [ -f "$MARKER_DIR/$CURRENT_HASH.tested" ]; then
      echo "OK: Migrations match tested hash (''${CURRENT_HASH:0:12}...)"
    else
      echo "WARN: Migration drift detected - current hash (''${CURRENT_HASH:0:12}...) not tested" >&2
      exit 1
    fi
  '';

in
{
  inherit
    getMigrationHash
    ensureMigrationTested
    markMigrationTested
    detectDrift
    ;
}
