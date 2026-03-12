{ pkgs }:
''
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

  write_text_file_atomic() {
    local target="$1"
    local value="$2"
    local parent_dir
    local tmp

    if [ -z "$target" ]; then
      return 0
    fi

    parent_dir="$(dirname "$target")"
    mkdir -p "$parent_dir"
    tmp="$(mktemp "$target.tmp.XXXXXX")"
    printf '%s\n' "$value" > "$tmp"
    mv "$tmp" "$target"
  }
''
