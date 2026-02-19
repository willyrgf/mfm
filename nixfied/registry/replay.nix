{ pkgs }:
{
  mkReplayTool =
    {
      name ? "nixfied-registry-replay",
    }:
    pkgs.writeShellScriptBin name ''
      set -euo pipefail

      root="''${REGISTRY_ROOT:-}"
      if [ -z "$root" ] && [ "$#" -gt 0 ]; then
        root="$1"
      fi
      if [ -z "$root" ]; then
        echo "ERROR: usage: ${name} <registry-root>"
        exit 2
      fi

      events_file="$root/events.ndjson"
      if [ ! -f "$events_file" ]; then
        echo "{}"
        exit 0
      fi

      ${pkgs.jq}/bin/jq -cs '
        reduce (sort_by(.seq)[]) as $event ({};
          .[(if ($event.taskId // "") != "" then "task:" + $event.taskId else "workflow:" + ($event.workflowId // "unknown") end)] = $event.state
        )
      ' "$events_file"
    '';
}
