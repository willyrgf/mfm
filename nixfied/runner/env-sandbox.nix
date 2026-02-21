{
  pkgs,
  projectRoot,
  model,
}:
''
  RUNTIME_JSON=${pkgs.lib.escapeShellArg (builtins.toJSON model.runtime)}
  SERVICES_JSON=${pkgs.lib.escapeShellArg (builtins.toJSON model.services)}
  PROJECT_NAME=${pkgs.lib.escapeShellArg model.identity.projectName}
  PROJECT_DESCRIPTION=${pkgs.lib.escapeShellArg model.identity.description}

  normalize_env_token() {
    printf '%s' "$1" | ${pkgs.coreutils}/bin/tr '[:lower:].-' '[:upper:]__' | ${pkgs.coreutils}/bin/tr -c 'A-Z0-9_' '_'
  }

  valid_log_level() {
    local value="$1"
    case "$value" in
      error|warn|info|debug|trace)
        return 0
        ;;
      *)
        return 1
        ;;
    esac
  }

  valid_output_mode() {
    local value="$1"
    case "$value" in
      stdout|logs|both)
        return 0
        ;;
      *)
        return 1
        ;;
    esac
  }

  is_sensitive_env_name() {
    local env_name="$1"
    local upper_name
    upper_name="$(printf '%s' "$env_name" | ${pkgs.coreutils}/bin/tr '[:lower:]' '[:upper:]')"

    case "$upper_name" in
      API_KEY|AWS_ACCESS_KEY_ID|AWS_SECRET_ACCESS_KEY|*_TOKEN|*_SECRET|*_PASSWORD|*_PRIVATE_KEY)
        return 0
        ;;
      *)
        return 1
        ;;
    esac
  }

  run_in_sandbox_runtime() {
    local runtime_json="$1"
    shift
    local command="$1"
    shift

    local workdir_kind
    local custom_workdir
    local workdir
    local locale
    local timezone
    local umask_value
    local runtime_path=""
    local base_path
    local final_path
    local host_developer_dir=""
    local host_sdkroot=""

    local slot_var
    local env_var
    local slot_default
    local env_default
    local slot_stride
    local slot_value
    local env_value
    local env_offset
    local runtime_dir_base
    local log_level_default
    local output_mode_default
    local task_log_level_default
    local task_output_mode_default
    local workflow_log_level_default
    local workflow_output_mode_default
    local surface_log_level_default
    local surface_output_mode_default
    local runtime_log_level_set
    local runtime_log_level_alias_set
    local runtime_log_level_value
    local runtime_log_level_alias_value
    local runtime_output_mode_set
    local runtime_output_mode_alias_set
    local runtime_output_mode_value
    local runtime_output_mode_alias_value
    local runtime_log_level_override=""
    local runtime_output_mode_override=""
    local invocation_log_level_set=0
    local invocation_log_level_alias_set=0
    local invocation_log_level_override=""
    local invocation_output_mode_set=0
    local invocation_output_mode_alias_set=0
    local invocation_output_mode_override=""
    local cli_log_level_override
    local cli_output_mode_override
    local resolved_log_level
    local resolved_output_mode
    local runtime_log_file_set
    local runtime_log_file_value
    local resolved_log_file=""
    local logs_root
    local logs_dir
    local log_file_run_token
    local allow_sensitive_pass_through

    workdir_kind="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '.workdir')"
    custom_workdir="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '.customWorkdir // empty')"

    case "$workdir_kind" in
      projectRoot)
        workdir="''${NIXFIED_CALLER_PWD:-${builtins.toString projectRoot}}"
        ;;
      stateRoot)
        workdir="$REGISTRY_ROOT"
        ;;
      custom)
        if [ -z "$custom_workdir" ]; then
          echo "ERROR: task runtime.workdir=custom but customWorkdir is empty"
          return 3
        fi
        workdir="$custom_workdir"
        ;;
      *)
        echo "ERROR: unknown runtime.workdir '$workdir_kind'"
        return 3
        ;;
    esac

    while IFS= read -r runtime_input; do
      if [ -n "$runtime_input" ]; then
        if [ -z "$runtime_path" ]; then
          runtime_path="$runtime_input/bin"
        else
          runtime_path="$runtime_path:$runtime_input/bin"
        fi
      fi
    done < <(printf '%s' "$RUNTIME_JSON" | ${pkgs.jq}/bin/jq -r '.runtimePackages[]?')

    while IFS= read -r runtime_input; do
      if [ -n "$runtime_input" ]; then
        if [ -z "$runtime_path" ]; then
          runtime_path="$runtime_input/bin"
        else
          runtime_path="$runtime_path:$runtime_input/bin"
        fi
      fi
    done < <(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '.runtimeInputs[]?')

    base_path="${pkgs.coreutils}/bin:${pkgs.findutils}/bin:${pkgs.gnused}/bin:${pkgs.gnugrep}/bin:${pkgs.jq}/bin:${pkgs.bash}/bin"
    if [ -n "$runtime_path" ]; then
      final_path="$runtime_path:$base_path"
    else
      final_path="$base_path"
    fi

    locale="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '.locale // "C.UTF-8"')"
    timezone="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '.timezone // "UTC"')"
    umask_value="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '.umask // "022"')"
    allow_sensitive_pass_through="$(
      printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r 'if (.allowSensitivePassThrough // false) then "1" else "0" end'
    )"

    slot_var="$(printf '%s' "$RUNTIME_JSON" | ${pkgs.jq}/bin/jq -r '.slot.var')"
    env_var="$(printf '%s' "$RUNTIME_JSON" | ${pkgs.jq}/bin/jq -r '.env.var')"
    slot_default="$(printf '%s' "$RUNTIME_JSON" | ${pkgs.jq}/bin/jq -r '.slot.default')"
    env_default="$(printf '%s' "$RUNTIME_JSON" | ${pkgs.jq}/bin/jq -r '.env.default')"
    slot_stride="$(printf '%s' "$RUNTIME_JSON" | ${pkgs.jq}/bin/jq -r '.slot.stride')"

    slot_value="''${!slot_var:-$slot_default}"
    env_value="''${!env_var:-$env_default}"

    if ! [[ "$slot_value" =~ ^[0-9]+$ ]]; then
      echo "ERROR: $slot_var must be an integer"
      return 3
    fi

    env_offset="$(printf '%s' "$RUNTIME_JSON" | ${pkgs.jq}/bin/jq -r --arg env "$env_value" '.env.offsets[$env] // empty')"
    if [ -z "$env_offset" ]; then
      echo "ERROR: unsupported $env_var '$env_value'"
      return 3
    fi

    runtime_dir_base="$(printf '%s' "$RUNTIME_JSON" | ${pkgs.jq}/bin/jq -r '.directories.base // empty')"
    log_level_default="$(printf '%s' "$RUNTIME_JSON" | ${pkgs.jq}/bin/jq -r '.logging.levelDefault // "info"')"
    output_mode_default="$(printf '%s' "$RUNTIME_JSON" | ${pkgs.jq}/bin/jq -r '.logging.outputDefault // "stdout"')"

    local home_value
    home_value="''${HOME:-$workdir}"

    if ! valid_log_level "$log_level_default"; then
      echo "ERROR: invalid runtime default LOG_LEVEL value='$log_level_default' (expected: error|warn|info|debug|trace)"
      return 2
    fi
    if ! valid_output_mode "$output_mode_default"; then
      echo "ERROR: invalid runtime default OUTPUT_MODE value='$output_mode_default' (expected: stdout|logs|both)"
      return 2
    fi

    task_log_level_default="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '.logging.levelDefault // empty')"
    task_output_mode_default="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '.logging.outputDefault // empty')"
    workflow_log_level_default=""
    workflow_output_mode_default=""
    if [ "''${NIXFIED_WORKFLOW_CONTEXT:-0}" = "1" ]; then
      workflow_log_level_default="''${NIXFIED_WORKFLOW_LOG_LEVEL_DEFAULT:-}"
      workflow_output_mode_default="''${NIXFIED_WORKFLOW_OUTPUT_MODE_DEFAULT:-}"
    fi

    surface_log_level_default="$task_log_level_default"
    if [ -z "$surface_log_level_default" ] && [ -n "$workflow_log_level_default" ]; then
      surface_log_level_default="$workflow_log_level_default"
    fi
    if [ -z "$surface_log_level_default" ]; then
      surface_log_level_default="$log_level_default"
    fi

    surface_output_mode_default="$task_output_mode_default"
    if [ -z "$surface_output_mode_default" ] && [ -n "$workflow_output_mode_default" ]; then
      surface_output_mode_default="$workflow_output_mode_default"
    fi
    if [ -z "$surface_output_mode_default" ]; then
      surface_output_mode_default="$output_mode_default"
    fi

    if ! valid_log_level "$surface_log_level_default"; then
      echo "ERROR: invalid logging.levelDefault value='$surface_log_level_default' (expected: error|warn|info|debug|trace)"
      return 2
    fi
    if ! valid_output_mode "$surface_output_mode_default"; then
      echo "ERROR: invalid logging.outputDefault value='$surface_output_mode_default' (expected: stdout|logs|both)"
      return 2
    fi

    runtime_log_level_set="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r 'if (.env // {} | has("LOG_LEVEL")) then "1" else "0" end')"
    runtime_log_level_alias_set="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r 'if (.env // {} | has("NIXFIED_LOG_LEVEL")) then "1" else "0" end')"
    runtime_log_level_value="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '.env.LOG_LEVEL // empty')"
    runtime_log_level_alias_value="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '.env.NIXFIED_LOG_LEVEL // empty')"
    if [ "$runtime_log_level_set" = "1" ] && [ -z "$runtime_log_level_value" ]; then
      echo "ERROR: runtime env LOG_LEVEL cannot be empty when set"
      return 2
    fi
    if [ "$runtime_log_level_alias_set" = "1" ] && [ -z "$runtime_log_level_alias_value" ]; then
      echo "ERROR: runtime env NIXFIED_LOG_LEVEL cannot be empty when set"
      return 2
    fi
    if [ "$runtime_log_level_set" = "1" ] && [ "$runtime_log_level_alias_set" = "1" ] && [ "$runtime_log_level_value" != "$runtime_log_level_alias_value" ]; then
      echo "ERROR: runtime env LOG_LEVEL and NIXFIED_LOG_LEVEL conflict ('$runtime_log_level_value' vs '$runtime_log_level_alias_value')"
      return 2
    fi
    if [ "$runtime_log_level_set" = "1" ]; then
      runtime_log_level_override="$runtime_log_level_value"
    elif [ "$runtime_log_level_alias_set" = "1" ]; then
      runtime_log_level_override="$runtime_log_level_alias_value"
    fi

    runtime_output_mode_set="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r 'if (.env // {} | has("OUTPUT_MODE")) then "1" else "0" end')"
    runtime_output_mode_alias_set="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r 'if (.env // {} | has("NIXFIED_OUTPUT_MODE")) then "1" else "0" end')"
    runtime_output_mode_value="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '.env.OUTPUT_MODE // empty')"
    runtime_output_mode_alias_value="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '.env.NIXFIED_OUTPUT_MODE // empty')"
    if [ "$runtime_output_mode_set" = "1" ] && [ -z "$runtime_output_mode_value" ]; then
      echo "ERROR: runtime env OUTPUT_MODE cannot be empty when set"
      return 2
    fi
    if [ "$runtime_output_mode_alias_set" = "1" ] && [ -z "$runtime_output_mode_alias_value" ]; then
      echo "ERROR: runtime env NIXFIED_OUTPUT_MODE cannot be empty when set"
      return 2
    fi
    if [ "$runtime_output_mode_set" = "1" ] && [ "$runtime_output_mode_alias_set" = "1" ] && [ "$runtime_output_mode_value" != "$runtime_output_mode_alias_value" ]; then
      echo "ERROR: runtime env OUTPUT_MODE and NIXFIED_OUTPUT_MODE conflict ('$runtime_output_mode_value' vs '$runtime_output_mode_alias_value')"
      return 2
    fi
    if [ "$runtime_output_mode_set" = "1" ]; then
      runtime_output_mode_override="$runtime_output_mode_value"
    elif [ "$runtime_output_mode_alias_set" = "1" ]; then
      runtime_output_mode_override="$runtime_output_mode_alias_value"
    fi

    if [ -n "''${LOG_LEVEL+x}" ]; then
      invocation_log_level_set=1
      invocation_log_level_override="$LOG_LEVEL"
    fi
    if [ -n "''${NIXFIED_LOG_LEVEL+x}" ]; then
      invocation_log_level_alias_set=1
      if [ "$invocation_log_level_set" -eq 1 ] && [ "$invocation_log_level_override" != "$NIXFIED_LOG_LEVEL" ]; then
        echo "ERROR: invocation LOG_LEVEL and NIXFIED_LOG_LEVEL conflict ('$invocation_log_level_override' vs '$NIXFIED_LOG_LEVEL')"
        return 2
      fi
      if [ "$invocation_log_level_set" -eq 0 ]; then
        invocation_log_level_override="$NIXFIED_LOG_LEVEL"
      fi
    fi

    if [ -n "''${OUTPUT_MODE+x}" ]; then
      invocation_output_mode_set=1
      invocation_output_mode_override="$OUTPUT_MODE"
    fi
    if [ -n "''${NIXFIED_OUTPUT_MODE+x}" ]; then
      invocation_output_mode_alias_set=1
      if [ "$invocation_output_mode_set" -eq 1 ] && [ "$invocation_output_mode_override" != "$NIXFIED_OUTPUT_MODE" ]; then
        echo "ERROR: invocation OUTPUT_MODE and NIXFIED_OUTPUT_MODE conflict ('$invocation_output_mode_override' vs '$NIXFIED_OUTPUT_MODE')"
        return 2
      fi
      if [ "$invocation_output_mode_set" -eq 0 ]; then
        invocation_output_mode_override="$NIXFIED_OUTPUT_MODE"
      fi
    fi

    cli_log_level_override="''${NIXFIED_CLI_LOG_LEVEL_OVERRIDE:-}"
    cli_output_mode_override="''${NIXFIED_CLI_OUTPUT_MODE_OVERRIDE:-}"

    resolved_log_level="$surface_log_level_default"
    if [ -n "$runtime_log_level_override" ]; then
      resolved_log_level="$runtime_log_level_override"
    fi
    if [ "$invocation_log_level_set" -eq 1 ] || [ "$invocation_log_level_alias_set" -eq 1 ]; then
      resolved_log_level="$invocation_log_level_override"
    fi
    if [ -n "$cli_log_level_override" ]; then
      resolved_log_level="$cli_log_level_override"
    fi

    resolved_output_mode="$surface_output_mode_default"
    if [ -n "$runtime_output_mode_override" ]; then
      resolved_output_mode="$runtime_output_mode_override"
    fi
    if [ "$invocation_output_mode_set" -eq 1 ] || [ "$invocation_output_mode_alias_set" -eq 1 ]; then
      resolved_output_mode="$invocation_output_mode_override"
    fi
    if [ -n "$cli_output_mode_override" ]; then
      resolved_output_mode="$cli_output_mode_override"
    fi

    if ! valid_log_level "$resolved_log_level"; then
      echo "ERROR: invalid LOG_LEVEL value='$resolved_log_level' (expected: error|warn|info|debug|trace)"
      return 2
    fi
    if ! valid_output_mode "$resolved_output_mode"; then
      echo "ERROR: invalid OUTPUT_MODE value='$resolved_output_mode' (expected: stdout|logs|both)"
      return 2
    fi

    LOG_LEVEL="$resolved_log_level"
    NIXFIED_LOG_LEVEL="$resolved_log_level"
    OUTPUT_MODE="$resolved_output_mode"
    NIXFIED_OUTPUT_MODE="$resolved_output_mode"

    runtime_log_file_set="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r 'if (.env // {} | has("NIXFIED_LOG_FILE")) then "1" else "0" end')"
    runtime_log_file_value="$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '.env.NIXFIED_LOG_FILE // empty')"
    if [ -n "''${NIXFIED_LOG_FILE+x}" ]; then
      resolved_log_file="$NIXFIED_LOG_FILE"
    elif [ "$runtime_log_file_set" = "1" ]; then
      resolved_log_file="$runtime_log_file_value"
    fi

    if [ "$resolved_output_mode" = "logs" ] || [ "$resolved_output_mode" = "both" ]; then
      if [ -z "$resolved_log_file" ]; then
        logs_root="$runtime_dir_base"
        if [ -z "$logs_root" ] || [[ "$logs_root" == *"$"* ]]; then
          logs_root="$REGISTRY_ROOT/runtime"
        fi
        logs_dir="$logs_root/logs"
        log_file_run_token="''${NIXFIED_ORCHESTRATOR_RUN_ID:-run-$(date -u +%Y%m%dT%H%M%SZ)-$$}"
        mkdir -p "$logs_dir"
        resolved_log_file="$logs_dir/$log_file_run_token.log"
      fi
    fi

    if [ -n "$resolved_log_file" ]; then
      NIXFIED_LOG_FILE="$resolved_log_file"
    else
      unset NIXFIED_LOG_FILE || true
    fi

    if [ -z "''${RUST_LOG+x}" ] && [ "$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r 'if (.env // {} | has("RUST_LOG")) then "1" else "0" end')" != "1" ]; then
      RUST_LOG="$resolved_log_level"
    fi
    if [ -z "''${MFM_LOG+x}" ] && [ "$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r 'if (.env // {} | has("MFM_LOG")) then "1" else "0" end')" != "1" ]; then
      MFM_LOG="$resolved_log_level"
    fi
    if [ -z "''${MFM_TEST_LOG_FILTER+x}" ] && [ "$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r 'if (.env // {} | has("MFM_TEST_LOG_FILTER")) then "1" else "0" end')" != "1" ]; then
      MFM_TEST_LOG_FILTER="$resolved_log_level"
    fi
    if [ -z "''${MFM_TEST_LOG+x}" ] && [ "$(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r 'if (.env // {} | has("MFM_TEST_LOG")) then "1" else "0" end')" != "1" ]; then
      case "$resolved_log_level" in
        debug|trace)
          MFM_TEST_LOG="1"
          ;;
        *)
          MFM_TEST_LOG="0"
          ;;
      esac
    fi

    if [ "$(${pkgs.coreutils}/bin/uname -s)" = "Darwin" ]; then
      host_developer_dir="''${DEVELOPER_DIR:-}"
      host_sdkroot="''${SDKROOT:-}"
      if [ -z "$host_sdkroot" ] && [ -x /usr/bin/xcrun ]; then
        if [ -n "$host_developer_dir" ]; then
          host_sdkroot="$(
            DEVELOPER_DIR="$host_developer_dir" /usr/bin/xcrun --sdk macosx --show-sdk-path 2>/dev/null || true
          )"
        else
          host_sdkroot="$(/usr/bin/xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)"
        fi
      fi
    fi

    local -a env_cmd
    env_cmd=(env -i "PATH=$final_path" "LANG=$locale" "LC_ALL=$locale" "TZ=$timezone" "HOME=$home_value")

    env_cmd+=("NIXFIED_PROJECT_NAME=$PROJECT_NAME")
    env_cmd+=("NIXFIED_PROJECT_DESCRIPTION=$PROJECT_DESCRIPTION")
    env_cmd+=("NIXFIED_RUNTIME_DIR_BASE=$runtime_dir_base")
    env_cmd+=("LOG_LEVEL=$resolved_log_level")
    env_cmd+=("NIXFIED_LOG_LEVEL=$resolved_log_level")
    env_cmd+=("OUTPUT_MODE=$resolved_output_mode")
    env_cmd+=("NIXFIED_OUTPUT_MODE=$resolved_output_mode")
    if [ -n "''${NIXFIED_LOG_FILE+x}" ]; then
      env_cmd+=("NIXFIED_LOG_FILE=$NIXFIED_LOG_FILE")
    fi

    while IFS=$'\t' read -r primitive_name primitive_default primitive_aliases_json; do
      local effective_value
      local alias

      if [ -z "$primitive_name" ]; then
        continue
      fi
      if [ "$primitive_name" = "LOG_LEVEL" ] || [ "$primitive_name" = "OUTPUT_MODE" ]; then
        continue
      fi

      if [ -n "''${!primitive_name+x}" ]; then
        effective_value="''${!primitive_name}"
      elif [ "$primitive_default" != "__NIXFIED_NULL__" ]; then
        effective_value="$primitive_default"
        env_cmd+=("$primitive_name=$effective_value")
      else
        continue
      fi

      while IFS= read -r alias; do
        if [ -n "$alias" ] && [ -z "''${!alias+x}" ]; then
          env_cmd+=("$alias=$effective_value")
        fi
      done < <(printf '%s' "$primitive_aliases_json" | ${pkgs.jq}/bin/jq -r '.[]?')
    done < <(
      printf '%s' "$RUNTIME_JSON" |
        ${pkgs.jq}/bin/jq -r '
          .primitives.defs // {}
          | to_entries[]?
          | [
              .key,
              (if .value.default == null then "__NIXFIED_NULL__" else (.value.default | tostring) end),
              ((.value.aliases // []) | tojson)
            ]
          | @tsv
        '
    )

    while IFS=$'\t' read -r port_name port_base; do
      local port_token
      local port_var
      local port_value

      if [ -z "$port_name" ] || [ -z "$port_base" ]; then
        continue
      fi

      port_token="$(normalize_env_token "$port_name")"
      port_var="''${port_token}_PORT"
      port_value="$(( port_base + env_offset + (slot_value * slot_stride) ))"
      env_cmd+=("$port_var=$port_value")
    done < <(
      printf '%s' "$RUNTIME_JSON" |
        ${pkgs.jq}/bin/jq -r '.ports // {} | to_entries[]? | [.key, (.value | tostring)] | @tsv'
    )

    while IFS=$'\t' read -r service_name service_enabled; do
      local service_token
      local service_var

      if [ -z "$service_name" ]; then
        continue
      fi

      service_token="$(normalize_env_token "$service_name")"
      service_var="NIXFIED_SERVICE_''${service_token}_ENABLED"
      env_cmd+=("$service_var=$service_enabled")
    done < <(
      printf '%s' "$SERVICES_JSON" |
        ${pkgs.jq}/bin/jq -r 'to_entries[]? | [.value.name, (if .value.enable then "1" else "0" end)] | @tsv'
    )

    while IFS=$'\t' read -r service_name config_key config_value; do
      local service_token
      local config_token
      local config_var

      if [ -z "$service_name" ] || [ -z "$config_key" ]; then
        continue
      fi

      service_token="$(normalize_env_token "$service_name")"
      config_token="$(normalize_env_token "$config_key")"
      config_var="NIXFIED_SERVICE_''${service_token}_''${config_token}"
      env_cmd+=("$config_var=$config_value")
    done < <(
      printf '%s' "$SERVICES_JSON" |
        ${pkgs.jq}/bin/jq -r '
          to_entries[]?
          | .value as $service
          | ($service.config // {} | to_entries[]? | [ $service.name, .key, (.value | tostring) ] | @tsv)
        '
    )

    while IFS= read -r pass_name; do
      if [ -z "$pass_name" ]; then
        continue
      fi

      if [ "$allow_sensitive_pass_through" != "1" ] && is_sensitive_env_name "$pass_name"; then
        echo "ERROR: sensitive passthrough env blocked name=$pass_name"
        return 3
      fi

      if [ -n "''${!pass_name+x}" ]; then
        env_cmd+=("$pass_name=''${!pass_name}")
      fi
    done < <(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '.passThroughEnv[]?')

    if [ -n "$host_developer_dir" ] && [ -z "''${DEVELOPER_DIR+x}" ]; then
      env_cmd+=("DEVELOPER_DIR=$host_developer_dir")
    fi
    if [ -n "$host_sdkroot" ] && [ -z "''${SDKROOT+x}" ]; then
      env_cmd+=("SDKROOT=$host_sdkroot")
    fi

    while IFS=$'\t' read -r env_name env_value; do
      if [ -n "$env_name" ]; then
        if [ "$env_name" = "LOG_LEVEL" ] || [ "$env_name" = "NIXFIED_LOG_LEVEL" ]; then
          continue
        fi
        if [ "$env_name" = "OUTPUT_MODE" ] || [ "$env_name" = "NIXFIED_OUTPUT_MODE" ]; then
          continue
        fi
        if [ "$env_name" = "NIXFIED_LOG_FILE" ]; then
          continue
        fi
        env_cmd+=("$env_name=$env_value")
      fi
    done < <(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '.env | to_entries[]? | [.key, (.value | tostring)] | @tsv')

    umask "$umask_value"

    (
      cd "$workdir"
      "''${env_cmd[@]}" ${pkgs.bash}/bin/bash -euo pipefail -c "$command" -- "$@"
    )
  }

  run_in_sandbox() {
    local task_json="$1"
    shift
    local command="$1"
    shift
    local runtime_json
    runtime_json="$(printf '%s' "$task_json" | ${pkgs.jq}/bin/jq -c '.runtime')"
    run_in_sandbox_runtime "$runtime_json" "$command" "$@"
  }
''
