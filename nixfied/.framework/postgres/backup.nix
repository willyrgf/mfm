# PostgreSQL backup infrastructure - WAL archiving, base backups, PITR restore
{
  pkgs,
  project,
  slots,
  config,
  loggingPrelude,
}:

let
  dataDirName = config.dataDirName or "postgres";
  pgdataExpr = slots.getServiceDir dataDirName;
  postgres = config.package or pkgs.postgresql_16;
  portKey = config.portKey or "postgres";
  portVar = slots.portVarName portKey;

  archiveWal = pkgs.writeShellScript "postgres-archive-wal" ''
    ${loggingPrelude}

    set -euo pipefail

    if [ -z "''${PGDATA:-}" ] || [ -z "''${BACKUP_BASE_DIR:-}" ]; then
      log_error "PGDATA and BACKUP_BASE_DIR must be set"
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
    ${loggingPrelude}

    set -euo pipefail

    if [ -z "''${PGDATA:-}" ] || [ -z "''${BACKUP_BASE_DIR:-}" ]; then
      log_error "PGDATA and BACKUP_BASE_DIR must be set"
      exit 1
    fi

    WAL_ARCHIVE="$BACKUP_BASE_DIR/wal"
    mkdir -p "$WAL_ARCHIVE"

    log_info "Configuring WAL archiving"
    cat >> "$PGDATA/postgresql.conf" <<EOF
    archive_mode = on
    archive_command = 'cp %p $WAL_ARCHIVE/%f'
    EOF

    log_ok "WAL archiving configured to $WAL_ARCHIVE"
  '';

  backup = pkgs.writeShellScript "postgres-backup" ''
    ${loggingPrelude}

    set -euo pipefail
    source <(${slots.getSlotInfo})

    PORT_VAR="${portVar}"
    export PGPORT="''${!PORT_VAR}"

    if [ -z "''${BACKUP_BASE_DIR:-}" ]; then
      log_error "BACKUP_BASE_DIR must be set (run source <(\$SLOT_INFO) first)"
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

    log_info "Creating base backup: $BACKUP_NAME"
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

    log_ok "Backup created: $BACKUP_PATH"
    echo "   Manifest: $BACKUP_PATH.manifest.json"
  '';

  restore = pkgs.writeShellScript "postgres-restore" ''
    ${loggingPrelude}

    set -euo pipefail

    STAGED_PGDATA=""
    PREVIOUS_PGDATA=""

    cleanup_restore() {
      local rc="$?"

      if [ "$rc" -ne 0 ] && [ -n "$PREVIOUS_PGDATA" ] && [ -d "$PREVIOUS_PGDATA" ] && [ ! -e "$PGDATA" ]; then
        mv "$PREVIOUS_PGDATA" "$PGDATA" 2>/dev/null || true
      fi

      if [ -n "$STAGED_PGDATA" ] && [ -d "$STAGED_PGDATA" ]; then
        rm -rf "$STAGED_PGDATA"
      fi

      return "$rc"
    }

    validate_restored_pgdata() {
      local candidate_dir="$1"

      if [ ! -f "$candidate_dir/PG_VERSION" ]; then
        log_error "restored backup is missing PG_VERSION dir=$candidate_dir"
        exit 1
      fi
    }

    trap cleanup_restore EXIT

    source <(${slots.getSlotInfo})

    BACKUP_PATH="''${1:-}"
    if [ -z "$BACKUP_PATH" ]; then
      echo "usage: postgres-restore <backup-path>" >&2
      exit 1
    fi

    if [ -z "''${PGDATA:-}" ]; then
      log_error "PGDATA must be set"
      exit 1
    fi

    EXPECTED_PGDATA="${pgdataExpr}"
    if [ "$PGDATA" != "$EXPECTED_PGDATA" ] && [ "''${POSTGRES_RESTORE_ALLOW_CUSTOM_PGDATA:-0}" != "1" ]; then
      log_error "PGDATA must match the slot/env postgres data dir or set POSTGRES_RESTORE_ALLOW_CUSTOM_PGDATA=1"
      echo "DETAIL: expected=$EXPECTED_PGDATA actual=$PGDATA" >&2
      exit 1
    fi

    case "$PGDATA" in
      ""|"/"|"/tmp"|"/var"|"/usr"|"/etc"|"/bin"|"/sbin"|"/opt"|"/home"|"/Users")
        log_error "refusing to restore using unsafe PGDATA path=$PGDATA"
        exit 1
        ;;
    esac
    case "$PGDATA" in
      /*) ;;
      *)
        log_error "PGDATA must be an absolute path (got '$PGDATA')"
        exit 1
        ;;
    esac

    if [ ! -d "$BACKUP_PATH" ] && [ ! -f "$BACKUP_PATH/base.tar.gz" ]; then
      log_error "Backup not found: $BACKUP_PATH"
      exit 1
    fi

    log_info "Restoring from backup: $BACKUP_PATH"

    # Stop postgres if running
    if [ -f "$PGDATA/postmaster.pid" ]; then
      ${postgres}/bin/pg_ctl -D "$PGDATA" stop -m fast 2>/dev/null || true
      sleep 2
    fi

    PGDATA_PARENT="$(dirname "$PGDATA")"
    mkdir -p "$PGDATA_PARENT"
    STAGED_PGDATA="$(${pkgs.coreutils}/bin/mktemp -d "$PGDATA_PARENT/.postgres-restore-staging.XXXXXX")"

    # Restore into a staging directory first so failed extracts do not destroy the live data dir.
    if [ -f "$BACKUP_PATH/base.tar.gz" ]; then
      tar xzf "$BACKUP_PATH/base.tar.gz" -C "$STAGED_PGDATA"
    else
      cp -a "$BACKUP_PATH/." "$STAGED_PGDATA/"
    fi

    validate_restored_pgdata "$STAGED_PGDATA"

    if [ -e "$PGDATA" ]; then
      PREVIOUS_PGDATA="$PGDATA.previous.$(${pkgs.coreutils}/bin/date +%Y%m%d-%H%M%S)-$$"
      mv "$PGDATA" "$PREVIOUS_PGDATA"
    fi

    mv "$STAGED_PGDATA" "$PGDATA"
    STAGED_PGDATA=""

    if [ -n "$PREVIOUS_PGDATA" ] && [ -d "$PREVIOUS_PGDATA" ]; then
      rm -rf "$PREVIOUS_PGDATA"
      PREVIOUS_PGDATA=""
    fi

    trap - EXIT
    log_ok "Backup restored to $PGDATA"
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
      CREATED=$(${pkgs.jq}/bin/jq -r '.created_at // "unknown"' "$manifest" 2>/dev/null || echo "unknown")
      COMMIT=$(${pkgs.jq}/bin/jq -r '.git_commit // "unknown"' "$manifest" 2>/dev/null || echo "unknown")
      echo "  $NAME (created: $CREATED, commit: $COMMIT)"
    done
  '';

  verifyBackup = pkgs.writeShellScript "postgres-verify-backup" ''
    ${loggingPrelude}

    set -euo pipefail

    BACKUP_PATH="''${1:-}"
    if [ -z "$BACKUP_PATH" ]; then
      echo "usage: postgres-verify-backup <backup-path>" >&2
      exit 1
    fi

    if [ ! -d "$BACKUP_PATH" ]; then
      log_error "Backup not found: $BACKUP_PATH"
      exit 1
    fi

    log_info "Verifying backup: $BACKUP_PATH"

    if [ -f "$BACKUP_PATH/base.tar.gz" ]; then
      if tar tzf "$BACKUP_PATH/base.tar.gz" >/dev/null 2>&1; then
        log_ok "Backup archive is valid"
      else
        log_error "Backup archive is corrupted"
        exit 1
      fi
    fi

    if [ -f "$BACKUP_PATH.manifest.json" ]; then
      echo "   Manifest found"
    else
      log_warn "No manifest found"
    fi

    log_ok "Backup verification passed"
  '';

  cleanupBackups = pkgs.writeShellScript "postgres-cleanup-backups" ''
    ${loggingPrelude}

    set -euo pipefail
    source <(${slots.getSlotInfo})

    BACKUP_DIR="$BACKUP_BASE_DIR/base"
    KEEP="''${1:-5}"

    if [ ! -d "$BACKUP_DIR" ]; then
      exit 0
    fi

    BACKUP_COUNT=$(ls -d "$BACKUP_DIR"/backup-* 2>/dev/null | wc -l || echo 0)
    if [ "$BACKUP_COUNT" -le "$KEEP" ]; then
      log_ok "$BACKUP_COUNT backups (keeping $KEEP), nothing to clean"
      exit 0
    fi

    TO_REMOVE=$((BACKUP_COUNT - KEEP))
    log_info "Removing $TO_REMOVE old backups (keeping newest $KEEP)"
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
