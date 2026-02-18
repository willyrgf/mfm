{ pkgs }:
pkgs.writeText "mfm-ci-steps-parity-s3.sh" ''
  export MFM_S3_ENDPOINT="$MINIO_ENDPOINT"
  export MFM_S3_REGION="$MINIO_REGION"
  export MFM_S3_BUCKET="$MINIO_BUCKET"
  export MFM_S3_PREFIX="$MINIO_PREFIX"

  run_hook SVC_MINIO_BUCKET_ENSURE "$MINIO_BUCKET"

  LOGFILE=$(artifact_path "parity-s3.log")
  log_capture "$LOGFILE" -- cargo nextest run -p mfm-integration-tests --features parity-tests --test parity_artifact_store_s3_contract
''
