{
  pkgs,
  appId,
  app,
  contractBundle,
  targetProgram,
  setupPrograms ? [ ],
  teardownPrograms ? [ ],
}:
let
  lib = pkgs.lib;
  mkShellApp = import ./mk-shell-app.nix { inherit pkgs; };
  commonRuntimeShell = import ../runtime/common-runtime.nix { inherit pkgs; };
  contractRef = ((app.validation or { }).contractRef or "");
  validatorProgram = import ../contracts/mkValidator.nix {
    inherit
      pkgs
      contractBundle
      contractRef
      ;
  };
  targetArgsLiteral = builtins.concatStringsSep " " (map lib.escapeShellArg (app.targetArgs or [ ]));
  setupProgramsLiteral = builtins.concatStringsSep " " (map lib.escapeShellArg setupPrograms);
  teardownProgramsLiteral = builtins.concatStringsSep " " (map lib.escapeShellArg teardownPrograms);
in
(mkShellApp {
  appName = "machine-output:${appId}";
  binPrefix = "nixfied-machine-output";
  body = ''
    ${commonRuntimeShell}
    target_program=${lib.escapeShellArg targetProgram}
    target_app_id=${lib.escapeShellArg (app.targetAppId or "")}
    contract_ref=${lib.escapeShellArg contractRef}
    validator_program=${lib.escapeShellArg validatorProgram}
    setup_programs=( ${setupProgramsLiteral} )
    teardown_programs=( ${teardownProgramsLiteral} )
    target_args=( ${targetArgsLiteral} )

    work_dir="$(mktemp -d "''${TMPDIR:-/tmp}/nixfied-machine-output.XXXXXX")"
    cleanup_machine_output() {
      rm -rf "$work_dir"
    }
    trap cleanup_machine_output EXIT

    render_captured_stream() {
      local level="$1"
      local label="$2"
      local stream="$3"
      local path="$4"

      if [ -s "$path" ]; then
        echo "$level: $label $stream:" >&2
        cat "$path" >&2
      fi
    }

    render_captured_logs() {
      local level="$1"
      local label="$2"
      local stdout_file="$3"
      local stderr_file="$4"

      render_captured_stream "$level" "$label" "stdout" "$stdout_file"
      render_captured_stream "$level" "$label" "stderr" "$stderr_file"
    }

    emit_failure_json() {
      local stage="$1"
      local code="$2"
      local message="$3"
      local failed_app_id="$4"
      local exit_code="$5"
      printf '{'
      printf '"ok":false'
      printf ',"appId":%s' "$(json_quote_string ${lib.escapeShellArg appId})"
      printf ',"targetAppId":%s' "$(json_quote_string "$target_app_id")"
      printf ',"stage":%s' "$(json_quote_string "$stage")"
      printf ',"code":%s' "$(json_quote_string "$code")"
      printf ',"message":%s' "$(json_quote_string "$message")"
      printf ',"failedAppId":%s' "$(json_string_or_null "$failed_app_id")"
      printf ',"contractRef":%s' "$(json_string_or_null "$contract_ref")"
      if [ "$stage" = "validation" ]; then
        printf ',"validator":"cue"'
      else
        printf ',"validator":null'
      fi
      printf ',"exitCode":%s' "$exit_code"
      printf '}\n'
    }

    run_captured_app() {
      local program="$1"
      local stdout_file="$2"
      local stderr_file="$3"
      shift 3
      "$program" "$@" >"$stdout_file" 2>"$stderr_file"
    }

    run_machine_output_target() {
      local program="$1"
      local payload_file="$2"
      local stdout_file="$3"
      local stderr_file="$4"
      shift 4
      ${pkgs.coreutils}/bin/env \
        NIXFIED_MACHINE_OUTPUT_FILE="$payload_file" \
        "$program" "$@" >"$stdout_file" 2>"$stderr_file"
    }

    setup_index=0
    for setup_program in "''${setup_programs[@]}"; do
      setup_index="$((setup_index + 1))"
      setup_stdout="$work_dir/setup-$setup_index.stdout"
      setup_stderr="$work_dir/setup-$setup_index.stderr"
      if run_captured_app "$setup_program" "$setup_stdout" "$setup_stderr"; then
        rc=0
        render_captured_logs "INFO" "setup app $setup_index" "$setup_stdout" "$setup_stderr"
      else
        rc="$?"
        render_captured_logs "ERROR" "setup app $setup_index" "$setup_stdout" "$setup_stderr"
        emit_failure_json "setup" "machine-output-setup-failed" "setup app $setup_index failed" "" "$rc"
        exit "$rc"
      fi
    done

    payload_file="$work_dir/payload.json"
    target_stdout="$work_dir/target.stdout"
    target_stderr="$work_dir/target.stderr"
    if run_machine_output_target "$target_program" "$payload_file" "$target_stdout" "$target_stderr" "''${target_args[@]}" "$@"; then
      rc=0
      render_captured_logs "INFO" "target app" "$target_stdout" "$target_stderr"
    else
      rc="$?"
      render_captured_logs "ERROR" "target app" "$target_stdout" "$target_stderr"
      emit_failure_json "target" "machine-output-target-failed" "target app '$target_app_id' failed" "$target_app_id" "$rc"
      exit "$rc"
    fi

    if [ ! -s "$payload_file" ]; then
      emit_failure_json "validation" "machine-output-validation-failed" "target app '$target_app_id' did not write machine payload to declared file" "$target_app_id" 1
      exit 1
    fi

    if "$validator_program" "$payload_file" >"$work_dir/validate.stdout" 2>"$work_dir/validate.stderr"; then
      :
    else
      render_captured_logs "ERROR" "validation" "$work_dir/validate.stdout" "$work_dir/validate.stderr"
      emit_failure_json "validation" "machine-output-validation-failed" "target app '$target_app_id' did not satisfy contract '$contract_ref'" "$target_app_id" 1
      exit 1
    fi

    teardown_index=0
    for teardown_program in "''${teardown_programs[@]}"; do
      teardown_index="$((teardown_index + 1))"
      teardown_stdout="$work_dir/teardown-$teardown_index.stdout"
      teardown_stderr="$work_dir/teardown-$teardown_index.stderr"
      if run_captured_app "$teardown_program" "$teardown_stdout" "$teardown_stderr"; then
        rc=0
        render_captured_logs "INFO" "teardown app $teardown_index" "$teardown_stdout" "$teardown_stderr"
      else
        rc="$?"
        render_captured_logs "ERROR" "teardown app $teardown_index" "$teardown_stdout" "$teardown_stderr"
        emit_failure_json "teardown" "machine-output-teardown-failed" "teardown app $teardown_index failed" "" "$rc"
        exit "$rc"
      fi
    done

    cat "$payload_file"
  '';
}).program
