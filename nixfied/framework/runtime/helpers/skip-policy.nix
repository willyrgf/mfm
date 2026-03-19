# Shared shell helpers for SKIP_<SERVICE> env var parsing.
{ pkgs }:

{
  skipPolicyFunctions = ''
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
  '';
}
