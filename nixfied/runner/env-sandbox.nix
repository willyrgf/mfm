{ pkgs, projectRoot }:
''
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

    local home_value
    home_value="''${HOME:-$workdir}"

    local -a env_cmd
    env_cmd=(env -i "PATH=$final_path" "LANG=$locale" "LC_ALL=$locale" "TZ=$timezone" "HOME=$home_value")

    while IFS= read -r pass_name; do
      if [ -n "$pass_name" ] && [ -n "''${!pass_name+x}" ]; then
        env_cmd+=("$pass_name=''${!pass_name}")
      fi
    done < <(printf '%s' "$runtime_json" | ${pkgs.jq}/bin/jq -r '.passThroughEnv[]?')

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
