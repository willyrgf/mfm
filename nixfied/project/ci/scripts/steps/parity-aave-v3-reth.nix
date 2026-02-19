{ pkgs }:
pkgs.writeText "mfm-ci-steps-parity-aave-v3-reth.sh" ''
  source <($SLOT_INFO)

  if [ -z "''${MFM_EPHEMERAL_ROOT:-}" ]; then
    echo "ERROR: parity-aave-v3-reth requires MFM_EPHEMERAL_ROOT for isolated runtime state" >&2
    exit 3
  fi

  export XDG_DATA_HOME="$MFM_EPHEMERAL_ROOT/data"

  export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

  export MFM_S3_ENDPOINT="$MINIO_ENDPOINT"
  export MFM_S3_REGION="$MINIO_REGION"
  export MFM_S3_BUCKET="$MINIO_BUCKET"
  export MFM_S3_PREFIX="$MINIO_PREFIX"

  export MFM_EVM_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"
  export MFM_EVM_RPC_SOURCES_JSON="[{\"id\":\"user_primary\",\"rpc_url\":\"http://127.0.0.1:$RETHHTTP_PORT\",\"kind\":\"remote_user\"}]"
  export MFM_EVM_RPC_PREFERRED_ORDER="user_primary"
  export MFM_EVM_RPC_SOURCE_ID="user_primary"
  export MFM_PARITY_AAVE_V3_RUN_IDS_PATH="$CI_ARTIFACTS_DIR/parity-aave-v3-run-ids.json"
  export MFM_PARITY_AAVE_V3_RETH_PROBE_PATH="$(artifact_path "parity-aave-v3-reth-probe.jsonl")"
  : > "$MFM_PARITY_AAVE_V3_RETH_PROBE_PATH"

  run_hook SVC_MINIO_BUCKET_ENSURE "$MINIO_BUCKET"

  probe_reth() {
    local phase="$1"
    local status_line=""
    local running=""
    local pid=""
    local wait_reason=""
    local log_path=""
    local now=""
    local health_ok=0

    status_line="$(run_hook SVC_RETH_STATUS 2>/dev/null | tail -n 1 || true)"
    running="$(echo "$status_line" | sed -n 's/.* running=\([^ ]*\).*/\1/p')"
    pid="$(echo "$status_line" | sed -n 's/.* pid=\([^ ]*\).*/\1/p')"
    wait_reason="$(echo "$status_line" | sed -n 's/.* wait_reason=\([^ ]*\).*/\1/p')"
    log_path="$(echo "$status_line" | sed -n 's/.* log_path=\([^ ]*\).*/\1/p')"
    now="$(date -u +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || true)"

    if run_hook SVC_RETH_HEALTH >/dev/null 2>&1; then
      health_ok=1
    fi

    ${pkgs.jq}/bin/jq -cn \
      --arg timestamp "''${now:-unknown}" \
      --arg phase "$phase" \
      --arg running "''${running:-unknown}" \
      --argjson health_ok "$health_ok" \
      --arg pid "''${pid:-unknown}" \
      --arg wait_reason "''${wait_reason:-none}" \
      --arg log_path "''${log_path:-none}" \
      '{
        timestamp: $timestamp,
        phase: $phase,
        running: $running,
        health_ok: $health_ok,
        pid: $pid,
        wait_reason: $wait_reason,
        log_path: $log_path
      }' >> "$MFM_PARITY_AAVE_V3_RETH_PROBE_PATH"

    [ "''${running:-}" = "true" ] && [ "$health_ok" -eq 1 ]
  }

  reth_ready_with_retry() {
    for _ in $(seq 1 80); do
      if run_hook SVC_RETH_READY >/dev/null 2>&1; then
        return 0
      fi
      sleep 0.25
    done
    return 1
  }

  ensure_reth_liveness() {
    if probe_reth "preflight"; then
      return 0
    fi

    echo "WARN: reth preflight probe failed; attempting one bounded restart" >&2
    if ! run_hook SVC_RETH_RESTART >/dev/null 2>&1; then
      probe_reth "restart_failed" || true
      echo "ERROR: failed to restart reth for parity-aave-v3-reth" >&2
      return 1
    fi

    if ! reth_ready_with_retry; then
      probe_reth "restart_not_ready" || true
      echo "ERROR: restarted reth did not become ready within retry window" >&2
      return 1
    fi

    if ! probe_reth "post_restart"; then
      echo "ERROR: reth remained unhealthy after bounded restart" >&2
      return 1
    fi
  }

  trap 'probe_reth "post_test" || true' EXIT

  ensure_reth_liveness

  LOGFILE=$(artifact_path "parity-aave-v3-reth.log")
  log_capture "$LOGFILE" -- cargo-nightly nextest run --cargo-profile ci -p mfm-integration-tests --features parity-tests --test parity_aave_v3_reth_scenario
''
