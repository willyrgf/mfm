# Parallel execution utility
{
  pkgs,
  loggingPrelude,
}:

let
  mkParallelRunner =
    commands:
    pkgs.writeShellScript "parallel-runner" ''
      ${loggingPrelude}

      set -euo pipefail
      declare -a OUTPUT_FILES
      declare -a PIDS

      i=0
      ${pkgs.lib.concatMapStringsSep "\n" (cmd: ''
        OUTPUT_FILE=$(mktemp 2>/dev/null || true)
        if [ -z "$OUTPUT_FILE" ]; then
          log_error "failed to allocate temporary output file for command $((i+1))"
          exit 1
        fi
        OUTPUT_FILES[$i]=$OUTPUT_FILE

        log_run "command $((i+1)): ${cmd}"
        (
          eval "${cmd}" 2>&1
          echo $? > "$OUTPUT_FILE.exit"
        ) > "$OUTPUT_FILE" 2>&1 &

        PIDS[$i]=$!
        i=$((i+1))
      '') commands}

      i=0
      EXIT_CODE=0
      for pid in "''${PIDS[@]}"; do
        wait $pid
        CMD_EXIT=$(cat "''${OUTPUT_FILES[$i]}.exit" 2>/dev/null || echo "1")
        if [ "$CMD_EXIT" -ne 0 ]; then
          EXIT_CODE=$CMD_EXIT
        fi
        i=$((i+1))
      done

      i=0
      for cmd in ${pkgs.lib.concatMapStringsSep " " (c: "\"${c}\"") commands}; do
        echo ""
        log_output "command $((i+1))"
        cat "''${OUTPUT_FILES[$i]}"
        i=$((i+1))
      done

      i=0
      for output_file in "''${OUTPUT_FILES[@]}"; do
        rm -f "$output_file" "$output_file.exit"
      done

      exit $EXIT_CODE
    '';
in
{
  inherit mkParallelRunner;
}
