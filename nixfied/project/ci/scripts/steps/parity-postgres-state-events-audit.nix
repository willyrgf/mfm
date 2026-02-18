{ pkgs }:
pkgs.writeText "mfm-ci-steps-parity-postgres-state-events-audit.sh" ''
  source <($SLOT_INFO)

  export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"
  export MFM_PARITY_EVM_RETH_RUN_IDS_PATH="$CI_ARTIFACTS_DIR/parity-evm-reth-run-ids.json"
  export MFM_PARITY_AAVE_V3_RUN_IDS_PATH="$CI_ARTIFACTS_DIR/parity-aave-v3-run-ids.json"

  LOGFILE=$(artifact_path "parity-postgres-state-events-audit.log")
  log_capture "$LOGFILE" -- cargo-nightly nextest run --cargo-profile ci -p mfm-integration-tests --features parity-tests --test parity_postgres_state_events_audit
''
