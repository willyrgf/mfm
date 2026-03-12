{
  pkgs,
  compareRepo ? "https://github.com/willyrgf/nixfied",
}:

pkgs.writeText "nixfied-vendored-metadata.sh" ''
  read_vendored_revision() {
    local metadata_file="$1"
    local revision=""

    if [ ! -f "$metadata_file" ]; then
      echo "unknown"
      return 0
    fi

    revision=$(${pkgs.gawk}/bin/awk '
      $0 ~ /^Framework source revision \(install\/upgrade\):$/ { in_section=1; next }
      in_section && $0 ~ /^[[:space:]]*-[[:space:]]+/ {
        line=$0
        sub(/^[[:space:]]*-[[:space:]]+/, "", line)
        print line
        exit
      }
    ' "$metadata_file" 2>/dev/null || true)

    if [ -z "$revision" ]; then
      revision="unknown"
    fi

    printf '%s\n' "$revision"
  }

  normalize_vendored_revision() {
    local revision="$1"
    revision="''${revision%-dirty}"
    printf '%s\n' "$revision"
  }

  is_hex_revision() {
    local revision=""
    revision="$(normalize_vendored_revision "$1")"
    case "$revision" in
      "" | *[!0-9a-f]*)
        return 1
        ;;
    esac
    [ "''${#revision}" -ge 7 ]
  }

  vendored_git_history_available() {
    [ -n "''${GIT:-}" ] \
      && [ -n "''${SOURCE_GIT_ROOT:-}" ] \
      && "$GIT" -C "$SOURCE_GIT_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1
  }

  git_revision_exists() {
    local revision=""
    revision="$(normalize_vendored_revision "$1")"
    vendored_git_history_available \
      && [ -n "$revision" ] \
      && [ "$revision" != "unknown" ] \
      && "$GIT" -C "$SOURCE_GIT_ROOT" rev-parse --verify --quiet "''${revision}^{commit}" >/dev/null 2>&1
  }

  limit_commit_lines() {
    local max_lines="$1"
    local line=""
    local count=0

    while IFS= read -r line; do
      [ -n "$line" ] || continue
      count=$((count + 1))
      if [ "$count" -le "$max_lines" ]; then
        printf -- '- %s\n' "$line"
      fi
    done

    if [ "$count" -gt "$max_lines" ]; then
      printf -- '- ... %s additional commits not shown\n' "$((count - max_lines))"
    fi
  }

  render_recent_framework_changes() {
    local current_revision=""
    current_revision="''${FRAMEWORK_REVISION:-unknown}"

    if git_revision_exists "$current_revision"; then
      "$GIT" -C "$SOURCE_GIT_ROOT" log --no-decorate --oneline -10 "$(normalize_vendored_revision "$current_revision")" 2>/dev/null \
        | limit_commit_lines 10
      return 0
    fi

    echo "- exact git history unavailable from packaged source"
  }

  render_upgrade_framework_changes() {
    local previous_revision=""
    local current_revision=""
    local raw_log=""

    previous_revision="''${PREV_FRAMEWORK_REVISION:-unknown}"
    current_revision="''${FRAMEWORK_REVISION:-unknown}"

    if [ "$previous_revision" = "$current_revision" ]; then
      echo "- none (previous vendored revision already matches current framework revision)"
      return 0
    fi

    if git_revision_exists "$previous_revision" && git_revision_exists "$current_revision"; then
      raw_log=$(
        "$GIT" \
          -C "$SOURCE_GIT_ROOT" \
          log \
          --no-decorate \
          --oneline \
          "$(normalize_vendored_revision "$previous_revision")..$(normalize_vendored_revision "$current_revision")" \
          2>/dev/null \
          || true
      )

      if [ -n "$raw_log" ]; then
        printf '%s\n' "$raw_log" | limit_commit_lines 30
        return 0
      fi
    fi

    echo "- exact git history unavailable from packaged source"
    if is_hex_revision "$previous_revision" && is_hex_revision "$current_revision"; then
      echo "- compare: ${compareRepo}/compare/$(normalize_vendored_revision "$previous_revision")...$(normalize_vendored_revision "$current_revision")"
    fi
  }

  write_vendored_metadata() {
    local target_file="$1"
    local previous_revision="''${PREV_FRAMEWORK_REVISION:-unknown}"
    local current_revision="''${FRAMEWORK_REVISION:-unknown}"
    local change_header="Recent framework changes:"
    local change_body=""
    local normalized_previous=""
    local normalized_current=""

    normalized_previous="$(normalize_vendored_revision "$previous_revision")"
    normalized_current="$(normalize_vendored_revision "$current_revision")"

    if [ -n "$previous_revision" ] && [ "$previous_revision" != "unknown" ]; then
      change_header="Changes since previous vendored revision:"
      if is_hex_revision "$previous_revision" && is_hex_revision "$current_revision"; then
        change_header="Changes since previous vendored revision ($normalized_previous..$normalized_current):"
      fi
      change_body="$(render_upgrade_framework_changes)"
    else
      change_body="$(render_recent_framework_changes)"
    fi

    {
      echo "Vendored Framework"
      echo "=================="
      echo ""
      echo 'This repository vendors the Nixfied framework under `nixfied/`.'
      echo ""
      echo "Framework source revision (install/upgrade):"
      echo "- $FRAMEWORK_REVISION"
      echo ""
      echo "$change_header"
      printf '%s\n' "$change_body"
      echo ""
      echo "Framework source revision workflow:"
      echo "- initialized via \`framework::install\`"
      echo "- upgraded via \`framework::upgrade\` (preserves \`nixfied/project/\` and \`nixfied/local/\` by default)"
      echo ""
      echo "Framework-owned paths:"
      echo "- \`flake.nix\`, \`flake.lock\`"
      echo "- framework-owned files under \`nixfied/\` except \`nixfied/project/\` and \`nixfied/local/\`"
      echo ""
      echo "User-owned customization paths:"
      echo "- \`nixfied/project/\` (primary command/task/workflow customization surface)"
      echo "- \`nixfied/local/\` (optional extensions)"
      echo ""
      echo "Prefer editing \`nixfied/project/\` and \`nixfied/local/\` over direct framework internals."
    } > "$target_file"
  }
''
