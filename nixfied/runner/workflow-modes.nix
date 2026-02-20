{ pkgs }:
''
  workflow_family_from_id() {
    local workflow_id="$1"
    if [[ "$workflow_id" =~ ^workflow\.([^.]+)\..+$ ]]; then
      printf '%s' "''${BASH_REMATCH[1]}"
      return 0
    fi
    return 1
  }

  workflow_modes_for_family() {
    local workflow_id="$1"
    local family
    local prefix

    family="$(workflow_family_from_id "$workflow_id" || true)"
    if [ -z "$family" ]; then
      return 0
    fi

    prefix="workflow.$family."
    ${pkgs.jq}/bin/jq -r \
      --arg prefix "$prefix" \
      '.workflows | keys[] | select(startswith($prefix)) | .[($prefix | length):]' \
      "$MODEL_FILE" \
      | ${pkgs.coreutils}/bin/sort -u
  }

  workflow_mode_is_simple_shorthand() {
    local mode="$1"
    [[ "$mode" =~ ^[a-z0-9-]+$ ]]
  }

  workflow_simple_shorthand_exists_for_family() {
    local workflow_id="$1"
    local candidate="$2"
    local mode

    while IFS= read -r mode; do
      if [ "$mode" = "$candidate" ] && workflow_mode_is_simple_shorthand "$mode"; then
        return 0
      fi
    done < <(workflow_modes_for_family "$workflow_id")

    return 1
  }

  workflow_expected_modes_for_family() {
    local workflow_id="$1"
    local mode
    local expected=""

    while IFS= read -r mode; do
      if [ -z "$expected" ]; then
        expected="$mode"
      else
        expected="$expected|$mode"
      fi
    done < <(workflow_modes_for_family "$workflow_id")

    printf '%s' "$expected"
  }

  workflow_resolve_mode_id() {
    local workflow_id="$1"
    local mode_override="$2"
    local family
    local candidate
    local expected

    if [ -z "$mode_override" ]; then
      printf '%s' "$workflow_id"
      return 0
    fi

    family="$(workflow_family_from_id "$workflow_id" || true)"
    if [ -z "$family" ]; then
      echo "ERROR: workflow '$workflow_id' does not support mode overrides" >&2
      return 2
    fi

    candidate="workflow.$family.$mode_override"
    if ${pkgs.jq}/bin/jq -e --arg workflowId "$candidate" '.workflows[$workflowId] != null' "$MODEL_FILE" >/dev/null; then
      printf '%s' "$candidate"
      return 0
    fi

    expected="$(workflow_expected_modes_for_family "$workflow_id")"
    if [ -n "$expected" ]; then
      echo "ERROR: unknown mode '$mode_override' (expected: $expected)" >&2
    else
      echo "ERROR: unknown mode '$mode_override'" >&2
    fi
    return 2
  }
''
