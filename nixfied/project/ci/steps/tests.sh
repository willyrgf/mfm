LOGFILE=$(artifact_path "tests.log")
log_capture "$LOGFILE" -- cargo nextest run --workspace
