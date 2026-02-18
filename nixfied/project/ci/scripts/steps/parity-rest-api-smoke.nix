{ pkgs }:
pkgs.writeText "mfm-ci-steps-parity-rest-api-smoke.sh" ''
  eval "$($SLOT_INFO)"

  export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/mfm_test"

  export MFM_S3_ENDPOINT="$MINIO_ENDPOINT"
  export MFM_S3_REGION="$MINIO_REGION"
  export MFM_S3_BUCKET="$MINIO_BUCKET"
  export MFM_S3_PREFIX="$MINIO_PREFIX"

  LOGFILE=$(artifact_path "parity-rest-api-smoke.log")
  log_capture "$LOGFILE" -- cargo nextest run -p mfm-integration-tests --features parity-tests --test parity_rest_api_postgres_s3_smoke
''
