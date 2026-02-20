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
    env_cmd+=("NIXFIED_LOG_LEVEL=''${NIXFIED_LOG_LEVEL:-$log_level_default}")
    env_cmd+=("NIXFIED_OUTPUT_MODE=''${NIXFIED_OUTPUT_MODE:-$output_mode_default}")

    while IFS=$'\t' read -r primitive_name primitive_default primitive_aliases_json; do
      local effective_value
      local alias

      if [ -z "$primitive_name" ]; then
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
      if [ -n "$pass_name" ] && [ -n "''${!pass_name+x}" ]; then
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
