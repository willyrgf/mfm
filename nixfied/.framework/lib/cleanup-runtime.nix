{ }:

{
  cleanupRuntime = ''
    _cleanup_initialized=false
    _cleanup_actions=()

    # with_cleanup CMD [arg...]
    # - register cleanup command + args to run on EXIT/INT/TERM (LIFO order).
    #   Commands are kept in memory as shell-quoted argv payloads.
    with_cleanup() {
      if [ "$#" -lt 1 ]; then
        echo "usage: with_cleanup <command> [arg...]" >&2
        return 1
      fi

      local action=""
      printf -v action '%q ' "$@"
      action="''${action% }"
      _cleanup_actions+=("$action")

      if [ "$_cleanup_initialized" = false ]; then
        _cleanup_initialized=true
        trap _run_cleanups EXIT INT TERM
      fi
    }

    _run_cleanups() {
      local i=$(( ''${#_cleanup_actions[@]} - 1 ))
      while [ $i -ge 0 ]; do
        (
          eval "''${_cleanup_actions[$i]}"
        ) || true
        i=$((i - 1))
      done
    }
  '';
}
