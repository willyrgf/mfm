LOGFILE=$(artifact_path "audit.log")
log_capture "$LOGFILE" -- cargo audit
