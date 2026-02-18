{ pkgs }:
pkgs.writeText "mfm-ci-steps-build.sh" ''
  LOGFILE=$(artifact_path "build.log")
  log_capture "$LOGFILE" -- cargo-nightly build --release --all-features
''
