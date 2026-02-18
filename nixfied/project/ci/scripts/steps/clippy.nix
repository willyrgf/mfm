{ pkgs }:
pkgs.writeText "mfm-ci-steps-clippy.sh" ''
  LOGFILE=$(artifact_path "clippy.log")
  log_capture "$LOGFILE" -- cargo-nightly clippy --workspace --lib --examples --tests --benches --all-features
''
