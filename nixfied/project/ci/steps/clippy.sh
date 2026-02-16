LOGFILE=$(artifact_path "clippy.log")
log_capture "$LOGFILE" -- cargo-nightly clippy --workspace --lib --examples --tests --benches --all-features
