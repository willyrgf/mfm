{ pkgs }:
pkgs.writeText "mfm-ci-steps-tests.sh" ''
  LOGFILE=$(artifact_path "tests.log")
  log_capture "$LOGFILE" -- cargo-nightly nextest run --workspace --cargo-profile ci
''
