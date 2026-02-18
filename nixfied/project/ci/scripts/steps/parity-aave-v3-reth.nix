{ pkgs }:
pkgs.writeText "mfm-ci-steps-parity-aave-v3-reth.sh" ''
  source <($SLOT_INFO)

  export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

  export MFM_S3_ENDPOINT="$MINIO_ENDPOINT"
  export MFM_S3_REGION="$MINIO_REGION"
  export MFM_S3_BUCKET="$MINIO_BUCKET"
  export MFM_S3_PREFIX="$MINIO_PREFIX"

  export MFM_EVM_RPC_URL="http://127.0.0.1:$RETHHTTP_PORT"
  export MFM_PARITY_AAVE_V3_RUN_IDS_PATH="$CI_ARTIFACTS_DIR/parity-aave-v3-run-ids.json"

  run_hook SVC_MINIO_BUCKET_ENSURE "$MINIO_BUCKET"

  LOGFILE=$(artifact_path "parity-aave-v3-reth.log")
  log_capture "$LOGFILE" -- cargo nextest run -p mfm-integration-tests --features parity-tests --test parity_aave_v3_reth_scenario
''
