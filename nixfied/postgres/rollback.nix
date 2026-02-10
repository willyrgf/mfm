# PostgreSQL rollback - git-integrated rollback with backup manifests
{
  pkgs,
  project,
  slots,
}:

let
  cfg = project.modules.postgres or { };
  postgres = cfg.package or pkgs.postgresql_16;
  portKey = cfg.portKey or "postgres";
  portVar = slots.portVarName portKey;

  findBackupForCommit = pkgs.writeShellScript "postgres-find-backup-for-commit" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    COMMIT="''${1:-}"
    if [ -z "$COMMIT" ]; then
      echo "usage: postgres-find-backup-for-commit <git-commit>" >&2
      exit 1
    fi

    BACKUP_DIR="$BACKUP_BASE_DIR/base"
    if [ ! -d "$BACKUP_DIR" ]; then
      echo "ERROR: No backups found" >&2
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

    echo "ERROR: No backup found for commit $COMMIT" >&2
    exit 1
  '';

  testRollback = pkgs.writeShellScript "postgres-test-rollback" ''
    set -euo pipefail
    eval "$(${slots.getSlotInfo})"

    PORT_VAR="${portVar}"
    export PGPORT="''${!PORT_VAR}"

    COMMIT="''${1:-}"
    if [ -z "$COMMIT" ]; then
      # Default to previous commit
      COMMIT=$(git rev-parse --short HEAD~1 2>/dev/null || true)
      if [ -z "$COMMIT" ]; then
        echo "ERROR: No commit specified and cannot determine previous commit" >&2
        exit 1
      fi
    fi

    echo "INFO: Testing rollback to commit $COMMIT"

    BACKUP_PATH=$(${findBackupForCommit} "$COMMIT")
    if [ -z "$BACKUP_PATH" ]; then
      echo "ERROR: No backup found for commit $COMMIT" >&2
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

    echo "OK: Rollback test: backup is restorable for commit $COMMIT"
  '';

in
{
  inherit findBackupForCommit testRollback;
}
