# PostgreSQL backup infrastructure - WAL archiving, base backups, PITR restore
{
  pkgs,
  project,
  slots,
}:

let
  cfg = project.modules.postgres or { };
  dataDirName = cfg.dataDirName or "postgres";
  pgdataExpr = slots.getServiceDir dataDirName;
  postgres = cfg.package or pkgs.postgresql_16;
  portKey = cfg.portKey or "postgres";
  portVar = slots.portVarName portKey;

  archiveWal = pkgs.writeShellScript "postgres-archive-wal" ''
    set -euo pipefail

    if [ -z "''${PGDATA:-}" ] || [ -z "''${BACKUP_BASE_DIR:-}" ]; then
      echo "ERROR: PGDATA and BACKUP_BASE_DIR must be set" >&2
      exit 1
    fi

    WAL_ARCHIVE="$BACKUP_BASE_DIR/wal"
    mkdir -p "$WAL_ARCHIVE"

    WAL_FILE="$1"
    WAL_NAME=$(basename "$WAL_FILE")

    cp "$WAL_FILE" "$WAL_ARCHIVE/$WAL_NAME"
    echo "   Archived WAL: $WAL_NAME"
  '';

  setupArchiving = pkgs.writeShellScript "postgres-setup-archiving" ''
    set -euo pipefail

    if [ -z "''${PGDATA:-}" ] || [ -z "''${BACKUP_BASE_DIR:-}" ]; then
      echo "ERROR: PGDATA and BACKUP_BASE_DIR must be set" >&2
      exit 1
    fi

    WAL_ARCHIVE="$BACKUP_BASE_DIR/wal"
    mkdir -p "$WAL_ARCHIVE"

    echo "INFO: Configuring WAL archiving"
    cat >> "$PGDATA/postgresql.conf" <<EOF
    archive_mode = on
    archive_command = 'cp %p $WAL_ARCHIVE/%f'
    EOF

    echo "OK: WAL archiving configured to $WAL_ARCHIVE"
  '';

  backup = pkgs.writeShellScript "postgres-backup" ''
    set -euo pipefail
    source <(${slots.getSlotInfo})

    PORT_VAR="${portVar}"
    export PGPORT="''${!PORT_VAR}"

    if [ -z "''${BACKUP_BASE_DIR:-}" ]; then
      echo "ERROR: BACKUP_BASE_DIR must be set (run source <(\$SLOT_INFO) first)" >&2
      exit 1
    fi

    BACKUP_DIR="$BACKUP_BASE_DIR/base"
    mkdir -p "$BACKUP_DIR"

    TIMESTAMP=$(date +%Y%m%d-%H%M%S)
    GIT_COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
    GIT_BRANCH=$(git branch --show-current 2>/dev/null || echo "unknown")
    BACKUP_NAME="backup-$TIMESTAMP"
    BACKUP_PATH="$BACKUP_DIR/$BACKUP_NAME"
    CREATED_AT="$(${pkgs.coreutils}/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"

    echo "INFO: Creating base backup: $BACKUP_NAME"
    ${postgres}/bin/pg_basebackup -h localhost -p "$PGPORT" -U postgres -D "$BACKUP_PATH" -Ft -z -P

    # Write backup manifest
    ${pkgs.jq}/bin/jq -n \
      --arg name "$BACKUP_NAME" \
      --arg timestamp "$TIMESTAMP" \
      --arg git_commit "$GIT_COMMIT" \
      --arg git_branch "$GIT_BRANCH" \
      --arg pgport "$PGPORT" \
      --arg slot "''${SLOT:-}" \
      --arg env "''${ENV:-}" \
      --arg created_at "$CREATED_AT" \
      '
      {
        name: $name,
        timestamp: $timestamp,
        git_commit: $git_commit,
        git_branch: $git_branch,
        pgport: $pgport,
        slot: (if $slot == "" then null else $slot end),
        env: (if $env == "" then null else $env end),
        created_at: $created_at
      }
      ' > "$BACKUP_PATH.manifest.json"

    echo "OK: Backup created: $BACKUP_PATH"
    echo "   Manifest: $BACKUP_PATH.manifest.json"
  '';

  restore = pkgs.writeShellScript "postgres-restore" ''
    set -euo pipefail

    source <(${slots.getSlotInfo})

    BACKUP_PATH="''${1:-}"
    if [ -z "$BACKUP_PATH" ]; then
      echo "usage: postgres-restore <backup-path>" >&2
      exit 1
    fi

    if [ -z "''${PGDATA:-}" ]; then
      echo "ERROR: PGDATA must be set" >&2
      exit 1
    fi

    EXPECTED_PGDATA="${pgdataExpr}"
    if [ "$PGDATA" != "$EXPECTED_PGDATA" ] && [ "''${POSTGRES_RESTORE_ALLOW_CUSTOM_PGDATA:-0}" != "1" ]; then
      echo "ERROR: PGDATA must match the slot/env postgres data dir or set POSTGRES_RESTORE_ALLOW_CUSTOM_PGDATA=1" >&2
      echo "DETAIL: expected=$EXPECTED_PGDATA actual=$PGDATA" >&2
      exit 1
    fi

    case "$PGDATA" in
      ""|"/"|"/tmp"|"/var"|"/usr"|"/etc"|"/bin"|"/sbin"|"/opt"|"/home"|"/Users")
        echo "ERROR: refusing to restore using unsafe PGDATA path=$PGDATA" >&2
        exit 1
        ;;
    esac
    case "$PGDATA" in
      /*) ;;
      *)
        echo "ERROR: PGDATA must be an absolute path (got '$PGDATA')" >&2
        exit 1
        ;;
    esac

    if [ ! -d "$BACKUP_PATH" ] && [ ! -f "$BACKUP_PATH/base.tar.gz" ]; then
      echo "ERROR: Backup not found: $BACKUP_PATH" >&2
      exit 1
    fi

    echo "INFO: Restoring from backup: $BACKUP_PATH"

    # Stop postgres if running
    if [ -f "$PGDATA/postmaster.pid" ]; then
      ${postgres}/bin/pg_ctl -D "$PGDATA" stop -m fast 2>/dev/null || true
      sleep 2
    fi

    # Clear existing data
    rm -rf "$PGDATA"
    mkdir -p "$PGDATA"

    # Restore from tar
    if [ -f "$BACKUP_PATH/base.tar.gz" ]; then
      tar xzf "$BACKUP_PATH/base.tar.gz" -C "$PGDATA"
    else
      cp -a "$BACKUP_PATH/." "$PGDATA/"
    fi

    echo "OK: Backup restored to $PGDATA"
  '';

  listBackups = pkgs.writeShellScript "postgres-list-backups" ''
    set -euo pipefail
    source <(${slots.getSlotInfo})

    BACKUP_DIR="$BACKUP_BASE_DIR/base"

    if [ ! -d "$BACKUP_DIR" ]; then
      echo "No backups found at $BACKUP_DIR"
      exit 0
    fi

    echo "PostgreSQL backups (slot $SLOT, env $ENV):"
    echo ""
    for manifest in "$BACKUP_DIR"/*.manifest.json; do
      [ -f "$manifest" ] || continue
      NAME=$(basename "$manifest" .manifest.json)
      CREATED=$(cat "$manifest" | grep '"created_at"' | cut -d'"' -f4 2>/dev/null || echo "unknown")
      COMMIT=$(cat "$manifest" | grep '"git_commit"' | cut -d'"' -f4 2>/dev/null || echo "unknown")
      echo "  $NAME (created: $CREATED, commit: $COMMIT)"
    done
  '';

  verifyBackup = pkgs.writeShellScript "postgres-verify-backup" ''
    set -euo pipefail

    BACKUP_PATH="''${1:-}"
    if [ -z "$BACKUP_PATH" ]; then
      echo "usage: postgres-verify-backup <backup-path>" >&2
      exit 1
    fi

    if [ ! -d "$BACKUP_PATH" ]; then
      echo "ERROR: Backup not found: $BACKUP_PATH" >&2
      exit 1
    fi

    echo "INFO: Verifying backup: $BACKUP_PATH"

    if [ -f "$BACKUP_PATH/base.tar.gz" ]; then
      if tar tzf "$BACKUP_PATH/base.tar.gz" >/dev/null 2>&1; then
        echo "OK: Backup archive is valid"
      else
        echo "ERROR: Backup archive is corrupted" >&2
        exit 1
      fi
    fi

    if [ -f "$BACKUP_PATH.manifest.json" ]; then
      echo "   Manifest found"
    else
      echo "WARN: No manifest found"
    fi

    echo "OK: Backup verification passed"
  '';

  cleanupBackups = pkgs.writeShellScript "postgres-cleanup-backups" ''
    set -euo pipefail
    source <(${slots.getSlotInfo})

    BACKUP_DIR="$BACKUP_BASE_DIR/base"
    KEEP="''${1:-5}"

    if [ ! -d "$BACKUP_DIR" ]; then
      exit 0
    fi

    BACKUP_COUNT=$(ls -d "$BACKUP_DIR"/backup-* 2>/dev/null | wc -l || echo 0)
    if [ "$BACKUP_COUNT" -le "$KEEP" ]; then
      echo "OK: $BACKUP_COUNT backups (keeping $KEEP), nothing to clean"
      exit 0
    fi

    TO_REMOVE=$((BACKUP_COUNT - KEEP))
    echo "INFO: Removing $TO_REMOVE old backups (keeping newest $KEEP)"
    ls -dt "$BACKUP_DIR"/backup-* | tail -n "$TO_REMOVE" | while read -r dir; do
      rm -rf "$dir" "$dir.manifest.json"
      echo "   Removed: $(basename "$dir")"
    done
  '';

in
{
  inherit
    archiveWal
    setupArchiving
    backup
    restore
    listBackups
    verifyBackup
    cleanupBackups
    ;
}
