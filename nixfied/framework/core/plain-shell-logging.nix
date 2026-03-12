{
  includeInfo ? true,
  includeWarn ? true,
  includeError ? true,
  includeOk ? true,
  includeSkip ? true,
  includeStop ? false,
  errorToStderr ? false,
  warnToStderr ? false,
  stopToStderr ? false,
  stopFormat ? "STOP: %s\n",
}:
let
  render = name: format: toStderr: ''
    ${name}() {
      printf ${builtins.toJSON format} "$*"${if toStderr then " >&2" else ""}
    }
  '';
in
builtins.concatStringsSep "\n\n" (
  builtins.filter (part: part != "") [
    (if includeInfo then render "log_info" "INFO: %s\n" false else "")
    (if includeWarn then render "log_warn" "WARN: %s\n" warnToStderr else "")
    (if includeError then render "log_error" "ERROR: %s\n" errorToStderr else "")
    (if includeOk then render "log_ok" "OK: %s\n" false else "")
    (if includeSkip then render "log_skip" "SKIP: %s\n" false else "")
    (if includeStop then render "log_stop" stopFormat stopToStderr else "")
  ]
)
