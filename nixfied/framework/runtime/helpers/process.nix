# Process management utilities - signal handlers, process managers
{
  pkgs,
  loggingPrelude,
}:

let
  mkSignalHandler =
    cleanupHook:
    pkgs.writeShellScript "signal-handler" ''
      ${loggingPrelude}

      shutdown() {
        echo ""
        log_info "Shutting down"
        ${cleanupHook}
        exit 0
      }

      trap shutdown SIGINT SIGTERM

      cleanup() {
        shutdown "$@"
      }
    '';

  mkProcessManager =
    {
      processName ? "",
      startupScript,
      cleanupHook ? "",
    }:
    pkgs.writeShellScript "process-manager" ''
      ${loggingPrelude}

      ${mkSignalHandler cleanupHook}

      log_info "Starting ${processName}"
      ${startupScript}
      wait
    '';
in
{
  inherit mkSignalHandler mkProcessManager;
}
