{
  pkgs,
  registryEventPayloadExpr,
}:
let
  lib = pkgs.lib;
in
''
  registry_next_seq() {
    local root="$1"
    local seq_file
    local lock_file
    local next="1"
    local lock_fd
    local tmp_file

    seq_file="$(registry_seq_file "$root")"
    lock_file="$(registry_seq_lock_file "$root")"
    mkdir -p "$root"
    lock_fd="$(registry_lock_acquire "$lock_file" "registry-seq" "$REGISTRY_DEFAULT_LOCK_TIMEOUT_SECONDS")" || return 1
    if [ -f "$seq_file" ]; then
      next="$(( $(cat "$seq_file") + 1 ))"
    fi
    tmp_file="$(mktemp "$seq_file.tmp.XXXXXX")"
    printf '%s' "$next" > "$tmp_file"
    mv "$tmp_file" "$seq_file"
    registry_lock_release "$lock_fd" "$lock_file"

    printf '%s' "$next"
  }

  registry_append_event() {
    local root="$1"
    local run_id="$2"
    local workflow_id="$3"
    local task_id="$4"
    local state="$5"
    local detail_json="$6"

    local events_file
    local seq_file
    local lock_file
    local lock_fd
    local seq
    local ts
    local rc
    local seq_tmp

    events_file="$(registry_events_file "$root")"
    seq_file="$(registry_seq_file "$root")"
    lock_file="$(registry_events_lock_file "$root")"
    mkdir -p "$root"
    lock_fd="$(registry_lock_acquire "$lock_file" "registry-events-append" "$REGISTRY_DEFAULT_LOCK_TIMEOUT_SECONDS")" || return 1
    if [ -f "$seq_file" ]; then
      seq="$(( $(cat "$seq_file") + 1 ))"
    else
      seq="1"
    fi
    seq_tmp="$(mktemp "$seq_file.tmp.XXXXXX")"
    printf '%s' "$seq" > "$seq_tmp"
    mv "$seq_tmp" "$seq_file"
    ts="$(date -u +"$REGISTRY_TIMESTAMP_FORMAT")"

    set +e
    ${pkgs.jq}/bin/jq -cnS \
      --argjson schemaVersion "$REGISTRY_EVENT_SCHEMA_VERSION" \
      --argjson seq "$seq" \
      --arg ts "$ts" \
      --arg runId "$run_id" \
      --arg workflowId "$workflow_id" \
      --arg taskId "$task_id" \
      --arg state "$state" \
      --argjson detail "$detail_json" \
      ${lib.escapeShellArg registryEventPayloadExpr} \
      >> "$events_file"
    rc="$?"
    set -e
    registry_lock_release "$lock_fd" "$lock_file"
    return "$rc"
  }
''
