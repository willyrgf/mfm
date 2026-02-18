{ pkgs }:
pkgs.writeText "mfm-ci-steps-architecture-verify.sh" ''
  LOGFILE=$(artifact_path "architecture-verify.log")
  log_capture "$LOGFILE" -- cargo run -p mfm-architecture-verify --
''
