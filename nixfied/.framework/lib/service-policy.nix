# Shared shell policy helpers for service reuse/ownership/discovery behavior.
{ pkgs }:

{
  policyRuntimeFunctions = ''
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

      echo "ERROR: SERVICE_REUSE_POLICY must be one of never|same-root|same-slot|cross-run (got '$reuse')" >&2
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

      echo "ERROR: SERVICE_OWNER_SCOPE must be ephemeral|persistent (got '$owner')" >&2
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

      echo "ERROR: SERVICE_DISCOVERY_SCOPE must be local|global (got '$discovery')" >&2
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
        echo "ERROR: cross-run reuse requires SERVICE_OWNER_SCOPE=persistent and SERVICE_DISCOVERY_SCOPE=global" >&2
        return 1
      fi

      if [ "$reuse" = "same-root" ] && { [ "$owner" != "ephemeral" ] || [ "$discovery" != "local" ]; }; then
        echo "ERROR: same-root reuse requires SERVICE_OWNER_SCOPE=ephemeral and SERVICE_DISCOVERY_SCOPE=local" >&2
        return 1
      fi

      if [ "$enforce_owner_discovery_alignment" = "1" ] && [ -n "$owner" ] && [ -n "$discovery" ]; then
        if [ "$owner" = "persistent" ] && [ "$discovery" != "global" ]; then
          echo "ERROR: persistent owner scope requires SERVICE_DISCOVERY_SCOPE=global" >&2
          return 1
        fi
        if [ "$owner" = "ephemeral" ] && [ "$discovery" != "local" ]; then
          echo "ERROR: ephemeral owner scope requires SERVICE_DISCOVERY_SCOPE=local" >&2
          return 1
        fi
      fi

      return 0
    }
  '';
}
