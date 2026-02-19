{ pkgs }:
{
  mkShellLib =
    { }:
    ''
      registry_lock_acquire() {
        local lock_dir="$1"
        while ! mkdir "$lock_dir" 2>/dev/null; do
          sleep 0.02
        done
      }

      registry_lock_release() {
        local lock_dir="$1"
        if [ -d "$lock_dir" ]; then
          rmdir "$lock_dir"
        fi
      }

      registry_next_seq() {
        local root="$1"
        local seq_file="$root/.seq"
        local lock_dir="$root/.seq-lock"
        local next="1"

        mkdir -p "$root"
        registry_lock_acquire "$lock_dir"
        if [ -f "$seq_file" ]; then
          next="$(( $(cat "$seq_file") + 1 ))"
        fi
        printf '%s' "$next" > "$seq_file"
        registry_lock_release "$lock_dir"

        printf '%s' "$next"
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
        local lock_dir="$root/.events-lock"
        local seq
        local ts
        local rc

        mkdir -p "$root"
        registry_lock_acquire "$lock_dir"
        if [ -f "$seq_file" ]; then
          seq="$(( $(cat "$seq_file") + 1 ))"
        else
          seq="1"
        fi
        printf '%s' "$seq" > "$seq_file"
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
        registry_lock_release "$lock_dir"
        return "$rc"
      }
    '';
}
