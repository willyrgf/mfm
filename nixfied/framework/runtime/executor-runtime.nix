{
  pkgs,
  model ? null,
}:
let
  lib = pkgs.lib;
  commonRuntimeShell = import ./common-runtime.nix { inherit pkgs; };
  skipPolicy = import ./helpers/skip-policy.nix { inherit pkgs; };
  workflows = if model == null then { } else model.workflows or { };
  workflowIds = builtins.sort builtins.lessThan (builtins.attrNames workflows);
  workflowUnitEntries = builtins.concatLists (
    map (
      workflowId:
      map (
        unit:
        let
          needs = unit.needs or [ ];
          locks = unit.locks or [ ];
          requirements = unit.requirements or { };
          produces = unit.produces or { };
          when = unit.when or { };
          whenEnvEquals = when.envEquals or { };
        in
        {
          key = "workflow-unit:${workflowId}:${unit.name}";
          value = {
            name = unit.name;
            taskId = unit.taskId or "";
            needsCount = toString (builtins.length needs);
            needs = needs;
            locks = if locks == [ ] then "" else "${lib.concatStringsSep " " locks} ";
            requiredServices = requirements.services or [ ];
            producesJson = builtins.toJSON {
              artifacts = produces.artifacts or [ ];
              stateKeys = produces.stateKeys or [ ];
            };
            skipIfMissingEnv = unit.skipIfMissingEnv or [ ];
            whenEnvPresent = when.envPresent or [ ];
            whenEnvEquals = map (name: "${name}\t${builtins.toString whenEnvEquals.${name}}") (
              builtins.sort builtins.lessThan (builtins.attrNames whenEnvEquals)
            );
          };
        }
      ) (workflows.${workflowId}.plan or [ ])
    ) workflowIds
  );
  renderCaseReturn =
    valueExpr: entries:
    lib.concatStringsSep "\n" (
      map (entry: ''
        ${lib.escapeShellArg entry.key})
          printf '%s' ${lib.escapeShellArg (valueExpr entry)}
          return 0
          ;;
      '') entries
    );
  renderCasePrintLines =
    valuesExpr: entries:
    lib.concatStringsSep "\n" (
      map (
        entry:
        let
          values = valuesExpr entry;
        in
        ''
          ${lib.escapeShellArg entry.key})
            ${
              if values == [ ] then
                ":"
              else
                "printf '%s\\n' " + lib.concatStringsSep " " (map lib.escapeShellArg values)
            }
            return 0
            ;;
        ''
      ) entries
    );
in
''
    ${commonRuntimeShell}
    ${skipPolicy.skipPolicyFunctions}

    normalize_run_artifacts_dir() {
      local base_dir="$1"
      local run_id="$2"
      local attempt_id="$3"

      if [ -z "$attempt_id" ]; then
        attempt_id="$run_id"
      fi

      case "$base_dir" in
        */"$attempt_id")
          printf '%s' "$base_dir"
          ;;
        */"$run_id")
          printf '%s/%s' "$base_dir" "$attempt_id"
          ;;
        *)
          printf '%s/%s/%s' "$base_dir" "$run_id" "$attempt_id"
          ;;
      esac
    }

    resolve_run_artifacts_dir() {
      local run_id="$1"
      local workflow_id="$2"
      local caller_root="''${CI_ARTIFACTS_ROOT:-}"
      local caller_dir="''${CI_ARTIFACTS_DIR:-}"
      local attempt_id="''${NIXFIED_ATTEMPT_ID:-''${NIXFIED_ORCHESTRATOR_ATTEMPT_ID:-}}"
      local configured_root=""
      local base_dir=""

      if [ -n "$caller_root" ] && [ -n "$caller_dir" ]; then
        echo "ERROR: CI_ARTIFACTS_ROOT and CI_ARTIFACTS_DIR cannot both be set"
        return 2
      fi

      if [ -n "$workflow_id" ]; then
        configured_root="$(workflow_artifacts_root "$workflow_id")"
      fi

      if [ -n "$caller_root" ]; then
        base_dir="$caller_root"
      elif [ -n "$caller_dir" ]; then
        base_dir="$caller_dir"
      elif [ -n "$configured_root" ]; then
        base_dir="$configured_root"
      elif [ "$REGISTRY_ROOT_EXPLICIT" = "1" ]; then
        base_dir="$REGISTRY_ROOT/artifacts"
      else
        base_dir="$ARTIFACTS_ROOT_DEFAULT"
      fi

      normalize_run_artifacts_dir "$base_dir" "$run_id" "$attempt_id"
    }

    ensure_run_artifacts_dir() {
      local run_id="$1"
      local workflow_id="$2"
      local managed_by_orchestrator="$3"
      local artifacts_dir

      if [ -n "''${CI_ARTIFACTS_DIR:-}" ] && {
        [ "$managed_by_orchestrator" = "1" ] ||
        [ "''${NIXFIED_WORKFLOW_NESTED:-0}" = "1" ] ||
        [ "''${NIXFIED_EXECUTION_EPHEMERAL:-0}" = "1" ]
      }; then
        mkdir -p "$CI_ARTIFACTS_DIR"
        return 0
      fi

      artifacts_dir="$(resolve_run_artifacts_dir "$run_id" "$workflow_id")" || return $?
      export CI_ARTIFACTS_DIR="$artifacts_dir"
      if ! mkdir -p "$CI_ARTIFACTS_DIR"; then
        echo "ERROR: failed to create artifacts directory '$CI_ARTIFACTS_DIR'"
        return 1
      fi
    }

    LOGGING_FILTERED_ARGS=()
    MACHINE_FILTERED_ARGS=()
    MACHINE_RUN_ID_FILE=""
    MACHINE_SUMMARY_FILE=""

    extract_logging_override_args() {
      local parse_options=1
      local arg=""
      local value=""
      local resolved_log_level="''${NIXFIED_CLI_LOG_LEVEL_OVERRIDE:-}"
      local resolved_output_mode="''${NIXFIED_CLI_OUTPUT_MODE_OVERRIDE:-}"

      LOGGING_FILTERED_ARGS=()

      while [ "$#" -gt 0 ]; do
        arg="$1"
        shift

        if [ "$parse_options" -eq 0 ]; then
          LOGGING_FILTERED_ARGS+=("$arg")
          continue
        fi

        case "$arg" in
          --)
            parse_options=0
            LOGGING_FILTERED_ARGS+=("--")
            ;;
          --log-level)
            if [ "$#" -lt 1 ]; then
              echo "ERROR: --log-level requires a value"
              return 2
            fi
            value="$1"
            shift
            if ! valid_log_level "$value"; then
              echo "ERROR: invalid --log-level '$value' (expected: error|warn|info|debug|trace)"
              return 2
            fi
            resolved_log_level="$value"
            ;;
          --log-level=*)
            value="''${arg#--log-level=}"
            if ! valid_log_level "$value"; then
              echo "ERROR: invalid --log-level '$value' (expected: error|warn|info|debug|trace)"
              return 2
            fi
            resolved_log_level="$value"
            ;;
          --output-mode)
            if [ "$#" -lt 1 ]; then
              echo "ERROR: --output-mode requires a value"
              return 2
            fi
            value="$1"
            shift
            if ! valid_output_mode "$value"; then
              echo "ERROR: invalid --output-mode '$value' (expected: stdout|logs|both)"
              return 2
            fi
            resolved_output_mode="$value"
            ;;
          --output-mode=*)
            value="''${arg#--output-mode=}"
            if ! valid_output_mode "$value"; then
              echo "ERROR: invalid --output-mode '$value' (expected: stdout|logs|both)"
              return 2
            fi
            resolved_output_mode="$value"
            ;;
          *)
            LOGGING_FILTERED_ARGS+=("$arg")
            ;;
        esac
      done

      if [ -n "$resolved_log_level" ]; then
        export NIXFIED_CLI_LOG_LEVEL_OVERRIDE="$resolved_log_level"
      else
        unset NIXFIED_CLI_LOG_LEVEL_OVERRIDE || true
      fi

      if [ -n "$resolved_output_mode" ]; then
        export NIXFIED_CLI_OUTPUT_MODE_OVERRIDE="$resolved_output_mode"
      else
        unset NIXFIED_CLI_OUTPUT_MODE_OVERRIDE || true
      fi
    }

    extract_machine_output_args() {
      local parse_options=1
      local arg=""
      local value=""

      MACHINE_FILTERED_ARGS=()
      MACHINE_RUN_ID_FILE="''${NIXFIED_RUN_ID_FILE_OVERRIDE:-}"
      MACHINE_SUMMARY_FILE="''${NIXFIED_SUMMARY_FILE_OVERRIDE:-}"

      while [ "$#" -gt 0 ]; do
        arg="$1"
        shift

        if [ "$parse_options" -eq 0 ]; then
          MACHINE_FILTERED_ARGS+=("$arg")
          continue
        fi

        case "$arg" in
          --)
            parse_options=0
            MACHINE_FILTERED_ARGS+=("--")
            ;;
          --run-id-file)
            if [ "$#" -lt 1 ]; then
              echo "ERROR: --run-id-file requires a value"
              return 2
            fi
            value="$1"
            shift
            if [ -z "$value" ]; then
              echo "ERROR: --run-id-file requires a non-empty value"
              return 2
            fi
            MACHINE_RUN_ID_FILE="$value"
            ;;
          --run-id-file=*)
            value="''${arg#--run-id-file=}"
            if [ -z "$value" ]; then
              echo "ERROR: --run-id-file requires a non-empty value"
              return 2
            fi
            MACHINE_RUN_ID_FILE="$value"
            ;;
          --summary-file)
            if [ "$#" -lt 1 ]; then
              echo "ERROR: --summary-file requires a value"
              return 2
            fi
            value="$1"
            shift
            if [ -z "$value" ]; then
              echo "ERROR: --summary-file requires a non-empty value"
              return 2
            fi
            MACHINE_SUMMARY_FILE="$value"
            ;;
          --summary-file=*)
            value="''${arg#--summary-file=}"
            if [ -z "$value" ]; then
              echo "ERROR: --summary-file requires a non-empty value"
              return 2
            fi
            MACHINE_SUMMARY_FILE="$value"
            ;;
          *)
            MACHINE_FILTERED_ARGS+=("$arg")
            ;;
        esac
      done
    }

    copy_file_atomic() {
      local source_file="$1"
      local target_file="$2"
      local parent_dir
      local tmp

      if [ -z "$target_file" ]; then
        return 0
      fi

      parent_dir="$(dirname "$target_file")" || {
        echo "ERROR: unable to determine parent directory for '$target_file'" >&2
        return 1
      }
      mkdir -p "$parent_dir" || {
        echo "ERROR: unable to create directory '$parent_dir'" >&2
        return 1
      }
      tmp="$(mktemp "$target_file.tmp.XXXXXX")" || {
        echo "ERROR: unable to create temp file for '$target_file'" >&2
        return 1
      }
      if ! ${pkgs.coreutils}/bin/cp "$source_file" "$tmp"; then
        rm -f "$tmp"
        echo "ERROR: failed to copy '$source_file' to temp file for '$target_file'" >&2
        return 1
      fi
      if ! mv "$tmp" "$target_file"; then
        rm -f "$tmp"
        echo "ERROR: failed to move temp file into '$target_file'" >&2
        return 1
      fi
    }

    workflow_unit_name() {
      local unit_json="$1"
      case "$unit_json" in
  ${renderCaseReturn (entry: entry.value.name) workflowUnitEntries}
        *)
          return 1
          ;;
      esac
    }

    workflow_unit_task_id() {
      local unit_json="$1"
      case "$unit_json" in
  ${renderCaseReturn (entry: entry.value.taskId) workflowUnitEntries}
        *)
          return 1
          ;;
      esac
    }

    workflow_unit_required_services() {
      local unit_json="$1"
      case "$unit_json" in
  ${renderCasePrintLines (entry: entry.value.requiredServices) workflowUnitEntries}
        *)
          return 0
          ;;
      esac
    }

    workflow_unit_needs_count() {
      local unit_json="$1"
      case "$unit_json" in
  ${renderCaseReturn (entry: entry.value.needsCount) workflowUnitEntries}
        *)
          return 1
          ;;
      esac
    }

    workflow_unit_dependencies() {
      local unit_json="$1"
      case "$unit_json" in
  ${renderCasePrintLines (entry: entry.value.needs) workflowUnitEntries}
        *)
          return 0
          ;;
      esac
    }

    workflow_unit_lock_list() {
      local unit_json="$1"
      case "$unit_json" in
  ${renderCaseReturn (entry: entry.value.locks) workflowUnitEntries}
        *)
          return 0
          ;;
      esac
    }

    workflow_unit_produces_json() {
      local unit_json="$1"
      case "$unit_json" in
  ${renderCaseReturn (entry: entry.value.producesJson) workflowUnitEntries}
        *)
          return 1
          ;;
      esac
    }

    workflow_unit_missing_env_csv() {
      local unit_json="$1"
      local missing=""
      local required_env

      while IFS= read -r required_env; do
        if [ -n "$required_env" ] && [ -z "''${!required_env:-}" ]; then
          if [ -z "$missing" ]; then
            missing="$required_env"
          else
            missing="$missing,$required_env"
          fi
        fi
      done < <(
        case "$unit_json" in
  ${renderCasePrintLines (entry: entry.value.skipIfMissingEnv) workflowUnitEntries}
          *)
            :
            ;;
        esac
      )

      printf '%s' "$missing"
    }

    workflow_unit_when_matches() {
      local unit_json="$1"
      local required_env
      local env_name
      local expected_value
      local actual_value

      while IFS= read -r required_env; do
        if [ -n "$required_env" ] && [ -z "''${!required_env:-}" ]; then
          return 1
        fi
      done < <(
        case "$unit_json" in
  ${renderCasePrintLines (entry: entry.value.whenEnvPresent) workflowUnitEntries}
          *)
            :
            ;;
        esac
      )

      while IFS=$'\t' read -r env_name expected_value; do
        if [ -z "$env_name" ]; then
          continue
        fi
        actual_value="''${!env_name:-}"
        if [ "$actual_value" != "$expected_value" ]; then
          return 1
        fi
      done < <(
        case "$unit_json" in
  ${renderCasePrintLines (entry: entry.value.whenEnvEquals) workflowUnitEntries}
          *)
            :
            ;;
        esac
      )

      return 0
    }
''
