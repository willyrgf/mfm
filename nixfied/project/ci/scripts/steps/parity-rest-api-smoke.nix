{ pkgs }:
pkgs.writeText "mfm-ci-steps-parity-rest-api-smoke.sh" ''
  source <($SLOT_INFO)

  export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

  export MFM_S3_ENDPOINT="$MINIO_ENDPOINT"
  export MFM_S3_REGION="$MINIO_REGION"
  export MFM_S3_BUCKET="$MINIO_BUCKET"
  export MFM_S3_PREFIX="$MINIO_PREFIX"

  run_hook SVC_MINIO_BUCKET_ENSURE "$MINIO_BUCKET"

  LOGFILE=$(artifact_path "parity-rest-api-smoke.log")
  log_capture "$LOGFILE" -- cargo-nightly nextest run --cargo-profile ci --jobs 1 -p mfm-integration-tests --features parity-tests --test parity_event_store_postgres_contract --test parity_artifact_store_s3_contract --test parity_rest_api_postgres_s3_smoke
''
