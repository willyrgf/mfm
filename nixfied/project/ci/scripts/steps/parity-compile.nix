{ pkgs }:
pkgs.writeText "mfm-ci-steps-parity-compile.sh" ''
  LOGFILE=$(artifact_path "parity-compile.log")
  log_capture "$LOGFILE" -- cargo-nightly nextest run --no-run --cargo-profile ci -p mfm-integration-tests --features parity-tests -p mfm --features parity-tests
''
