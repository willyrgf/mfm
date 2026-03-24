{ pkgs }:
let
  events = import ./events.nix { inherit pkgs; };
  shellCommon = import ../../core/shell-common.nix { inherit pkgs; };
  commonRuntimeShell = import ../common-runtime.nix { inherit pkgs; };
in
{
  mkReplayTool =
    {
      name ? "nixfied-registry-replay",
    }:
    pkgs.writeShellScriptBin name ''
      set -euo pipefail

      ${events.mkShellLib { }}
      ${shellCommon}
      ${commonRuntimeShell}

      root="''${REGISTRY_ROOT:-}"
      if [ -z "$root" ] && [ "$#" -gt 0 ]; then
        root="$1"
      fi
      if [ -z "$root" ]; then
        nixfied_exit_usage "usage: ${name} <registry-root>"
      fi

      events_index_file="$(registry_events_index_snapshot "$root")"
      if [ -z "$events_index_file" ] || [ ! -f "$events_index_file" ]; then
        echo "{}"
        exit 0
      fi

      declare -A replay_state=()
      replay_key=""
      seq=""
      ts_epoch=""
      ts=""
      run_id=""
      attempt_id=""
      workflow_id=""
      task_id=""
      state=""
      reason=""
      exit_code=""

      while IFS=$'\t' read -r seq ts_epoch ts run_id attempt_id workflow_id task_id state reason exit_code; do
        if [ -n "$task_id" ]; then
          replay_key="task:$task_id"
        elif [ -n "$workflow_id" ]; then
          replay_key="workflow:$workflow_id"
        else
          continue
        fi
        replay_state["$replay_key"]="$state"
      done < "$events_index_file"

      printf '{'
      first=1
      while IFS= read -r replay_key; do
        [ -n "$replay_key" ] || continue
        if [ "$first" -eq 0 ]; then
          printf ','
        fi
        printf '%s:%s' "$(json_quote_string "$replay_key")" "$(json_quote_string "''${replay_state[$replay_key]}")"
        first=0
      done < <(printf '%s\n' "''${!replay_state[@]}" | ${pkgs.coreutils}/bin/sort)
      printf '}\n'
      registry_snapshot_cleanup "$events_index_file"
    '';
}
