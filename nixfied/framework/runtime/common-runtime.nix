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

    parent_dir="$(dirname "$target")" || {
      echo "ERROR: unable to determine parent directory for '$target'" >&2
      return 1
    }
    mkdir -p "$parent_dir" || {
      echo "ERROR: unable to create directory '$parent_dir'" >&2
      return 1
    }
    tmp="$(mktemp "$target.tmp.XXXXXX")" || {
      echo "ERROR: unable to create temp file for '$target'" >&2
      return 1
    }
    if ! printf '%s\n' "$value" > "$tmp"; then
      rm -f "$tmp"
      echo "ERROR: failed to write temp file for '$target'" >&2
      return 1
    fi
    if ! mv "$tmp" "$target"; then
      rm -f "$tmp"
      echo "ERROR: failed to move temp file into '$target'" >&2
      return 1
    fi
  }
''
