{ pkgs }:
let
  commonRuntimeShell = import ./common-runtime.nix { inherit pkgs; };
in
''
  ${commonRuntimeShell}

  normalize_run_artifacts_dir() {
    local base_dir="$1"
    local run_id="$2"

    case "$base_dir" in
      */"$run_id")
        printf '%s' "$base_dir"
        ;;
      *)
        printf '%s/%s' "$base_dir" "$run_id"
        ;;
    esac
  }

  resolve_run_artifacts_dir() {
    local run_id="$1"
    local workflow_id="$2"
    local caller_root="''${CI_ARTIFACTS_ROOT:-}"
    local caller_dir="''${CI_ARTIFACTS_DIR:-}"
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

    normalize_run_artifacts_dir "$base_dir" "$run_id"
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

  emit_workflow_result_json() {
    local run_id="$1"
    local workflow_id="$2"
    local summary_file="$3"
    local exit_code="$4"

    if [ -n "$summary_file" ] && [ -f "$summary_file" ]; then
      ${pkgs.jq}/bin/jq -cnS \
        --arg runId "$run_id" \
        --arg workflowId "$workflow_id" \
        --arg summaryJson "$summary_file" \
        --argjson exitCode "$exit_code" \
        --slurpfile summary "$summary_file" \
        '{
          run_id: $runId,
          workflow_id: $workflowId,
          exit_code: $exitCode,
          summary_json: $summaryJson,
          summary: ($summary[0] // null)
        }'
      return 0
    fi

    ${pkgs.jq}/bin/jq -cnS \
      --arg runId "$run_id" \
      --arg workflowId "$workflow_id" \
      --argjson exitCode "$exit_code" \
      '{
        run_id: $runId,
        workflow_id: $workflowId,
        exit_code: $exitCode,
        summary_json: null,
        summary: null
      }'
  }

  LOGGING_FILTERED_ARGS=()
  MACHINE_FILTERED_ARGS=()
  MACHINE_JSON=0
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
    MACHINE_JSON=0
    if [ "''${NIXFIED_JSON_OUTPUT_OVERRIDE:-0}" = "1" ]; then
      MACHINE_JSON=1
    fi
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
        --json)
          MACHINE_JSON=1
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

    parent_dir="$(dirname "$target_file")"
    mkdir -p "$parent_dir"
    tmp="$(mktemp "$target_file.tmp.XXXXXX")"
    ${pkgs.coreutils}/bin/cp "$source_file" "$tmp"
    mv "$tmp" "$target_file"
  }

  workflow_unit_name() {
    local unit_json="$1"
    printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '.name'
  }

  workflow_unit_task_id() {
    local unit_json="$1"
    printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '.taskId'
  }

  workflow_unit_service_name() {
    local unit_json="$1"
    printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '(.serviceName // "")'
  }

  workflow_unit_needs_count() {
    local unit_json="$1"
    printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '(.needs // []) | length'
  }

  workflow_unit_dependencies() {
    local unit_json="$1"
    printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '.needs[]?'
  }

  workflow_unit_lock_list() {
    local unit_json="$1"
    printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '.locks[]?' | ${pkgs.gawk}/bin/awk 'NF {printf "%s ", $0}'
  }

  workflow_unit_produces_json() {
    local unit_json="$1"
    printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -c '.produces // {artifacts: [], stateKeys: []}'
  }

  workflow_service_skip_env_var() {
    local service_name="$1"
    local safe_service_name
    if [ -z "$service_name" ]; then
      printf '%s' ""
      return 0
    fi

    safe_service_name="$(printf '%s' "$service_name" | ${pkgs.coreutils}/bin/tr '[:lower:]' '[:upper:]' | ${pkgs.coreutils}/bin/tr -cs 'A-Z0-9_' '_')"
    if [ -z "$safe_service_name" ]; then
      printf '%s' ""
      return 0
    fi
    printf 'SKIP_%s' "$safe_service_name"
  }

  is_truthy_skip_value() {
    local raw_value="$1"
    local normalized_value

    normalized_value="$(printf '%s' "$raw_value" | ${pkgs.coreutils}/bin/tr '[:upper:]' '[:lower:]' | ${pkgs.coreutils}/bin/tr -d '[:space:]')"
    case "$normalized_value" in
      1|true|yes|on)
        return 0
        ;;
      *)
        return 1
        ;;
    esac
  }

  is_service_skipped() {
    local service_name="$1"
    local env_name
    local env_value

    env_name="$(workflow_service_skip_env_var "$service_name")"
    if [ -z "$env_name" ]; then
      return 1
    fi

    env_value="''${!env_name:-}"
    is_truthy_skip_value "$env_value" && return 0
    return 1
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
    done < <(printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '.skipIfMissingEnv[]?')

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
    done < <(printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '.when.envPresent[]?')

    while IFS=$'\t' read -r env_name expected_value; do
      if [ -z "$env_name" ]; then
        continue
      fi
      actual_value="''${!env_name:-}"
      if [ "$actual_value" != "$expected_value" ]; then
        return 1
      fi
    done < <(printf '%s' "$unit_json" | ${pkgs.jq}/bin/jq -r '.when.envEquals // {} | to_entries[]? | [.key, (.value | tostring)] | @tsv')

    return 0
  }
''
