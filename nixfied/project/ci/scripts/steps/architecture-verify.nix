{ pkgs }:
pkgs.writeText "mfm-ci-steps-architecture-verify.sh" ''
  LOGFILE=$(artifact_path "architecture-verify.log")
  log_capture "$LOGFILE" -- cargo-nightly run --profile ci -p mfm-architecture-verify --
''
