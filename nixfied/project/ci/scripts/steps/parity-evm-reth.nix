{ pkgs }:
pkgs.writeText "mfm-ci-steps-parity-evm-reth.sh" ''
  source <($SLOT_INFO)

  export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

  export MFM_S3_ENDPOINT="$MINIO_ENDPOINT"
  export MFM_S3_REGION="$MINIO_REGION"
  export MFM_S3_BUCKET="$MINIO_BUCKET"
  export MFM_S3_PREFIX="$MINIO_PREFIX"

  export MFM_EVM_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"
  export MFM_PARITY_EVM_RETH_RUN_IDS_PATH="$CI_ARTIFACTS_DIR/parity-evm-reth-run-ids.json"

  run_hook SVC_MINIO_BUCKET_ENSURE "$MINIO_BUCKET"

  LOGFILE=$(artifact_path "parity-evm-reth.log")
  log_capture "$LOGFILE" -- cargo-nightly nextest run --cargo-profile ci -p mfm-integration-tests --features parity-tests --test parity_rest_api_evm_reth_pipeline --test parity_portfolio_tracker_reth_mock_erc20 --test parity_portfolio_tracker_reth_snapshot
''
