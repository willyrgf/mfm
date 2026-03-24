{ pkgs }:
''
  registry_events_snapshot() {
    local root="$1"
    local events_file
    local lock_file
    local lock_fd
    local snapshot_file

    events_file="$(registry_events_file "$root")"
    lock_file="$(registry_events_lock_file "$root")"
    if [ ! -f "$events_file" ]; then
      printf '%s' ""
      return 0
    fi

    lock_fd="$(registry_lock_acquire_shared "$lock_file" "registry-events-snapshot" "$REGISTRY_DEFAULT_LOCK_TIMEOUT_SECONDS")" || return 1
    snapshot_file="$(mktemp "''${TMPDIR:-/tmp}/$REGISTRY_SNAPSHOT_TEMPLATE")"
    cp "$events_file" "$snapshot_file"
    registry_lock_release "$lock_fd" "$lock_file"
    printf '%s' "$snapshot_file"
  }

  registry_events_index_snapshot() {
    local root="$1"
    local index_file
    local lock_file
    local lock_fd
    local snapshot_file

    index_file="$(registry_events_index_file "$root")"
    lock_file="$(registry_events_lock_file "$root")"
    if [ ! -f "$index_file" ]; then
      printf '%s' ""
      return 0
    fi

    lock_fd="$(registry_lock_acquire_shared "$lock_file" "registry-events-index-snapshot" "$REGISTRY_DEFAULT_LOCK_TIMEOUT_SECONDS")" || return 1
    snapshot_file="$(mktemp "''${TMPDIR:-/tmp}/$REGISTRY_SNAPSHOT_TEMPLATE")"
    cp "$index_file" "$snapshot_file"
    registry_lock_release "$lock_fd" "$lock_file"
    printf '%s' "$snapshot_file"
  }

  registry_snapshot_cleanup() {
    local snapshot_file="$1"
    if [ -n "$snapshot_file" ] && [ -f "$snapshot_file" ]; then
      rm -f "$snapshot_file"
    fi
  }
''
