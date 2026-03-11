# Shared shell policy helpers for service reuse/ownership/discovery behavior.
{ pkgs }:

{
  policyRuntimeFunctions = ''
    nixfied_policy_log_error() {
      local message="$1"
      if command -v log_error >/dev/null 2>&1; then
        log_error "$message"
      else
        echo "ERROR: $message" >&2
      fi
    }

    nixfied_policy_owner_scope_from_reuse() {
      case "''${1:-}" in
        same-slot|cross-run)
          echo "persistent"
          ;;
        same-root)
          echo "ephemeral"
          ;;
        *)
          echo ""
          ;;
      esac
    }

    nixfied_policy_discovery_scope_from_reuse() {
      case "''${1:-}" in
        same-slot|cross-run)
          echo "global"
          ;;
        same-root)
          echo "local"
          ;;
        *)
          echo ""
          ;;
      esac
    }

    nixfied_policy_infer_reuse_policy() {
      local explicit_reuse="''${1:-}"
      local owner_scope="''${2:-}"
      local discovery_scope="''${3:-}"
      local fallback="''${4:-}"

      if [ -n "$explicit_reuse" ]; then
        echo "$explicit_reuse"
        return 0
      fi

      if [ "$owner_scope" = "persistent" ] || [ "$discovery_scope" = "global" ]; then
        echo "same-slot"
        return 0
      fi

      if [ "$owner_scope" = "ephemeral" ] || [ "$discovery_scope" = "local" ]; then
        echo "same-root"
        return 0
      fi

      echo "$fallback"
      return 0
    }

    nixfied_policy_infer_owner_scope() {
      local explicit_owner="''${1:-}"
      local explicit_reuse="''${2:-}"
      local explicit_discovery="''${3:-}"
      local from_reuse=""

      if [ -n "$explicit_owner" ]; then
        echo "$explicit_owner"
        return 0
      fi

      from_reuse="$(nixfied_policy_owner_scope_from_reuse "$explicit_reuse")"
      if [ -n "$from_reuse" ]; then
        echo "$from_reuse"
        return 0
      fi

      case "$explicit_discovery" in
        global)
          echo "persistent"
          ;;
        local)
          echo "ephemeral"
          ;;
        *)
          echo ""
          ;;
      esac
    }

    nixfied_policy_infer_discovery_scope() {
      local explicit_discovery="''${1:-}"
      local explicit_reuse="''${2:-}"
      local owner_scope="''${3:-}"
      local from_reuse=""

      if [ -n "$explicit_discovery" ]; then
        echo "$explicit_discovery"
        return 0
      fi

      from_reuse="$(nixfied_policy_discovery_scope_from_reuse "$explicit_reuse")"
      if [ -n "$from_reuse" ]; then
        echo "$from_reuse"
        return 0
      fi

      case "$owner_scope" in
        persistent)
          echo "global"
          ;;
        ephemeral)
          echo "local"
          ;;
        *)
          echo ""
          ;;
      esac
    }

    nixfied_policy_resolve_service_root() {
      local runtime_service_root="''${1:-}"
      local persistent_root_base="''${2:-}"
      local slot_value="''${3:-}"
      local env_value="''${4:-}"
      local explicit_reuse="''${5:-''${SERVICE_REUSE_POLICY:-}}"
      local explicit_owner="''${6:-''${SERVICE_OWNER_SCOPE:-}}"
      local explicit_discovery="''${7:-''${SERVICE_DISCOVERY_SCOPE:-}}"
      local owner_scope=""
      local discovery_scope=""
      local reuse_policy=""

      owner_scope="$(nixfied_policy_infer_owner_scope "$explicit_owner" "$explicit_reuse" "$explicit_discovery")"
      discovery_scope="$(nixfied_policy_infer_discovery_scope "$explicit_discovery" "$explicit_reuse" "$owner_scope")"
      reuse_policy="$(nixfied_policy_infer_reuse_policy "$explicit_reuse" "$owner_scope" "$discovery_scope" "")"

      nixfied_policy_validate_matrix "$reuse_policy" "$owner_scope" "$discovery_scope" 1 1 || return 1

      case "$reuse_policy" in
        same-slot)
          if [ -z "$persistent_root_base" ]; then
            nixfied_policy_log_error "persistent service root base is required for same-slot reuse"
            return 1
          fi
          if [ -z "$env_value" ] || [ -z "$slot_value" ]; then
            nixfied_policy_log_error "slot/env values are required for same-slot service root resolution"
            return 1
          fi
          printf '%s' "$persistent_root_base/$env_value/slot-$slot_value"
          ;;
        cross-run)
          if [ -z "$persistent_root_base" ]; then
            nixfied_policy_log_error "persistent service root base is required for cross-run reuse"
            return 1
          fi
          printf '%s' "$persistent_root_base/shared"
          ;;
        never|same-root|"")
          printf '%s' "$runtime_service_root"
          ;;
        *)
          nixfied_policy_log_error "unresolved service root reuse policy '$reuse_policy'"
          return 1
          ;;
      esac
    }

    nixfied_policy_validate_reuse_policy() {
      local reuse="''${1:-}"
      local allow_empty="''${2:-0}"

      case "$reuse" in
        never|same-root|same-slot|cross-run)
          return 0
          ;;
        "")
          if [ "$allow_empty" = "1" ]; then
            return 0
          fi
          ;;
      esac

      nixfied_policy_log_error "SERVICE_REUSE_POLICY must be one of never|same-root|same-slot|cross-run (got '$reuse')"
      return 1
    }

    nixfied_policy_validate_owner_scope() {
      local owner="''${1:-}"
      local allow_empty="''${2:-0}"

      case "$owner" in
        ephemeral|persistent)
          return 0
          ;;
        "")
          if [ "$allow_empty" = "1" ]; then
            return 0
          fi
          ;;
      esac

      nixfied_policy_log_error "SERVICE_OWNER_SCOPE must be ephemeral|persistent (got '$owner')"
      return 1
    }

    nixfied_policy_validate_discovery_scope() {
      local discovery="''${1:-}"
      local allow_empty="''${2:-0}"

      case "$discovery" in
        local|global)
          return 0
          ;;
        "")
          if [ "$allow_empty" = "1" ]; then
            return 0
          fi
          ;;
      esac

      nixfied_policy_log_error "SERVICE_DISCOVERY_SCOPE must be local|global (got '$discovery')"
      return 1
    }

    nixfied_policy_validate_matrix() {
      local reuse="''${1:-}"
      local owner="''${2:-}"
      local discovery="''${3:-}"
      local allow_empty="''${4:-0}"
      local enforce_owner_discovery_alignment="''${5:-0}"

      nixfied_policy_validate_reuse_policy "$reuse" "$allow_empty" || return 1
      nixfied_policy_validate_owner_scope "$owner" "$allow_empty" || return 1
      nixfied_policy_validate_discovery_scope "$discovery" "$allow_empty" || return 1

      if [ "$reuse" = "cross-run" ] && { [ "$owner" != "persistent" ] || [ "$discovery" != "global" ]; }; then
        nixfied_policy_log_error "cross-run reuse requires SERVICE_OWNER_SCOPE=persistent and SERVICE_DISCOVERY_SCOPE=global"
        return 1
      fi

      if [ "$reuse" = "same-root" ] && { [ "$owner" != "ephemeral" ] || [ "$discovery" != "local" ]; }; then
        nixfied_policy_log_error "same-root reuse requires SERVICE_OWNER_SCOPE=ephemeral and SERVICE_DISCOVERY_SCOPE=local"
        return 1
      fi

      if [ "$enforce_owner_discovery_alignment" = "1" ] && [ -n "$owner" ] && [ -n "$discovery" ]; then
        if [ "$owner" = "persistent" ] && [ "$discovery" != "global" ]; then
          nixfied_policy_log_error "persistent owner scope requires SERVICE_DISCOVERY_SCOPE=global"
          return 1
        fi
        if [ "$owner" = "ephemeral" ] && [ "$discovery" != "local" ]; then
          nixfied_policy_log_error "ephemeral owner scope requires SERVICE_DISCOVERY_SCOPE=local"
          return 1
        fi
      fi

      return 0
    }
  '';
}
