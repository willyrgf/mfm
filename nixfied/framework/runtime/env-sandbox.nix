{
  pkgs,
  projectRoot,
  model,
  services,
  serviceHookEnv ? { },
}:
let
  lib = pkgs.lib;
  commonRuntimeShell = import ./common-runtime.nix { inherit pkgs; };

  valueToString =
    value:
    if value == null then
      ""
    else if builtins.isBool value then
      if value then "1" else "0"
    else if builtins.isAttrs value || builtins.isList value then
      builtins.toJSON value
    else
      toString value;

  normalizeStaticToken =
    value: lib.toUpper (lib.replaceStrings [ "." "-" ":" "/" " " ] [ "_" "_" "_" "_" "_" ] value);

  runtimeEnvOffsetNames = builtins.sort builtins.lessThan (
    builtins.attrNames (model.runtime.env.offsets or { })
  );
  runtimePrimitiveNames = builtins.sort builtins.lessThan (
    builtins.attrNames (model.runtime.primitives.defs or { })
  );
  runtimePortNames = builtins.sort builtins.lessThan (
    builtins.attrNames (model.runtime.ports or { })
  );
  serviceIds = builtins.sort builtins.lessThan (builtins.attrNames (services));

  staticRuntimePackagesPath = lib.concatStringsSep ":" (
    map (runtimeInput: "${runtimeInput}/bin") model.runtime.runtimePackages
  );

  staticRuntimeEnvOffsetCase = lib.concatStringsSep "\n" (
    map (envName: ''
      ${lib.escapeShellArg envName})
        printf '%s' ${lib.escapeShellArg (toString model.runtime.env.offsets.${envName})}
        ;;
    '') runtimeEnvOffsetNames
  );

  staticRuntimePrimitivesTsv = lib.concatStringsSep "\n" (
    map (
      primitiveName:
      let
        primitive = model.runtime.primitives.defs.${primitiveName};
      in
      "${primitiveName}\t${
        if primitive.default == null then "__NIXFIED_NULL__" else valueToString primitive.default
      }\t${lib.concatStringsSep " " (primitive.aliases or [ ])}"
    ) runtimePrimitiveNames
  );

  staticRuntimePortsTsv = lib.concatStringsSep "\n" (
    map (portName: "${portName}\t${toString model.runtime.ports.${portName}}") runtimePortNames
  );

  staticServiceNames = lib.concatStringsSep "\n" (
    map (
      serviceId:
      let
        service = services.${serviceId};
      in
      "${service.name}\t${service.config.dataDirName or service.name}"
    ) serviceIds
  );

  serviceHookEntries = builtins.concatLists (
    map (
      serviceId:
      let
        service = services.${serviceId};
        serviceName = service.name;
        serviceToken = normalizeStaticToken serviceName;
        hookNames = builtins.filter (hookName: lib.hasPrefix "SVC_${serviceToken}_" hookName) (
          builtins.attrNames serviceHookEnv
        );
      in
      map (hookName: {
        inherit
          serviceName
          hookName
          ;
        value = serviceHookEnv.${hookName};
      }) hookNames
    ) serviceIds
  );

  staticServiceHookEnvCmds = lib.concatStringsSep "\n" (
    map (entry: ''
      if runtime_service_selected ${lib.escapeShellArg entry.serviceName}; then
        env_cmd+=(${lib.escapeShellArg "${entry.hookName}=${entry.value}"})
      fi
    '') serviceHookEntries
  );

in
''
    ${commonRuntimeShell}

    PROJECT_NAME=${pkgs.lib.escapeShellArg model.identity.projectName}
    PROJECT_DESCRIPTION=${pkgs.lib.escapeShellArg model.identity.description}
    PROJECT_ID=${pkgs.lib.escapeShellArg model.identity.projectId}
    PROJECT_ID_UPPER=${
      pkgs.lib.escapeShellArg (
        pkgs.lib.toUpper (pkgs.lib.replaceStrings [ "-" "." ] [ "_" "_" ] model.identity.projectId)
      )
    }
    RUNTIME_DIR_BASE_DEFAULT=${pkgs.lib.escapeShellArg model.runtime.directories.base}
    ENV_SANDBOX_STATIC_RUNTIME_PACKAGES_PATH=${lib.escapeShellArg staticRuntimePackagesPath}
    ENV_SANDBOX_STATIC_RUNTIME_SLOT_VAR=${lib.escapeShellArg model.runtime.slot.var}
    ENV_SANDBOX_STATIC_RUNTIME_ENV_VAR=${lib.escapeShellArg model.runtime.env.var}
    ENV_SANDBOX_STATIC_RUNTIME_SLOT_DEFAULT=${lib.escapeShellArg (toString model.runtime.slot.default)}
    ENV_SANDBOX_STATIC_RUNTIME_ENV_DEFAULT=${lib.escapeShellArg model.runtime.env.default}
    ENV_SANDBOX_STATIC_RUNTIME_SLOT_STRIDE=${lib.escapeShellArg (toString model.runtime.slot.stride)}
    ENV_SANDBOX_STATIC_RUNTIME_DIR_BASE=${lib.escapeShellArg model.runtime.directories.base}
    ENV_SANDBOX_STATIC_LOG_LEVEL_DEFAULT=${lib.escapeShellArg model.runtime.logging.levelDefault}
    ENV_SANDBOX_STATIC_OUTPUT_MODE_DEFAULT=${lib.escapeShellArg model.runtime.logging.outputDefault}
    ENV_SANDBOX_STATIC_RUNTIME_PRIMITIVES_TSV=${lib.escapeShellArg staticRuntimePrimitivesTsv}
    ENV_SANDBOX_STATIC_RUNTIME_PORTS_TSV=${lib.escapeShellArg staticRuntimePortsTsv}
    ENV_SANDBOX_STATIC_SERVICE_NAMES=${lib.escapeShellArg staticServiceNames}

    normalize_env_token() {
      printf '%s' "$1" | ${pkgs.coreutils}/bin/tr '[:lower:].-' '[:upper:]__' | ${pkgs.coreutils}/bin/tr -c 'A-Z0-9_' '_'
    }

    runtime_service_selected() {
      local service_name="$1"
      local selected_services_csv="''${NIXFIED_SELECTED_SERVICES_CSV:-}"
      local selected_service=""
      local old_ifs="$IFS"
      local selected_parts=()

      if [ -z "$selected_services_csv" ]; then
        return 1
      fi

      IFS=','
      read -r -a selected_parts <<< "$selected_services_csv"
      IFS="$old_ifs"

      for selected_service in "''${selected_parts[@]}"; do
        selected_service="$(printf '%s' "$selected_service" | ${pkgs.coreutils}/bin/tr -d '[:space:]')"
        if [ -n "$selected_service" ] && [ "$selected_service" = "$service_name" ]; then
          return 0
        fi
      done

      return 1
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

    is_reserved_runtime_env_name() {
      local env_name="$1"

      case "$env_name" in
        HOME|TMPDIR|XDG_DATA_HOME|XDG_STATE_HOME|XDG_CACHE_HOME|REGISTRY_ROOT|CI_ARTIFACTS_DIR|NIXFIED_SERVICE_ROOT)
          return 0
          ;;
        NIXFIED_RUNTIME_DIR_SCOPE_OVERRIDE|NIXFIED_RUNTIME_DIR_SCOPE|NIXFIED_RUNTIME_DIR_BASE)
          return 0
          ;;
        NIXFIED_RUNTIME_HOME|NIXFIED_RUNTIME_TMPDIR|NIXFIED_RUNTIME_XDG_DATA_HOME|NIXFIED_RUNTIME_XDG_STATE_HOME|NIXFIED_RUNTIME_XDG_CACHE_HOME)
          return 0
          ;;
        NIXFIED_RUNTIME_REGISTRY_ROOT|NIXFIED_RUNTIME_ARTIFACTS_DIR|NIXFIED_RUNTIME_SERVICE_ROOT)
          return 0
          ;;
        NIXFIED_MODEL_FILE|NIXFIED_RUN_ID)
          return 0
          ;;
        NIXFIED_EXECUTOR_BIN|NIXFIED_ORCHESTRATOR_BIN|NIXFIED_EXECUTOR_SELF|NIXFIED_ORCHESTRATOR_SELF)
          return 0
          ;;
        NIXFIED_EXECUTION_EPHEMERAL|NIXFIED_ORCHESTRATOR_RUN_ID|NIXFIED_ORCHESTRATOR_PROCESS_MODE|NIXFIED_ORCHESTRATOR_WORKFLOW_ID|NIXFIED_PARENT_WORKFLOW_ID|NIXFIED_TASK_ID)
          return 0
          ;;
        NIXFIED_WORKFLOW_SETUP_STARTED_AT|NIXFIED_WORKFLOW_SETUP_STARTED_EPOCH)
          return 0
          ;;
        SVC_*)
          return 0
          ;;
        *)
          return 1
          ;;
      esac
    }

    ensure_runtime_dir() {
      local path="$1"
      if [ -n "$path" ]; then
        mkdir -p "$path"
      fi
    }

    env_sandbox_runtime_env_offset() {
      case "$1" in
  ${staticRuntimeEnvOffsetCase}
        *)
          return 1
          ;;
      esac
    }

    load_runtime_plan() {
      local runtime_json="$1"
      local runtime_plan_shell=""

      runtime_plan_shell="$(
        printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '
          def emit($name; $value): "\($name)=\($value | @sh)";
          . as $runtime
          | ($runtime.env // {}) as $env
          | [
              emit("runtime_inputs_path"; (($runtime.runtimeInputs // []) | map(. + "/bin") | join(":"))),
              emit("locale"; ($runtime.locale // "C.UTF-8")),
              emit("timezone"; ($runtime.timezone // "UTC")),
              emit("umask_value"; ($runtime.umask // "022")),
              emit("allow_sensitive_pass_through"; (if ($runtime.allowSensitivePassThrough // false) then "1" else "0" end)),
              emit("workdir_kind"; ($runtime.workdir // "projectRoot")),
              emit("custom_workdir"; ($runtime.customWorkdir // "")),
              emit("task_log_level_default"; ($runtime.logging.levelDefault // "")),
              emit("task_output_mode_default"; ($runtime.logging.outputDefault // "")),
              emit("runtime_env_tsv"; (($env | to_entries | map([.key, (.value | tostring)] | @tsv) | join("\n")))),
              emit("pass_through_env_tsv"; (($runtime.passThroughEnv // []) | join("\n"))),
              emit("runtime_log_level_set"; (if ($env | has("LOG_LEVEL")) then "1" else "0" end)),
              emit("runtime_log_level_alias_set"; (if ($env | has("NIXFIED_LOG_LEVEL")) then "1" else "0" end)),
              emit("runtime_log_level_value"; (if ($env | has("LOG_LEVEL")) then ($env.LOG_LEVEL | tostring) else "" end)),
              emit("runtime_log_level_alias_value"; (if ($env | has("NIXFIED_LOG_LEVEL")) then ($env.NIXFIED_LOG_LEVEL | tostring) else "" end)),
              emit("runtime_output_mode_set"; (if ($env | has("OUTPUT_MODE")) then "1" else "0" end)),
              emit("runtime_output_mode_alias_set"; (if ($env | has("NIXFIED_OUTPUT_MODE")) then "1" else "0" end)),
              emit("runtime_output_mode_value"; (if ($env | has("OUTPUT_MODE")) then ($env.OUTPUT_MODE | tostring) else "" end)),
              emit("runtime_output_mode_alias_value"; (if ($env | has("NIXFIED_OUTPUT_MODE")) then ($env.NIXFIED_OUTPUT_MODE | tostring) else "" end)),
              emit("runtime_log_file_set"; (if ($env | has("NIXFIED_LOG_FILE")) then "1" else "0" end)),
              emit("runtime_log_file_value"; (if ($env | has("NIXFIED_LOG_FILE")) then ($env.NIXFIED_LOG_FILE | tostring) else "" end)),
              emit("runtime_has_rust_log"; (if ($env | has("RUST_LOG")) then "1" else "0" end)),
              emit("runtime_has_mfm_log"; (if ($env | has("MFM_LOG")) then "1" else "0" end)),
              emit("runtime_has_mfm_test_log_filter"; (if ($env | has("MFM_TEST_LOG_FILTER")) then "1" else "0" end)),
              emit("runtime_has_mfm_test_log"; (if ($env | has("MFM_TEST_LOG")) then "1" else "0" end))
            ]
          | .[]
        '
      )" || {
        echo "ERROR: failed to parse runtime plan"
        return 1
      }

      eval "$runtime_plan_shell"
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
      local runtime_inputs_path=""
      local base_path
      local final_path
      local host_developer_dir=""
      local host_sdkroot=""
      local pass_through_env_tsv=""
      local runtime_env_tsv=""

      local slot_var
      local env_var
      local slot_default
      local env_default
      local slot_stride
      local slot_value
      local env_value
      local env_offset
      local runtime_dir_base
      local runtime_scope_root
      local runtime_scope_override
      local project_ephemeral_flag_var
      local project_ephemeral_root_var
      local ephemeral_root=""
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
      local runtime_has_rust_log="0"
      local runtime_has_mfm_log="0"
      local runtime_has_mfm_test_log_filter="0"
      local runtime_has_mfm_test_log="0"
      local resolved_log_file=""
      local logs_root
      local logs_dir
      local log_file_run_token
      local allow_sensitive_pass_through
      local home_value
      local tmp_value
      local xdg_data_value
      local xdg_state_value
      local xdg_cache_value
      local registry_root_value
      local artifacts_dir_value
      local services_root

      resolve_project_root_workdir() {
        local project_root_real="${builtins.toString projectRoot}"
        local anchor_root="$project_root_real"
        local effective_root="$project_root_real"
        local caller_pwd="''${NIXFIED_CALLER_PWD:-}"
        local caller_pwd_real=""
        local caller_relative=""

        if [ -d "$project_root_real" ]; then
          project_root_real="$(cd "$project_root_real" && pwd -P)"
        fi

        if [ -n "''${ORIGINAL_ROOT:-}" ] && [ -d "$ORIGINAL_ROOT" ]; then
          anchor_root="$(cd "$ORIGINAL_ROOT" && pwd -P)"
        else
          anchor_root="$project_root_real"
        fi

        if [ "''${NIXFIED_EXECUTION_EPHEMERAL:-0}" = "1" ]; then
          effective_root="$(pwd -P)"
        else
          effective_root="$project_root_real"
        fi

        if [ -n "$caller_pwd" ] && [ -d "$caller_pwd" ]; then
          caller_pwd_real="$(cd "$caller_pwd" && pwd -P)"

          case "$caller_pwd_real" in
            "$effective_root")
              printf '%s' "$effective_root"
              return 0
              ;;
            "$effective_root"/*)
              printf '%s' "$caller_pwd_real"
              return 0
              ;;
            "$anchor_root")
              caller_relative="."
              ;;
            "$anchor_root"/*)
              caller_relative="''${caller_pwd_real#"$anchor_root"/}"
              ;;
          esac

          if [ -n "$caller_relative" ]; then
            if [ "$caller_relative" = "." ]; then
              printf '%s' "$effective_root"
            else
              printf '%s/%s' "$effective_root" "$caller_relative"
            fi
            return 0
          fi

          case "$project_root_real" in
            /nix/store/*)
              # Remote framework proxy apps execute from a store path, but they
              # should treat the caller checkout as the project root by default.
              printf '%s' "$caller_pwd_real"
              return 0
              ;;
          esac
        fi

        printf '%s' "$effective_root"
      }

      load_runtime_plan "$runtime_json" || return 1

      runtime_path="$ENV_SANDBOX_STATIC_RUNTIME_PACKAGES_PATH"
      if [ -n "$runtime_inputs_path" ]; then
        if [ -z "$runtime_path" ]; then
          runtime_path="$runtime_inputs_path"
        else
          runtime_path="$runtime_path:$runtime_inputs_path"
        fi
      fi

      base_path="${pkgs.coreutils}/bin:${pkgs.findutils}/bin:${pkgs.gnused}/bin:${pkgs.gnugrep}/bin:${pkgs.jq}/bin:${pkgs.bash}/bin"
      if [ -n "$runtime_path" ]; then
        final_path="$runtime_path:$base_path"
      else
        final_path="$base_path"
      fi

      slot_var="$ENV_SANDBOX_STATIC_RUNTIME_SLOT_VAR"
      env_var="$ENV_SANDBOX_STATIC_RUNTIME_ENV_VAR"
      slot_default="$ENV_SANDBOX_STATIC_RUNTIME_SLOT_DEFAULT"
      env_default="$ENV_SANDBOX_STATIC_RUNTIME_ENV_DEFAULT"
      slot_stride="$ENV_SANDBOX_STATIC_RUNTIME_SLOT_STRIDE"

      slot_value="''${!slot_var:-$slot_default}"
      env_value="''${!env_var:-$env_default}"

      if ! [[ "$slot_value" =~ ^[0-9]+$ ]]; then
        echo "ERROR: $slot_var must be an integer"
        return 3
      fi

      if ! env_offset="$(env_sandbox_runtime_env_offset "$env_value")"; then
        echo "ERROR: unsupported $env_var '$env_value'"
        return 3
      fi

      runtime_dir_base="$ENV_SANDBOX_STATIC_RUNTIME_DIR_BASE"
      project_ephemeral_flag_var="''${PROJECT_ID_UPPER}_EPHEMERAL"
      project_ephemeral_root_var="''${PROJECT_ID_UPPER}_EPHEMERAL_ROOT"
      ephemeral_root="''${!project_ephemeral_root_var:-}"
      runtime_scope_override="''${NIXFIED_RUNTIME_DIR_SCOPE_OVERRIDE:-}"
      log_level_default="$ENV_SANDBOX_STATIC_LOG_LEVEL_DEFAULT"
      output_mode_default="$ENV_SANDBOX_STATIC_OUTPUT_MODE_DEFAULT"

      if [ -z "$runtime_dir_base" ] || [[ "$runtime_dir_base" == *"$"* ]]; then
        runtime_dir_base="$RUNTIME_DIR_BASE_DEFAULT"
      fi
      if [ -n "$runtime_scope_override" ]; then
        runtime_scope_root="$runtime_scope_override"
      elif [ -n "$ephemeral_root" ]; then
        runtime_scope_root="$ephemeral_root"
      else
        runtime_scope_root="$runtime_dir_base/$env_value/slot-$slot_value"
      fi

      home_value="$runtime_scope_root/home"
      tmp_value="$runtime_scope_root/tmp"
      xdg_data_value="$runtime_scope_root/xdg/data"
      xdg_state_value="$runtime_scope_root/xdg/state"
      xdg_cache_value="$runtime_scope_root/xdg/cache"
      registry_root_value="$runtime_scope_root/registry"
      artifacts_dir_value="$runtime_scope_root/artifacts"
      services_root="$runtime_scope_root/services"

      ensure_runtime_dir "$home_value"
      ensure_runtime_dir "$tmp_value"
      ensure_runtime_dir "$xdg_data_value"
      ensure_runtime_dir "$xdg_state_value"
      ensure_runtime_dir "$xdg_cache_value"
      ensure_runtime_dir "$registry_root_value"
      ensure_runtime_dir "$artifacts_dir_value"
      ensure_runtime_dir "$services_root"

      case "$workdir_kind" in
        projectRoot)
          workdir="$(resolve_project_root_workdir)"
          ;;
        stateRoot)
          workdir="$registry_root_value"
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

      if ! valid_log_level "$log_level_default"; then
        echo "ERROR: invalid runtime default LOG_LEVEL value='$log_level_default' (expected: error|warn|info|debug|trace)"
        return 2
      fi
      if ! valid_output_mode "$output_mode_default"; then
        echo "ERROR: invalid runtime default OUTPUT_MODE value='$output_mode_default' (expected: stdout|logs|both)"
        return 2
      fi

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

      if [ -z "''${RUST_LOG+x}" ] && [ "$runtime_has_rust_log" != "1" ]; then
        RUST_LOG="$resolved_log_level"
      fi
      if [ -z "''${MFM_LOG+x}" ] && [ "$runtime_has_mfm_log" != "1" ]; then
        MFM_LOG="$resolved_log_level"
      fi
      if [ -z "''${MFM_TEST_LOG_FILTER+x}" ] && [ "$runtime_has_mfm_test_log_filter" != "1" ]; then
        MFM_TEST_LOG_FILTER="$resolved_log_level"
      fi
      if [ -z "''${MFM_TEST_LOG+x}" ] && [ "$runtime_has_mfm_test_log" != "1" ]; then
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

      local host_nix_user_conf_files="''${NIX_USER_CONF_FILES:-}"
      local host_nix_config="''${NIX_CONFIG:-}"
      local host_nix_ssl_cert_file="''${NIX_SSL_CERT_FILE:-}"

      local -a env_cmd
      env_cmd=(
        env -i
        "PATH=$final_path"
        "LANG=$locale"
        "LC_ALL=$locale"
        "TZ=$timezone"
        "HOME=$home_value"
        "TMPDIR=$tmp_value"
        "XDG_DATA_HOME=$xdg_data_value"
        "XDG_STATE_HOME=$xdg_state_value"
        "XDG_CACHE_HOME=$xdg_cache_value"
        "REGISTRY_ROOT=$registry_root_value"
        "CI_ARTIFACTS_DIR=$artifacts_dir_value"
        "NIXFIED_SERVICE_ROOT=$services_root"
      )

      env_cmd+=("NIXFIED_PROJECT_NAME=$PROJECT_NAME")
      env_cmd+=("NIXFIED_PROJECT_DESCRIPTION=$PROJECT_DESCRIPTION")
      env_cmd+=("NIXFIED_RUNTIME_DIR_BASE=$runtime_dir_base")
      env_cmd+=("NIXFIED_RUNTIME_DIR_SCOPE=$runtime_scope_root")
      env_cmd+=("NIXFIED_RUNTIME_HOME=$home_value")
      env_cmd+=("NIXFIED_RUNTIME_TMPDIR=$tmp_value")
      env_cmd+=("NIXFIED_RUNTIME_XDG_DATA_HOME=$xdg_data_value")
      env_cmd+=("NIXFIED_RUNTIME_XDG_STATE_HOME=$xdg_state_value")
      env_cmd+=("NIXFIED_RUNTIME_XDG_CACHE_HOME=$xdg_cache_value")
      env_cmd+=("NIXFIED_RUNTIME_REGISTRY_ROOT=$registry_root_value")
      env_cmd+=("NIXFIED_RUNTIME_ARTIFACTS_DIR=$artifacts_dir_value")
      env_cmd+=("NIXFIED_RUNTIME_SERVICE_ROOT=$services_root")
      env_cmd+=("LOG_LEVEL=$resolved_log_level")
      env_cmd+=("NIXFIED_LOG_LEVEL=$resolved_log_level")
      env_cmd+=("OUTPUT_MODE=$resolved_output_mode")
      env_cmd+=("NIXFIED_OUTPUT_MODE=$resolved_output_mode")
      if [ -n "$host_nix_user_conf_files" ]; then
        env_cmd+=("NIX_USER_CONF_FILES=$host_nix_user_conf_files")
      fi
      if [ -n "$host_nix_config" ]; then
        env_cmd+=("NIX_CONFIG=$host_nix_config")
      fi
      if [ -n "$host_nix_ssl_cert_file" ]; then
        env_cmd+=("NIX_SSL_CERT_FILE=$host_nix_ssl_cert_file")
      fi
      if [ -n "''${NIXFIED_EXECUTOR_BIN:-}" ]; then
        env_cmd+=("NIXFIED_EXECUTOR_BIN=$NIXFIED_EXECUTOR_BIN")
      fi
      if [ -n "''${NIXFIED_ORCHESTRATOR_BIN:-}" ]; then
        env_cmd+=("NIXFIED_ORCHESTRATOR_BIN=$NIXFIED_ORCHESTRATOR_BIN")
      fi
      if [ -n "$ephemeral_root" ]; then
        env_cmd+=("''${project_ephemeral_root_var}=$ephemeral_root")
      fi
      if [ -n "''${!project_ephemeral_flag_var:-}" ]; then
        env_cmd+=("''${project_ephemeral_flag_var}=1")
      fi
      if [ -n "''${NIXFIED_LOG_FILE+x}" ]; then
        env_cmd+=("NIXFIED_LOG_FILE=$NIXFIED_LOG_FILE")
      fi
      if [ -n "''${NIXFIED_EXECUTOR_SELF:-}" ]; then
        env_cmd+=("NIXFIED_EXECUTOR_SELF=$NIXFIED_EXECUTOR_SELF")
      fi
      if [ -n "''${NIXFIED_ORCHESTRATOR_SELF:-}" ]; then
        env_cmd+=("NIXFIED_ORCHESTRATOR_SELF=$NIXFIED_ORCHESTRATOR_SELF")
      fi
      if [ -n "''${NIXFIED_EXECUTION_EPHEMERAL:-}" ]; then
        env_cmd+=("NIXFIED_EXECUTION_EPHEMERAL=$NIXFIED_EXECUTION_EPHEMERAL")
      fi
      if [ -n "''${NIXFIED_ORCHESTRATOR_RUN_ID:-}" ]; then
        env_cmd+=("NIXFIED_ORCHESTRATOR_RUN_ID=$NIXFIED_ORCHESTRATOR_RUN_ID")
      fi
      if [ -n "''${NIXFIED_ORCHESTRATOR_PROCESS_MODE:-}" ]; then
        env_cmd+=("NIXFIED_ORCHESTRATOR_PROCESS_MODE=$NIXFIED_ORCHESTRATOR_PROCESS_MODE")
      fi
      if [ -n "''${NIXFIED_ORCHESTRATOR_WORKFLOW_ID:-}" ]; then
        env_cmd+=("NIXFIED_ORCHESTRATOR_WORKFLOW_ID=$NIXFIED_ORCHESTRATOR_WORKFLOW_ID")
      fi
      if [ -n "''${NIXFIED_PARENT_WORKFLOW_ID:-}" ]; then
        env_cmd+=("NIXFIED_PARENT_WORKFLOW_ID=$NIXFIED_PARENT_WORKFLOW_ID")
      fi
      if [ -n "''${NIXFIED_SELECTED_SERVICES_CSV:-}" ]; then
        env_cmd+=("NIXFIED_SELECTED_SERVICES_CSV=$NIXFIED_SELECTED_SERVICES_CSV")
      fi
      if [ -n "''${NIXFIED_TASK_ID:-}" ]; then
        env_cmd+=("NIXFIED_TASK_ID=$NIXFIED_TASK_ID")
      fi
      if [ -n "''${NIXFIED_WORKFLOW_SETUP_STARTED_AT:-}" ]; then
        env_cmd+=("NIXFIED_WORKFLOW_SETUP_STARTED_AT=$NIXFIED_WORKFLOW_SETUP_STARTED_AT")
      fi
      if [ -n "''${NIXFIED_WORKFLOW_SETUP_STARTED_EPOCH:-}" ]; then
        env_cmd+=("NIXFIED_WORKFLOW_SETUP_STARTED_EPOCH=$NIXFIED_WORKFLOW_SETUP_STARTED_EPOCH")
      fi
      if [ -n "''${NIXFIED_MODEL_FILE:-}" ]; then
        env_cmd+=("NIXFIED_MODEL_FILE=$NIXFIED_MODEL_FILE")
      fi
      if [ -n "''${NIXFIED_RUN_ID:-}" ]; then
        env_cmd+=("NIXFIED_RUN_ID=$NIXFIED_RUN_ID")
      fi
      if [ -n "''${NIX_BUILD_TOP:-}" ]; then
        env_cmd+=("NIX_BUILD_TOP=$NIX_BUILD_TOP")
      fi

      while IFS=$'\t' read -r primitive_name primitive_default primitive_aliases; do
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

        for alias in $primitive_aliases; do
          if [ -n "$alias" ] && [ -z "''${!alias+x}" ]; then
            env_cmd+=("$alias=$effective_value")
          fi
        done
      done <<< "$ENV_SANDBOX_STATIC_RUNTIME_PRIMITIVES_TSV"

      while IFS=$'\t' read -r port_name port_base || [ -n "$port_name" ]; do
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
      done <<< "$ENV_SANDBOX_STATIC_RUNTIME_PORTS_TSV"

      while IFS=$'\t' read -r service_name service_data_dir_name || [ -n "$service_name" ]; do
        local service_token
        local service_root
        local service_data_dir
        local service_state_dir
        local service_log_dir

        if [ -z "$service_name" ]; then
          continue
        fi
        if ! runtime_service_selected "$service_name"; then
          continue
        fi

        service_token="$(normalize_env_token "$service_name")"
        if [ -z "$service_data_dir_name" ]; then
          service_data_dir_name="$service_name"
        fi
        service_root="$services_root/$service_data_dir_name"
        service_data_dir="$service_root/data"
        service_state_dir="$service_root/state"
        service_log_dir="$service_root/log"

        ensure_runtime_dir "$service_data_dir"
        ensure_runtime_dir "$service_state_dir"
        ensure_runtime_dir "$service_log_dir"

        env_cmd+=("NIXFIED_SERVICE_''${service_token}_DATA_DIR=$service_data_dir")
        env_cmd+=("NIXFIED_SERVICE_''${service_token}_STATE_DIR=$service_state_dir")
        env_cmd+=("NIXFIED_SERVICE_''${service_token}_LOG_DIR=$service_log_dir")
      done <<< "$ENV_SANDBOX_STATIC_SERVICE_NAMES"

  ${staticServiceHookEnvCmds}

      while IFS= read -r pass_name || [ -n "$pass_name" ]; do
        if [ -z "$pass_name" ]; then
          continue
        fi

        if is_reserved_runtime_env_name "$pass_name"; then
          echo "ERROR: runtime-owned passthrough env blocked name=$pass_name"
          return 3
        fi

        if [ "$allow_sensitive_pass_through" != "1" ] && is_sensitive_env_name "$pass_name"; then
          echo "ERROR: sensitive passthrough env blocked name=$pass_name"
          return 3
        fi

        if [ -n "''${!pass_name+x}" ]; then
          env_cmd+=("$pass_name=''${!pass_name}")
        fi
      done <<< "$pass_through_env_tsv"

      if [ -n "$host_developer_dir" ] && [ -z "''${DEVELOPER_DIR+x}" ]; then
        env_cmd+=("DEVELOPER_DIR=$host_developer_dir")
      fi
      if [ -n "$host_sdkroot" ] && [ -z "''${SDKROOT+x}" ]; then
        env_cmd+=("SDKROOT=$host_sdkroot")
      fi

      while IFS=$'\t' read -r env_name env_value || [ -n "$env_name" ]; do
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
          if is_reserved_runtime_env_name "$env_name"; then
            echo "ERROR: runtime-owned env override blocked name=$env_name"
            return 3
          fi
          env_cmd+=("$env_name=$env_value")
        fi
      done <<< "$runtime_env_tsv"

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
