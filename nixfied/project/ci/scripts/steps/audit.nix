{ pkgs }:
pkgs.writeText "mfm-ci-steps-audit.sh" ''
  LOGFILE=$(artifact_path "audit.log")
  log_capture "$LOGFILE" -- cargo audit
''
