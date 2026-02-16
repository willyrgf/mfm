LOGFILE=$(artifact_path "build.log")
log_capture "$LOGFILE" -- cargo build --release --all-features
