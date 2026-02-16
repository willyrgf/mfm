LOGFILE=$(artifact_path "architecture-verify.log")
log_capture "$LOGFILE" -- cargo run -p mfm-architecture-verify --
