LOGFILE=$(artifact_path "fmt.log")
log_capture "$LOGFILE" -- cargo-nightly fmt --all -- --check
