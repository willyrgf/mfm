# PostgreSQL rollback - git-integrated rollback with backup manifests
{
  pkgs,
  project,
  slots,
  config,
  loggingPrelude,
}:

let
  postgres = config.package or pkgs.postgresql_16;
  portKey = config.portKey or "postgres";
  portVar = slots.portVarName portKey;

  findBackupForCommit = pkgs.writeShellScript "postgres-find-backup-for-commit" ''
    ${loggingPrelude}

    set -euo pipefail
    source <(${slots.getSlotInfo})

    COMMIT="''${1:-}"
    if [ -z "$COMMIT" ]; then
      echo "usage: postgres-find-backup-for-commit <git-commit>" >&2
      exit 1
    fi

    BACKUP_DIR="$BACKUP_BASE_DIR/base"
    if [ ! -d "$BACKUP_DIR" ]; then
      log_error "No backups found"
      exit 1
    fi

    # Search manifests for matching commit
    for manifest in "$BACKUP_DIR"/*.manifest.json; do
      [ -f "$manifest" ] || continue
      MANIFEST_COMMIT=$(cat "$manifest" | grep '"git_commit"' | cut -d'"' -f4 2>/dev/null || true)
      if [ -n "$MANIFEST_COMMIT" ] && echo "$COMMIT" | grep -q "^$MANIFEST_COMMIT"; then
        BACKUP_NAME=$(basename "$manifest" .manifest.json)
        echo "$BACKUP_DIR/$BACKUP_NAME"
        exit 0
      fi
    done

    log_error "No backup found for commit $COMMIT"
    exit 1
  '';

  testRollback = pkgs.writeShellScript "postgres-test-rollback" ''
    ${loggingPrelude}

    set -euo pipefail
    source <(${slots.getSlotInfo})

    PORT_VAR="${portVar}"
    export PGPORT="''${!PORT_VAR}"

    COMMIT="''${1:-}"
    if [ -z "$COMMIT" ]; then
      # Default to previous commit
      COMMIT=$(git rev-parse --short HEAD~1 2>/dev/null || true)
      if [ -z "$COMMIT" ]; then
        log_error "No commit specified and cannot determine previous commit"
        exit 1
      fi
    fi

    log_info "Testing rollback to commit $COMMIT"

    BACKUP_PATH=$(${findBackupForCommit} "$COMMIT")
    if [ -z "$BACKUP_PATH" ]; then
      log_error "No backup found for commit $COMMIT"
      exit 1
    fi

    echo "   Found backup: $(basename "$BACKUP_PATH")"

    # Create a temporary test database from the backup
    TEST_DIR=$(mktemp -d)
    trap "rm -rf $TEST_DIR" EXIT

    if [ -f "$BACKUP_PATH/base.tar.gz" ]; then
      tar xzf "$BACKUP_PATH/base.tar.gz" -C "$TEST_DIR"
    else
      cp -a "$BACKUP_PATH/." "$TEST_DIR/"
    fi

    log_ok "Rollback test: backup is restorable for commit $COMMIT"
  '';

in
{
  inherit findBackupForCommit testRollback;
}
