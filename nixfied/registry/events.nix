{ pkgs }:
{
  mkShellLib =
    { }:
    ''
      registry_lock_close_fd() {
        local lock_fd="$1"
        eval "exec $lock_fd>&-" 2>/dev/null || eval "exec $lock_fd<&-" 2>/dev/null || true
      }

      registry_lock_meta_file() {
        local lock_file="$1"
        printf '%s.meta' "$lock_file"
      }

      registry_lock_owner_summary() {
        local meta_file="$1"
        if [ ! -f "$meta_file" ]; then
          printf 'unknown'
          return 0
        fi

        ${pkgs.gawk}/bin/awk '
          BEGIN {
            pid = ""
            hostname = ""
            purpose = ""
            acquired = ""
            started = ""
          }
          /^pid=/ { pid = substr($0, 5) }
          /^hostname=/ { hostname = substr($0, 10) }
          /^purpose=/ { purpose = substr($0, 9) }
          /^acquired_at=/ { acquired = substr($0, 13) }
          /^process_started_at=/ { started = substr($0, 20) }
          END {
            printf "pid=%s hostname=%s purpose=%s acquired_at=%s process_started_at=%s", pid, hostname, purpose, acquired, started
          }
        ' "$meta_file"
      }

      registry_lock_write_owner_metadata() {
        local lock_file="$1"
        local purpose="$2"
        local meta_file
        local acquired_at
        local hostname
        local process_started_at

        meta_file="$(registry_lock_meta_file "$lock_file")"
        acquired_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
        hostname="$(${pkgs.coreutils}/bin/uname -n 2>/dev/null || printf 'unknown')"
        process_started_at="$(${pkgs.procps}/bin/ps -o lstart= -p $$ 2>/dev/null | ${pkgs.gnused}/bin/sed -E 's/^[[:space:]]+//')"
        if [ -z "$process_started_at" ]; then
          process_started_at="$acquired_at"
        fi

        {
          printf 'pid=%s\n' "$$"
          printf 'hostname=%s\n' "$hostname"
          printf 'purpose=%s\n' "$purpose"
          printf 'acquired_at=%s\n' "$acquired_at"
          printf 'process_started_at=%s\n' "$process_started_at"
        } > "$meta_file"
      }

      registry_lock_acquire_mode() {
        local lock_file="$1"
        local purpose="$2"
        local lock_mode="$3"
        local timeout_seconds="''${4:-30}"
        local lock_fd
        local meta_file
        local -a flock_args

        if ! mkdir -p "$(dirname "$lock_file")"; then
          echo "ERROR: failed to prepare registry lock directory for '$lock_file'" >&2
          return 1
        fi
        if ! : >> "$lock_file"; then
          echo "ERROR: failed to prepare registry lock file '$lock_file'" >&2
          return 1
        fi

        if [ "$lock_mode" = "shared" ]; then
          if ! exec {lock_fd}<"$lock_file"; then
            echo "ERROR: failed to open registry lock '$lock_file' mode=shared" >&2
            return 1
          fi
          flock_args=(-s)
        else
          if ! exec {lock_fd}>>"$lock_file"; then
            echo "ERROR: failed to open registry lock '$lock_file' mode=exclusive" >&2
            return 1
          fi
          flock_args=()
        fi

        if ! ${pkgs.flock}/bin/flock -w "$timeout_seconds" "''${flock_args[@]}" "$lock_fd"; then
          meta_file="$(registry_lock_meta_file "$lock_file")"
          echo "ERROR: registry lock timeout purpose=$purpose lock=$lock_file owner=$(registry_lock_owner_summary "$meta_file")" >&2
          registry_lock_close_fd "$lock_fd"
          return 1
        fi

        if [ "$lock_mode" != "shared" ]; then
          registry_lock_write_owner_metadata "$lock_file" "$purpose"
        fi

        printf '%s' "$lock_fd"
      }

      registry_lock_acquire() {
        registry_lock_acquire_mode "$1" "$2" "exclusive" "''${3:-30}"
      }

      registry_lock_acquire_shared() {
        registry_lock_acquire_mode "$1" "$2" "shared" "''${3:-30}"
      }

      registry_lock_release() {
        local lock_fd="$1"
        local lock_file="$2"
        local meta_file
        local owner_pid=""

        meta_file="$(registry_lock_meta_file "$lock_file")"
        registry_lock_close_fd "$lock_fd"
        if [ -f "$meta_file" ]; then
          owner_pid="$(${pkgs.gnused}/bin/sed -n 's/^pid=//p' "$meta_file" | ${pkgs.coreutils}/bin/head -n 1 || true)"
          if [ "$owner_pid" = "$$" ]; then
            rm -f "$meta_file"
          fi
        fi
        return 0
      }

      registry_next_seq() {
        local root="$1"
        local seq_file="$root/.seq"
        local lock_file="$root/locks/seq.lock"
        local next="1"
        local lock_fd
        local tmp_file

        mkdir -p "$root"
        lock_fd="$(registry_lock_acquire "$lock_file" "registry-seq" 30)" || return 1
        if [ -f "$seq_file" ]; then
          next="$(( $(cat "$seq_file") + 1 ))"
        fi
        tmp_file="$(mktemp "$seq_file.tmp.XXXXXX")"
        printf '%s' "$next" > "$tmp_file"
        mv "$tmp_file" "$seq_file"
        registry_lock_release "$lock_fd" "$lock_file"

        printf '%s' "$next"
      }

      registry_events_snapshot() {
        local root="$1"
        local events_file="$root/events.ndjson"
        local lock_file="$root/locks/events.lock"
        local lock_fd
        local snapshot_file

        if [ ! -f "$events_file" ]; then
          printf '%s' ""
          return 0
        fi

        lock_fd="$(registry_lock_acquire_shared "$lock_file" "registry-events-snapshot" 30)" || return 1
        snapshot_file="$(mktemp "''${TMPDIR:-/tmp}/nixfied-events-snapshot.XXXXXX")"
        cp "$events_file" "$snapshot_file"
        registry_lock_release "$lock_fd" "$lock_file"
        printf '%s' "$snapshot_file"
      }

      registry_snapshot_cleanup() {
        local snapshot_file="$1"
        if [ -n "$snapshot_file" ] && [ -f "$snapshot_file" ]; then
          rm -f "$snapshot_file"
        fi
      }

      registry_append_event() {
        local root="$1"
        local run_id="$2"
        local workflow_id="$3"
        local task_id="$4"
        local state="$5"
        local detail_json="$6"

        local events_file="$root/events.ndjson"
        local seq_file="$root/.seq"
        local lock_file="$root/locks/events.lock"
        local lock_fd
        local seq
        local ts
        local rc
        local seq_tmp

        mkdir -p "$root"
        lock_fd="$(registry_lock_acquire "$lock_file" "registry-events-append" 30)" || return 1
        if [ -f "$seq_file" ]; then
          seq="$(( $(cat "$seq_file") + 1 ))"
        else
          seq="1"
        fi
        seq_tmp="$(mktemp "$seq_file.tmp.XXXXXX")"
        printf '%s' "$seq" > "$seq_tmp"
        mv "$seq_tmp" "$seq_file"
        ts="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

        set +e
        ${pkgs.jq}/bin/jq -cnS \
          --argjson schemaVersion 1 \
          --argjson seq "$seq" \
          --arg ts "$ts" \
          --arg runId "$run_id" \
          --arg workflowId "$workflow_id" \
          --arg taskId "$task_id" \
          --arg state "$state" \
          --argjson detail "$detail_json" \
          '{detail: $detail, runId: $runId, schemaVersion: $schemaVersion, seq: $seq, state: $state, taskId: $taskId, ts: $ts, workflowId: $workflowId}' \
          >> "$events_file"
        rc="$?"
        set -e
        registry_lock_release "$lock_fd" "$lock_file"
        return "$rc"
      }
    '';
}
