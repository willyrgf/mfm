# Process management utilities - signal handlers, process managers
{ pkgs }:

let
  mkSignalHandler =
    cleanupHook:
    pkgs.writeShellScript "signal-handler" ''
      shutdown() {
        echo ""
        echo "INFO: Shutting down"
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
      ${mkSignalHandler cleanupHook}

      echo "INFO: Starting ${processName}"
      ${startupScript}
      wait
    '';
in
{
  inherit mkSignalHandler mkProcessManager;
}
